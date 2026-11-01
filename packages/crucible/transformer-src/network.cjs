const path = require("node:path");
const { packageOf } = require("./type-id.cjs");
const {
	limits,
	dataLimits,
	kinds,
	depthLimits,
	replica,
	serverOnlyPaths,
	viewNameProblems,
} = require("./declarations.cjs");

const declarationMacros = Object.keys(kinds);
const macros = [...declarationMacros, "receive"];

function createNetwork({ ts, program, checker, api, kindOf, instanceType, modifiers, isMacro }) {
	const unalias = (s) => s && (s.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(s) : s);
	const standard = (symbol, pkg) =>
		symbol?.declarations?.some((d) => d.getSourceFile().fileName.replaceAll("\\", "/").includes(`/@rbxts/${pkg}/`));
	const exempt = (file) =>
		program.isSourceFileDefaultLibrary(file) || /node_modules[\\/]@rbxts[\\/]/.test(file.fileName);
	const global = (decl) => {
		if (!ts.isExternalModule(decl.getSourceFile())) return true;
		for (let p = decl.parent; p; p = p.parent)
			if (ts.isModuleDeclaration(p) && p.flags & ts.NodeFlags.GlobalAugmentation) return true;
		return false;
	};
	const flags = ts.TypeFlags;
	const root = path.dirname(
		program.getCompilerOptions().configFilePath ?? path.join(program.getCurrentDirectory(), "tsconfig.json"),
	);
	const absent = (type) => !!(type.flags & flags.Undefined) || (type.isUnion() && type.types.some(absent));
	const literal = (type) => {
		if (type.isStringLiteral() || type.isNumberLiteral()) return type.value;
		if (type.flags & flags.BooleanLiteral) return type.intrinsicName === "true";
	};
	const compare = (a, b) => {
		const rank = { boolean: 0, number: 1, string: 2 };
		return (
			rank[typeof a] - rank[typeof b] ||
			(typeof a === "string" ? Buffer.compare(Buffer.from(a), Buffer.from(b)) : Number(a) - Number(b))
		);
	};
	const sorted = (names) => names.sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
	const widths = new Set();
	for (const sf of program.getSourceFiles()) {
		if (packageOf(sf.fileName)?.json.name !== "@trioforge/crucible") continue;
		const walk = (node) => {
			if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === "width")
				widths.add(checker.getSymbolAtLocation(node.name));
			ts.forEachChild(node, walk);
		};
		walk(sf);
	}
	const width = (type) => {
		if (!type.isIntersection()) return;
		if (!type.types.some((t) => t.flags & flags.Number)) return;
		const found = new Set();
		for (const part of type.types) {
			if (part.flags & flags.Number) continue;
			const props = checker.getPropertiesOfType(part);
			if (props.length !== 1) return;
			const prop = props[0];
			const name = prop.declarations?.[0]?.name;
			if (
				!name ||
				!ts.isComputedPropertyName(name) ||
				!widths.has(unalias(checker.getSymbolAtLocation(name.expression)))
			)
				return;
			const value = checker.getTypeOfSymbolAtLocation(prop, prop.valueDeclaration ?? prop.declarations[0]);
			for (const t of value.isUnion() ? value.types : [value]) if (t.isStringLiteral()) found.add(t.value);
		}
		return found.size === 1 ? [...found][0] : "conflicting";
	};
	const engine = (type) => {
		const symbol = type.getSymbol();
		if (!standard(symbol, "types")) return;
		const name = symbol.name;
		if (["Vector3", "Color3", "CFrame"].includes(name)) return { kind: name.toLowerCase(), runtime: name };
		const instances = checker.resolveName("Instances", symbol.declarations[0], ts.SymbolFlags.Type, false);
		if (instances && checker.getPropertyOfType(checker.getDeclaredTypeOfSymbol(instances), name))
			return { kind: "instance", classes: [name], runtime: "Instance" };
		return { kind: "unsupported" };
	};
	const collection = (type) => {
		if (checker.isTupleType(type)) return "tuple";
		if (checker.isArrayType(type)) return "array";
		const symbol = type.getSymbol();
		if (["Map", "ReadonlyMap"].includes(symbol?.name) && standard(symbol, "compiler-types")) return "map";
	};
	let declarations;
	const index = () => {
		if (declarations) return declarations;
		declarations = [];
		for (const sf of program.getSourceFiles()) {
			if (sf.isDeclarationFile) continue;
			const walk = (node) => {
				const kind = declarationMacros.find((name) => isMacro(node, name));
				if (kind)
					declarations.push({
						node,
						kind,
						name:
							node.arguments[0] && ts.isStringLiteral(node.arguments[0])
								? node.arguments[0].text
								: undefined,
					});
				ts.forEachChild(node, walk);
			};
			walk(sf);
		}
		return declarations;
	};
	const serverOnly = (type) => api(type.getSymbol()) === "ServerOnly";
	const plainIntersection = (type) =>
		type.isIntersection() &&
		type.types.every(
			(part) =>
				part.flags & flags.Object && !part.getCallSignatures().length && !part.getConstructSignatures().length,
		);
	const targetOf = (node) => {
		if (node.typeArguments?.length) return checker.getTypeFromTypeNode(node.typeArguments[0]);
		const signature = checker.getResolvedSignature(node);
		const returned = signature && checker.getReturnTypeOfSignature(signature);
		if (returned && returned.objectFlags & ts.ObjectFlags.Reference) return checker.getTypeArguments(returned)[0];
	};
	function lowerDeclaration(node, kind, report, reached = new Set()) {
		const spec = kinds[kind];
		const state = spec.position !== "payload";
		const persist = spec.position === "persist";
		const depthLimit = depthLimits[spec.position];
		const active = new Set();
		const fail = (at, location, reason, code = "CRU019") => {
			report(at, code, `${node.arguments[0]?.text ?? kind} ${location}: ${reason}`);
			return { kind: "nil" };
		};
		const inherited = new Set();
		const checkSymbols = (symbols, location) => {
			for (const symbol of symbols) {
				if (!symbol) continue;
				reached.add(symbol);
				const decls = (symbol.declarations ?? []).filter((d) => !exempt(d.getSourceFile()));
				if (decls.some(global) || new Set(decls.map((d) => d.getSourceFile().fileName)).size > 1)
					fail(
						node,
						location,
						`${symbol.name} must be declared in one imported module; incremental builds cannot see global declarations or merges.`,
						"CRU010",
					);
				if (!inherited.has(symbol)) {
					inherited.add(symbol);
					for (const decl of decls)
						if (ts.isInterfaceDeclaration(decl) || ts.isClassDeclaration(decl))
							for (const clause of decl.heritageClauses ?? [])
								if (clause.token === ts.SyntaxKind.ExtendsKeyword)
									for (const base of clause.types) {
										references(base, location);
										mention(checker.getTypeAtLocation(base), location);
									}
				}
			}
		};
		const mention = (type, location) => {
			checkSymbols([type.aliasSymbol, type.getSymbol()], location);
			if (type.isIntersection()) for (const part of type.types) mention(part, location);
		};
		const declared = (type, location) => checkSymbols([type.aliasSymbol, type.getSymbol()], location);
		const referenced = new Set();
		const references = (syntax, location) => {
			if (!syntax || referenced.has(syntax)) return;
			referenced.add(syntax);
			if (ts.isTypeReferenceNode(syntax) || ts.isExpressionWithTypeArguments(syntax)) {
				const symbol = unalias(
					checker.getSymbolAtLocation(ts.isTypeReferenceNode(syntax) ? syntax.typeName : syntax.expression),
				);
				checkSymbols([symbol], location);
				for (const decl of symbol?.declarations ?? [])
					if (ts.isTypeAliasDeclaration(decl) && !exempt(decl.getSourceFile()))
						references(decl.type, location);
			}
			if (ts.isTypeQueryNode(syntax)) {
				checkSymbols([unalias(checker.getSymbolAtLocation(syntax.exprName))], location);
				const queried = checker.getTypeAtLocation(syntax.exprName);
				for (const signature of [...queried.getCallSignatures(), ...queried.getConstructSignatures()])
					mention(checker.getReturnTypeOfSignature(signature), location);
			}
			ts.forEachChild(syntax, (child) => references(child, location));
		};
		for (const type of node.typeArguments ?? []) references(type, state ? "value" : "payload");
		const instance = (classes, location, position) => {
			if (state)
				return fail(
					node,
					location,
					persist
						? "Instances cannot be saved or replicated; store a stable id"
						: "replicated state cannot hold Instances; replicate a stable id",
				);
			if (position.toClient && !position.nilable && !(classes.length === 1 && classes[0] === "Player"))
				fail(
					node,
					location,
					"a client may not have streamed this Instance in; admit undefined or wrap collection elements in objects with an optional field.",
					"CRU020",
				);
			return { kind: "instance", classes: sorted(classes) };
		};
		const fields = (type, location, position, depth, omit) => {
			if (type.getCallSignatures().length || type.getConstructSignatures().length)
				return fail(node, location, "functions cannot cross the network");
			const props = checker
				.getPropertiesOfType(type)
				.filter((p) => p.name !== omit)
				.sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)));
			if (!props.length && omit === undefined)
				return fail(
					node,
					location,
					spec.handles
						? "an empty object type accepts anything; annotate an empty template table: {} as Record<string, number>"
						: "an empty object type accepts anything; use undefined",
				);
			return {
				kind: "object",
				fields: props.map((prop) => {
					const at = prop.valueDeclaration ?? prop.declarations?.[0] ?? node;
					for (const decl of prop.declarations ?? []) {
						const owner = ts.isParameter(decl) ? decl.parent.parent : decl.parent;
						if (owner && (ts.isClassLike(owner) || ts.isInterfaceDeclaration(owner)) && owner.name)
							reached.add(checker.getSymbolAtLocation(owner.name));
					}
					references(at.type, `${location}.${prop.name}`);
					if (prop.escapedName.toString().startsWith("__@"))
						fail(node, location, "symbol property names cannot cross the network");
					if (ts.isNumericLiteralName(prop.name))
						fail(node, `${location}.${prop.name}`, "numeric property names are not supported; use a Map");
					if (prop.declarations?.some((d) => ts.isMethodSignature(d) || ts.isMethodDeclaration(d)))
						fail(node, `${location}.${prop.name}`, "functions cannot cross the network");
					if (spec.handles && dataLimits.RESERVED_HANDLE_NAMES.includes(prop.name))
						fail(
							node,
							`${location}.${prop.name}`,
							`\`${prop.name}\` is a handle method name (set, update, insert, remove); rename the field`,
							"CRU026",
						);
					const value = checker.getTypeOfSymbolAtLocation(prop, at);
					if (value.flags & flags.Undefined)
						return [
							prop.name,
							fail(
								node,
								`${location}.${prop.name}`,
								spec.handles
									? "a position that is always undefined; annotate it: undefined as string | undefined"
									: "a field of type undefined carries no information; remove it",
							),
						];
					return [
						prop.name,
						lower(
							value,
							`${location}.${prop.name}`,
							{
								toClient: position.toClient,
								nilable: !!(prop.flags & ts.SymbolFlags.Optional) || absent(value),
								path: position.path,
								marked: position.marked,
							},
							depth + 1,
						),
					];
				}),
			};
		};
		const key = (type, location) => {
			declared(type, location);
			if (type.flags & flags.String) return { kind: "string" };
			if (state && !(type.isUnion() ? type.types : [type]).every((t) => t.isStringLiteral()))
				return fail(
					node,
					location,
					"state maps are keyed by string or a union of string literals; frames carry string-keyed records",
				);
			const w = width(type);
			if (["u8", "u16", "u32"].includes(w)) return { kind: w };
			const parts = type.isUnion() ? type.types : [type];
			if (parts.length > limits.MAX_UNION_BRANCHES) return fail(node, location, "too many union branches");
			if (
				parts.every(
					(t) =>
						["string", "number"].includes(typeof literal(t)) &&
						(typeof literal(t) !== "number" || Number.isFinite(literal(t))),
				)
			)
				return {
					kind: "union",
					branches: parts
						.map((t) => literal(t))
						.sort(compare)
						.map((value) => ({ kind: "literal", value })),
				};
			return fail(
				node,
				location,
				"map keys must be string, u8, u16, u32 or a literal union; float keys can round to a duplicate",
			);
		};
		const union = (type, location, position, depth) => {
			const parts = type.types;
			const missing = parts.some((p) => p.flags & flags.Undefined);
			position = { ...position, nilable: position.nilable || missing, path: false };
			const branches = missing ? [{ kind: "nil" }] : [];
			const literals = [],
				typed = [],
				objects = [],
				classes = [];
			const bool = parts.filter((p) => p.flags & flags.BooleanLiteral).length === 2;
			if (bool) typed.push({ runtime: "boolean", node: { kind: "bool" } });
			for (const part of parts) {
				declared(part, location);
				if (part.flags & flags.Undefined || (bool && part.flags & flags.BooleanLiteral)) continue;
				const value = literal(part);
				if (value !== undefined) {
					literals.push({ kind: "literal", value });
					continue;
				}
				const e = engine(part);
				if (e?.kind === "instance") {
					classes.push(...e.classes);
					continue;
				}
				if (
					(part.flags & flags.Object || (state && plainIntersection(part))) &&
					!e &&
					!collection(part) &&
					!checker.getIndexInfosOfType(part).length &&
					!part.getCallSignatures().length &&
					!part.getConstructSignatures().length
				)
					objects.push(part);
				else {
					const descriptor = lower(part, location, position, depth + 1);
					const runtime =
						e?.runtime ??
						{
							bool: "boolean",
							f64: "number",
							f32: "number",
							u8: "number",
							u16: "number",
							u32: "number",
							string: "string",
							array: "table",
							tuple: "table",
							map: "table",
							object: "table",
							dynamic: "dynamic",
						}[descriptor.kind];
					typed.push({ runtime, node: descriptor });
				}
			}
			if (classes.length)
				typed.push({ runtime: "Instance", node: instance([...new Set(classes)], location, position) });
			const variants = [];
			if (objects.length > 1) {
				const candidates = sorted(checker.getPropertiesOfType(objects[0]).map((p) => p.name));
				let discriminant, values;
				for (const name of candidates) {
					const seen = new Set();
					const groups = objects.map((object) => {
						const prop = checker.getPropertyOfType(object, name);
						if (!prop || prop.flags & ts.SymbolFlags.Optional) return;
						references((prop.valueDeclaration ?? prop.declarations?.[0])?.type, `${location}.${name}`);
						const t = checker.getTypeOfSymbolAtLocation(
							prop,
							prop.valueDeclaration ?? prop.declarations?.[0] ?? node,
						);
						const list = (t.isUnion() ? t.types : [t]).map(literal);
						if (list.some((v) => v === undefined || seen.has(`${typeof v}:${v}`))) return;
						for (const v of list) seen.add(`${typeof v}:${v}`);
						return list;
					});
					if (groups.every(Boolean)) {
						discriminant = name;
						values = groups;
						break;
					}
				}
				if (discriminant === undefined)
					return fail(
						node,
						location,
						`union members ${objects.map((o) => checker.typeToString(o)).join(", ")} have no discriminant; give each a distinct literal property`,
					);
				if (ts.isNumericLiteralName(discriminant) || discriminant.startsWith("__@"))
					return fail(node, location, "numeric or symbol property names are not supported; use a Map");
				for (let i = 0; i < objects.length; i++)
					for (const value of values[i])
						variants.push({
							kind: "variant",
							key: discriminant,
							value,
							body: fields(objects[i], location, position, depth + 2, discriminant),
						});
			} else if (objects.length)
				typed.push({ runtime: "table", node: lower(objects[0], location, position, depth + 1) });
			if (
				new Set(typed.map((t) => t.runtime)).size !== typed.length ||
				(variants.length && typed.some((t) => t.runtime === "table"))
			)
				return fail(node, location, "union has ambiguous runtime kinds");
			branches.push(
				...literals.sort((a, b) => compare(a.value, b.value)),
				...typed
					.sort((a, b) => Buffer.compare(Buffer.from(a.runtime ?? ""), Buffer.from(b.runtime ?? "")))
					.map((t) => t.node),
				...variants.sort((a, b) => compare(a.value, b.value)),
			);
			if (branches.length > limits.MAX_UNION_BRANCHES) return fail(node, location, "too many union branches");
			return { kind: "union", branches };
		};
		const lower = (type, location, position, depth = 1) => {
			declared(type, location);
			if (depth > depthLimit) return fail(node, location, `schema exceeds the depth limit of ${depthLimit}`);
			if (state && type.isUnion()) {
				const present = type.types.filter((t) => !(t.flags & flags.Undefined));
				if (present.length === 1 && present.length < type.types.length && serverOnly(present[0])) {
					const inner = lower(present[0], location, position, depth);
					return inner.serverOnly
						? {
								kind: "union",
								branches: [{ kind: "nil" }, { ...inner, serverOnly: false }],
								serverOnly: true,
							}
						: inner;
				}
			}
			if (state && serverOnly(type)) {
				if (!spec.handles)
					return fail(
						node,
						location,
						"serverOnly marks a profileStore template field; it means nothing here",
					);
				if (position.root)
					fail(
						node,
						location,
						"serverOnly marks a template field, never the store's whole value; wrap the value in an object and mark the field",
					);
				if (position.marked) return fail(node, location, "this field is already inside a serverOnly field");
				if (!position.path)
					return fail(
						node,
						location,
						"serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member",
					);
				const inner = lower(checker.getTypeArguments(type)[0], location, { ...position, marked: true }, depth);
				return { ...inner, serverOnly: true };
			}
			if (active.has(type)) return fail(node, location, "recursive payload type");
			active.add(type);
			try {
				if (type.flags & flags.Any) return fail(node, location, "any cannot be checked; use unknown");
				if (type.flags & flags.Unknown)
					return state
						? fail(
								node,
								location,
								"unknown cannot be stored or replicated state; give the value a declared type",
							)
						: { kind: "dynamic" };
				if (type.flags & flags.Undefined) return { kind: "nil" };
				if (type.flags & flags.Void) return fail(node, location, "use undefined for no payload");
				if (
					type.flags &
					(flags.Never | flags.ESSymbolLike | flags.BigIntLike | flags.TemplateLiteral | flags.TypeParameter)
				)
					return fail(node, location, `cannot encode ${checker.typeToString(type)}`);
				if (type.flags & flags.Boolean) return { kind: "bool" };
				const value = literal(type);
				if (value !== undefined)
					return typeof value === "number" && !Number.isFinite(value)
						? fail(node, location, "nonfinite literal")
						: { kind: "literal", value };
				if (type.flags & flags.String) return { kind: "string" };
				if (type.flags & flags.Number) return { kind: "f64" };
				const w = width(type);
				if (w)
					return ["f64", "f32", "u8", "u16", "u32"].includes(w)
						? { kind: w }
						: fail(node, location, "conflicting widths");
				if (type.isUnion()) return union(type, location, position, depth);
				const flat = state && plainIntersection(type);
				if (type.isIntersection() && !flat)
					return fail(
						node,
						location,
						type.types.some((t) => t.flags & flags.NumberLike)
							? "only a Crucible width brand may intersect number; this brand is foreign"
							: "only numeric width intersections are supported",
					);
				const e = engine(type);
				if (e?.kind === "instance") return instance(e.classes, location, position);
				if (e?.kind === "unsupported") return fail(node, location, "Roblox datatype not supported in v1");
				if (e && persist)
					return fail(
						node,
						location,
						`${e.runtime} cannot be saved: a save holds JSON; store its components`,
					);
				if (e) return { kind: e.kind };
				const c = collection(type);
				const args = type.objectFlags & ts.ObjectFlags.Reference ? checker.getTypeArguments(type) : [];
				const element = (t, location, value = false) => {
					if (state && value) t = checker.getNonNullableType(t);
					else if (absent(t))
						fail(node, location, "collection elements and map values cannot admit undefined");
					return lower(t, location, { toClient: position.toClient, nilable: false }, depth + 1);
				};
				if (c === "tuple") {
					if (
						type.target.elementFlags.some(
							(ef) => ef & (ts.ElementFlags.Optional | ts.ElementFlags.Rest | ts.ElementFlags.Variadic),
						)
					)
						return fail(node, location, "tuples must be fixed length with no optional or rest elements");
					return { kind: "tuple", elements: args.map((t, i) => element(t, `${location}[${i}]`)) };
				}
				if (c === "array" && state && args[0].flags & flags.Never)
					return fail(
						node,
						`${location}[]`,
						"an empty array has no element type; annotate it: [] as string[]",
					);
				if (c === "array") return { kind: "array", element: element(args[0], `${location}[]`) };
				if (c === "map")
					return {
						kind: "map",
						key: key(args[0], `${location}.key`),
						value: element(args[1], `${location}.value`, true),
					};
				if (type.flags & flags.Object || flat) {
					const indices = checker.getIndexInfosOfType(type);
					if (
						indices.length === 1 &&
						indices[0].keyType.flags & flags.String &&
						!checker.getPropertiesOfType(type).length
					)
						return {
							kind: "map",
							key: { kind: "string" },
							value: element(indices[0].type, `${location}.value`, true),
						};
					if (indices.length)
						return fail(node, location, "only a string index signature can encode a record");
					return fields(type, location, position, depth);
				}
				return fail(node, location, `cannot encode ${checker.typeToString(type)}`);
			} finally {
				active.delete(type);
			}
		};
		let result;
		if (!state)
			result = (node.typeArguments ?? []).map((type, i) =>
				lower(
					checker.getTypeFromTypeNode(type),
					i === 1 ? "response" : kind === "askServer" ? "request" : "payload",
					{ toClient: kind === "toClient" || i === 1, nilable: false },
				),
			);
		else {
			const target = targetOf(node);
			if (!target) return [];
			const value = lower(target, "value", { toClient: false, nilable: false, path: true, root: true });
			if (spec.store && value.kind !== "object" && value.kind !== "map")
				fail(node, "value", `a store's value is a record; wrap a ${checker.typeToString(target)} in an object`);
			else if (spec.handles && value.kind === "map")
				fail(
					node,
					"value",
					"a profile store's value is an object with declared fields; wrap the record in a field",
				);
			result = [value];
		}
		const cost = (n, depth = 1) => {
			if (depth > depthLimit) fail(node, "value", `schema exceeds the depth limit of ${depthLimit}`);
			return (
				1 +
				(n.fields?.reduce((v, [, child]) => v + cost(child, depth + 1), 0) ?? 0) +
				(n.branches ?? n.elements ?? []).reduce((v, child) => v + cost(child, depth + 1), 0) +
				[
					n.element,
					n.key && typeof n.key === "object" ? n.key : undefined,
					n.value && typeof n.value === "object" ? n.value : undefined,
					n.body,
				]
					.filter(Boolean)
					.reduce((v, child) => v + cost(child, depth + 1), 0)
			);
		};
		if (result.reduce((v, n) => v + cost(n), 0) > limits.MAX_SCHEMA_NODES)
			fail(node, state ? "value" : "payload", "schema exceeds MAX_SCHEMA_NODES");
		if (!spec.handles) return result;
		const view = replica(result[0]);
		for (const [location, reason] of viewNameProblems(node.arguments[0]?.text, view))
			fail(node, location, reason, "CRU018");
		return [result[0], view];
	}
	let reached;
	let entryFiles;
	const entryExports = new Map();
	const canonicalFiles = new Map();
	const canonical = (file) => {
		if (!canonicalFiles.has(file)) canonicalFiles.set(file, path.normalize(ts.sys.realpath?.(file) ?? file));
		return canonicalFiles.get(file);
	};
	return ({ source, report, runtime, side, visitInput }) => {
		const f = ts.factory;
		const str = (value) => f.createStringLiteral(value);
		const arr = (values) => f.createArrayLiteralExpression(values);
		const value = (v) =>
			typeof v === "string"
				? str(v)
				: typeof v === "boolean"
					? v
						? f.createTrue()
						: f.createFalse()
					: v < 0
						? f.createPrefixUnaryExpression(ts.SyntaxKind.MinusToken, f.createNumericLiteral(-v))
						: f.createNumericLiteral(v);
		const emit = (n) => {
			const target = f.createPropertyAccessExpression(runtime("n"), n.kind);
			const args = {
				literal: () => [value(n.value)],
				instance: () => [arr(n.classes.map(str))],
				object: () => [arr(n.fields.map(([name, child]) => arr([str(name), emit(child)])))],
				array: () => [emit(n.element)],
				tuple: () => [arr(n.elements.map(emit))],
				map: () => [emit(n.key), emit(n.value)],
				union: () => [arr(n.branches.map(emit))],
				variant: () => [str(n.key), value(n.value), emit(n.body)],
			}[n.kind];
			return args ? f.createCallExpression(target, undefined, args()) : target;
		};
		if (!entryFiles) {
			entryFiles = new Map();
			for (const s of ["server", "client"]) {
				const resolved = ts.resolveModuleName(
					`@trioforge/crucible/${s}`,
					source.fileName,
					program.getCompilerOptions(),
					ts.sys,
				).resolvedModule;
				if (resolved) {
					entryFiles.set(canonical(resolved.resolvedFileName), s);
					const module = program.getSourceFile(resolved.resolvedFileName);
					const symbol = module && checker.getSymbolAtLocation(module);
					for (const exported of symbol ? checker.getExportsOfModule(symbol) : [])
						entryExports.set(unalias(exported), s);
				}
			}
		}
		const exportSide = (node, symbol) => {
			if (!symbol) return;
			if (entryExports.has(symbol)) return entryExports.get(symbol);
			if (!(symbol.flags & (ts.SymbolFlags.Variable | ts.SymbolFlags.Property))) return;
			const type = checker.getTypeAtLocation(node);
			return entryExports.get(unalias(type.aliasSymbol)) ?? entryExports.get(type.getSymbol());
		};
		const wrongSide = (node, file) => {
			const s = entryFiles.get(canonical(file));
			if (s && s !== side(source))
				report(
					node,
					"CRU024",
					`This file cannot reference @trioforge/crucible/${s}; move it under a ${s} folder.`,
				);
		};
		const callback = (expr) => {
			if (!expr) return;
			if (ts.isFunctionExpression(expr) || ts.isArrowFunction(expr)) return expr;
			if (!ts.isIdentifier(expr) && !ts.isPropertyAccessExpression(expr)) return;
			const symbol = unalias(checker.getSymbolAtLocation(ts.isPropertyAccessExpression(expr) ? expr.name : expr));
			if (symbol?.declarations?.length !== 1) return;
			const decl = symbol.declarations[0];
			if (ts.isFunctionDeclaration(decl) || ts.isMethodDeclaration(decl)) return decl;
			if (
				ts.isVariableDeclaration(decl) &&
				ts.getCombinedNodeFlags(decl) & ts.NodeFlags.Const &&
				decl.initializer &&
				(ts.isArrowFunction(decl.initializer) || ts.isFunctionExpression(decl.initializer))
			)
				return decl.initializer;
		};
		const receive = (node) => {
			const field = node.parent;
			if (
				!ts.isPropertyDeclaration(field) ||
				field.initializer !== node ||
				!modifiers(field, ts.ModifierFlags.Readonly) ||
				modifiers(field, ts.ModifierFlags.Static) ||
				kindOf(instanceType(field.parent)) !== "subsystem"
			) {
				report(node, "CRU021", "receive() must initialize a readonly instance field of a Subsystem.");
				return;
			}
			const name = checker.getSymbolAtLocation(field.name)?.name;
			const seen = new Set();
			const shadow = (type) => {
				if (seen.has(type)) return false;
				seen.add(type);
				const prop = checker.getPropertyOfType(type, name);
				return (
					prop?.declarations?.some(
						(d) => ts.isPropertyDeclaration(d) && d.initializer && isMacro(d.initializer, "receive"),
					) || (type.getBaseTypes?.() ?? []).some(shadow)
				);
			};
			if ((instanceType(field.parent).getBaseTypes?.() ?? []).some(shadow))
				report(node, "CRU021", "A receiver field cannot shadow an inherited receiver.");
			const target = node.arguments[0] && checker.getTypeAtLocation(node.arguments[0]);
			const kind = target && !target.isUnionOrIntersection() && api(target.getSymbol());
			if (!["ToServer", "ToClient", "AskServer", "ClientReady"].includes(kind))
				report(node, "CRU021", "receive target must be exactly one message token or ClientReady.");
			else {
				const required = kind === "ToClient" ? "client" : "server";
				if (side(field) !== required)
					report(node, "CRU022", `${kind} must be received by a ${required} Subsystem.`);
				const fn = callback(node.arguments[1]);
				const signature = fn && checker.getSignatureFromDeclaration(fn);
				if (signature) {
					const ret = checker.getReturnTypeOfSignature(signature);
					const parts = ret.isUnion() ? ret.types : [ret];
					if (
						modifiers(fn, ts.ModifierFlags.Async) ||
						parts.some(
							(t) => t.getSymbol()?.name === "Promise" && standard(t.getSymbol(), "compiler-types"),
						)
					)
						report(
							node,
							"CRU012",
							"Network callbacks cannot be async or return a Promise; spawn a thread inside the callback.",
						);
					else if (
						kind !== "AskServer" &&
						parts.some(
							(t) =>
								!(t.flags & (flags.Void | flags.Undefined | flags.Never)) &&
								!(kind === "ClientReady" && t.getCallSignatures().length),
						)
					)
						report(
							node,
							"CRU012",
							`${kind} callbacks return nothing${kind === "ClientReady" ? " or a cleanup function" : "; use a block body"}; this one can return ${checker.typeToString(ret)}.`,
						);
				}
			}
			return name;
		};
		const isCallee = (expression) => {
			while (ts.isParenthesizedExpression(expression.parent)) expression = expression.parent;
			return ts.isCallExpression(expression.parent) && expression.parent.expression === expression;
		};
		const inspect = (node) => {
			if (ts.isElementAccessExpression(node)) {
				const name = api(checker.getTypeAtLocation(node).getSymbol());
				if (macros.includes(name) && !isCallee(node))
					report(node, "CRU023", `${name} is a declaration macro and cannot be used as a value.`);
			}
			if (
				ts.isModuleDeclaration(node) &&
				(node.flags & ts.NodeFlags.GlobalAugmentation || ts.isStringLiteral(node.name))
			) {
				if (!reached) {
					reached = new Set();
					for (const declaration of index())
						lowerDeclaration(declaration.node, declaration.kind, () => {}, reached);
				}
				const added = (body) =>
					(!body ? [] : ts.isModuleBlock(body) ? body.statements : [body]).flatMap((statement) => [
						...(ts.isVariableStatement(statement) ? statement.declarationList.declarations : [statement]),
						...(ts.isModuleDeclaration(statement) ? added(statement.body) : []),
					]);
				if (added(node.body).some((d) => reached.has(checker.getSymbolAtLocation(d.name))))
					report(
						node,
						"CRU010",
						"This augmentation changes a type a Crucible declaration lowers; incremental builds cannot see the merge. Declare the type in one module.",
					);
			}
			if (
				ts.isStringLiteral(node) &&
				(ts.isImportDeclaration(node.parent) ||
					ts.isExportDeclaration(node.parent) ||
					(ts.isLiteralTypeNode(node.parent) && ts.isImportTypeNode(node.parent.parent)) ||
					(ts.isCallExpression(node.parent) && node.parent.expression.kind === ts.SyntaxKind.ImportKeyword))
			) {
				const resolved = ts.resolveModuleName(
					node.text,
					source.fileName,
					program.getCompilerOptions(),
					ts.sys,
				).resolvedModule;
				if (resolved) wrongSide(node, resolved.resolvedFileName);
			}
			if (ts.isIdentifier(node)) {
				const symbol = unalias(checker.getSymbolAtLocation(node));
				for (const decl of symbol?.declarations ?? []) wrongSide(node, decl.getSourceFile().fileName);
				const owner = exportSide(node, symbol);
				if (
					owner &&
					owner !== side(source) &&
					!symbol.declarations?.some((d) => entryFiles.has(canonical(d.getSourceFile().fileName)))
				)
					report(
						node,
						"CRU024",
						`This file cannot reference @trioforge/crucible/${owner} through ${node.text}; move it under a ${owner} folder.`,
					);
				const name = api(symbol);
				if (macros.includes(name)) {
					const p = node.parent;
					const expression = ts.isPropertyAccessExpression(p) && p.name === node ? p : node;
					if (!isCallee(expression) && !ts.isImportSpecifier(p) && !ts.isExportSpecifier(p))
						report(node, "CRU023", `${name} is a declaration macro and cannot be used as a value.`);
				}
			}
		};
		const visit = (node) => {
			if (isMacro(node, "receive")) {
				const name = receive(node);
				return (
					name &&
					f.createCallExpression(runtime("receive"), node.typeArguments, [
						f.createThis(),
						str(name),
						visitInput(node.arguments[0]),
						visitInput(node.arguments[1]),
					])
				);
			}
			const kind = declarationMacros.find((name) => isMacro(node, name));
			if (!kind) return;
			const spec = kinds[kind];
			const decl = node.parent,
				list = decl.parent,
				statement = list?.parent;
			if (
				!ts.isVariableDeclaration(decl) ||
				decl.initializer !== node ||
				!ts.isIdentifier(decl.name) ||
				!ts.isVariableDeclarationList(list) ||
				!(list.flags & ts.NodeFlags.Const) ||
				!ts.isVariableStatement(statement) ||
				statement.parent !== source ||
				!modifiers(statement, ts.ModifierFlags.Export) ||
				!ts.isExternalModule(source) ||
				side(node) !== spec.side
			)
				report(
					node,
					"CRU017",
					`Declare ${spec.label} as an exported top-level const in a ${spec.side} module.`,
				);
			const typeArguments = node.typeArguments?.length ?? 0;
			if (spec.typeArguments === "optional" ? typeArguments > 1 : typeArguments !== spec.typeArguments)
				report(
					node,
					"CRU017",
					spec.position === "payload"
						? `${kind} needs explicit payload types; use undefined for no payload.`
						: `${kind} needs ${spec.typeArguments === "optional" ? "at most one" : "an explicit"} type argument.`,
				);
			const name = node.arguments[0];
			if (
				!name ||
				!ts.isStringLiteral(name) ||
				Buffer.byteLength(name.text) > spec.maxBytes ||
				!/^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)*$/.test(name.text) ||
				name.text.split(".")[0] === "Crucible"
			)
				report(
					node,
					"CRU018",
					`${spec.names} names are 1 to ${spec.maxBytes} bytes of dot-separated segments [A-Za-z][A-Za-z0-9_]*; Crucible is reserved.`,
				);
			else {
				const other = index().find(
					(d) => d.node !== node && d.name === name.text && kinds[d.kind].names === spec.names,
				);
				if (other) {
					const sf = other.node.getSourceFile();
					report(
						node,
						"CRU018",
						`${name.text} is already declared at ${path.relative(root, sf.fileName).replaceAll("\\", "/")}:${sf.getLineAndCharacterOfPosition(other.node.getStart()).line + 1}.`,
					);
				}
			}
			let timeout = f.createIdentifier("undefined");
			const options = node.arguments[1];
			if (kind === "askServer" && options) {
				const prop =
					ts.isObjectLiteralExpression(options) && options.properties.length === 1 && options.properties[0];
				if (
					!prop ||
					!ts.isPropertyAssignment(prop) ||
					prop.name.getText().replaceAll('"', "").replaceAll("'", "") !== "timeout" ||
					!ts.isNumericLiteral(prop.initializer) ||
					!(
						Number(prop.initializer.text) > 0 &&
						Number(prop.initializer.text) <= limits.MAX_ASK_TIMEOUT_SECONDS
					)
				)
					report(node, "CRU017", "askServer options must be { timeout: <number literal in (0, 120]> }.");
				else timeout = prop.initializer;
			}
			if (node.arguments.length > spec.arguments) report(node, "CRU017", "Unexpected declaration arguments.");
			const descriptors = lowerDeclaration(node, kind, report);
			if (spec.position !== "payload" && !descriptors.length)
				report(node, "CRU017", `Cannot read ${kind}'s value type; give it an explicit type argument.`);
			const tail =
				kind === "askServer"
					? [timeout]
					: spec.handles && descriptors.length
						? [arr(serverOnlyPaths(descriptors[0]).map((segments) => arr(segments.map(str))))]
						: [];
			return f.createCallExpression(runtime(kind), node.typeArguments, [
				name ?? str("invalid"),
				...spec.passed.map((i) => node.arguments[i] && visitInput(node.arguments[i])).filter(Boolean),
				...descriptors.map(emit),
				...tail,
			]);
		};
		return { inspect, visit };
	};
}

module.exports = { createNetwork, limits };
