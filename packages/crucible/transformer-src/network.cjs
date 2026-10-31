const path = require("node:path");
const { packageOf } = require("./type-id.cjs");

const limits = Object.freeze({
	MESSAGE_NAME_MAX_BYTES: 64,
	MAX_ASK_TIMEOUT_SECONDS: 120,
	MAX_UNION_BRANCHES: 256,
	MAX_SCHEMA_NODES: 4096,
	MAX_DEPTH: 32,
});
const macros = ["toServer", "toClient", "askServer", "receive"];

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
				const kind = macros.slice(0, 3).find((name) => isMacro(node, name));
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
	function lowerMessage(node, kind, report, reached = new Set()) {
		const active = new Set();
		const fail = (at, location, reason, code = "CRU019") => {
			report(at, code, `${node.arguments[0]?.text ?? kind} ${location}: ${reason}`);
			return { kind: "nil" };
		};
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
			}
		};
		const declared = (type, location) => checkSymbols([type.aliasSymbol, type.getSymbol()], location);
		const referenced = new Set();
		const references = (syntax, location) => {
			if (!syntax || referenced.has(syntax)) return;
			referenced.add(syntax);
			if (ts.isTypeReferenceNode(syntax)) {
				const symbol = unalias(checker.getSymbolAtLocation(syntax.typeName));
				checkSymbols([symbol], location);
				for (const decl of symbol?.declarations ?? [])
					if (ts.isTypeAliasDeclaration(decl) && !exempt(decl.getSourceFile()))
						references(decl.type, location);
			}
			ts.forEachChild(syntax, (child) => references(child, location));
		};
		for (const type of node.typeArguments ?? []) references(type, "payload");
		const instance = (classes, location, position) => {
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
				return fail(node, location, "an empty object type accepts anything; use undefined");
			return {
				kind: "object",
				fields: props.map((prop) => {
					const at = prop.valueDeclaration ?? prop.declarations?.[0] ?? node;
					references(at.type, `${location}.${prop.name}`);
					if (prop.escapedName.toString().startsWith("__@"))
						fail(node, location, "symbol property names cannot cross the network");
					if (ts.isNumericLiteralName(prop.name))
						fail(node, `${location}.${prop.name}`, "numeric property names are not supported; use a Map");
					if (prop.declarations?.some((d) => ts.isMethodSignature(d) || ts.isMethodDeclaration(d)))
						fail(node, `${location}.${prop.name}`, "functions cannot cross the network");
					const value = checker.getTypeOfSymbolAtLocation(prop, at);
					return [
						prop.name,
						lower(
							value,
							`${location}.${prop.name}`,
							{
								toClient: position.toClient,
								nilable: !!(prop.flags & ts.SymbolFlags.Optional) || absent(value),
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
			position = { ...position, nilable: position.nilable || missing };
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
					part.flags & flags.Object &&
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
			if (depth > limits.MAX_DEPTH) return fail(node, location, "schema exceeds MAX_DEPTH");
			if (active.has(type)) return fail(node, location, "recursive payload type");
			active.add(type);
			try {
				if (type.flags & flags.Any) return fail(node, location, "any cannot be checked; use unknown");
				if (type.flags & flags.Unknown) return { kind: "dynamic" };
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
				if (type.isIntersection())
					return fail(node, location, "only numeric width intersections are supported");
				const e = engine(type);
				if (e?.kind === "instance") return instance(e.classes, location, position);
				if (e?.kind === "unsupported") return fail(node, location, "Roblox datatype not supported in v1");
				if (e) return { kind: e.kind };
				const c = collection(type);
				const args = type.objectFlags & ts.ObjectFlags.Reference ? checker.getTypeArguments(type) : [];
				const element = (t, location) => {
					if (absent(t)) fail(node, location, "collection elements and map values cannot admit undefined");
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
				if (c === "array") return { kind: "array", element: element(args[0], `${location}[]`) };
				if (c === "map")
					return {
						kind: "map",
						key: key(args[0], `${location}.key`),
						value: element(args[1], `${location}.value`),
					};
				if (type.flags & flags.Object) {
					const indices = checker.getIndexInfosOfType(type);
					if (
						indices.length === 1 &&
						indices[0].keyType.flags & flags.String &&
						!checker.getPropertiesOfType(type).length
					)
						return {
							kind: "map",
							key: { kind: "string" },
							value: element(indices[0].type, `${location}.value`),
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
		const result = (node.typeArguments ?? []).map((type, i) =>
			lower(
				checker.getTypeFromTypeNode(type),
				i === 1 ? "response" : kind === "askServer" ? "request" : "payload",
				{ toClient: kind === "toClient" || i === 1, nilable: false },
			),
		);
		const cost = (n, depth = 1) => {
			if (depth > limits.MAX_DEPTH) fail(node, "payload", "schema exceeds MAX_DEPTH");
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
			fail(node, "payload", "schema exceeds MAX_SCHEMA_NODES");
		return result;
	}
	let reached;
	let entryFiles;
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
				if (resolved) entryFiles.set(canonical(resolved.resolvedFileName), s);
			}
		}
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
		const inspect = (node) => {
			if (ts.isElementAccessExpression(node)) {
				const name = api(checker.getTypeAtLocation(node).getSymbol());
				if (macros.includes(name) && !(ts.isCallExpression(node.parent) && node.parent.expression === node))
					report(node, "CRU023", `${name} is a declaration macro and cannot be used as a value.`);
			}
			if (
				ts.isModuleDeclaration(node) &&
				(node.flags & ts.NodeFlags.GlobalAugmentation || ts.isStringLiteral(node.name))
			) {
				if (!reached) {
					reached = new Set();
					for (const declaration of index())
						lowerMessage(declaration.node, declaration.kind, () => {}, reached);
				}
				const walk = (child) => {
					if (ts.isIdentifier(child) && reached.has(unalias(checker.getSymbolAtLocation(child))))
						report(
							node,
							"CRU010",
							"This augmentation changes a network payload; incremental builds cannot see the merge. Declare the type in one module.",
						);
					ts.forEachChild(child, walk);
				};
				walk(node);
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
				const name = api(symbol);
				if (macros.includes(name)) {
					const p = node.parent;
					const expression = ts.isPropertyAccessExpression(p) && p.name === node ? p : node;
					if (
						!(ts.isCallExpression(expression.parent) && expression.parent.expression === expression) &&
						!ts.isImportSpecifier(p) &&
						!ts.isExportSpecifier(p)
					)
						report(node, "CRU023", `${name} is a declaration macro and cannot be used as a value.`);
				}
			}
		};
		const visit = (node) => {
			if (isMacro(node, "receive")) {
				const name = receive(node);
				const cb = node.arguments[1];
				let typed = cb;
				if (cb && (ts.isArrowFunction(cb) || ts.isFunctionExpression(cb))) {
					const params = cb.parameters.map((p) =>
						f.updateParameterDeclaration(
							p,
							p.modifiers,
							p.dotDotDotToken,
							p.name,
							p.questionToken,
							p.type ??
								checker.typeToTypeNode(
									checker.getTypeAtLocation(p),
									p,
									ts.NodeBuilderFlags.NoTruncation,
								),
							p.initializer,
						),
					);
					typed = ts.isArrowFunction(cb)
						? f.updateArrowFunction(
								cb,
								cb.modifiers,
								cb.typeParameters,
								params,
								cb.type,
								cb.equalsGreaterThanToken,
								cb.body,
							)
						: f.updateFunctionExpression(
								cb,
								cb.modifiers,
								cb.asteriskToken,
								cb.name,
								cb.typeParameters,
								params,
								cb.type,
								cb.body,
							);
				}
				return (
					name &&
					f.createCallExpression(runtime("receive"), undefined, [
						f.createThis(),
						str(name),
						visitInput(node.arguments[0]),
						visitInput(typed),
					])
				);
			}
			const kind = macros.slice(0, 3).find((name) => isMacro(node, name));
			if (!kind) return;
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
				side(node) !== "shared"
			)
				report(node, "CRU017", "Declare messages as an exported top-level const in a shared module.");
			if (node.typeArguments?.length !== (kind === "askServer" ? 2 : 1))
				report(node, "CRU017", `${kind} needs explicit payload types; use undefined for no payload.`);
			const name = node.arguments[0];
			if (
				!name ||
				!ts.isStringLiteral(name) ||
				Buffer.byteLength(name.text) > limits.MESSAGE_NAME_MAX_BYTES ||
				!/^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)*$/.test(name.text) ||
				name.text.split(".")[0] === "Crucible"
			)
				report(
					node,
					"CRU018",
					"Message names are 1 to 64 bytes of dot-separated segments [A-Za-z][A-Za-z0-9_]*; Crucible is reserved.",
				);
			else {
				const other = index().find((d) => d.node !== node && d.name === name.text);
				if (other) {
					const sf = other.node.getSourceFile();
					report(
						node,
						"CRU018",
						`${name.text} is already declared at ${sf.fileName}:${sf.getLineAndCharacterOfPosition(other.node.getStart()).line + 1}.`,
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
			if (node.arguments.length > (kind === "askServer" ? 2 : 1))
				report(node, "CRU017", "Unexpected declaration arguments.");
			return f.createCallExpression(runtime(kind), node.typeArguments, [
				name ?? str("invalid"),
				...lowerMessage(node, kind, report).map(emit),
				...(kind === "askServer" ? [timeout] : []),
			]);
		};
		return { inspect, visit };
	};
}

module.exports = { createNetwork, limits };
