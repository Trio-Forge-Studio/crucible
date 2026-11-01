const path = require("node:path");
const { createRequire } = require("node:module");
const { packageOf, typeId, sourceRoot } = require("./type-id.cjs");
const { createReporter } = require("./diagnostics.cjs");
const { createNetwork } = require("./network.cjs");
const { createDataChecks } = require("./data.cjs");

module.exports = function transformer(program, config = {}, extras = {}) {
	const ts = extras.ts ?? require("typescript");
	const checker = program.getTypeChecker();
	const f = ts.factory;
	const symbols = new Map();
	const unalias = (s) => s && (s.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(s) : s);
	const describe = (error) =>
		error instanceof RangeError && error.message === "Maximum call stack size exceeded"
			? "Cannot transform input: the type checker ran out of stack. A type in this file nests or inherits too deeply to analyse."
			: `Cannot transform input: ${String(error instanceof Error ? error.message : error).split(/\r?\n/)[0]}`;
	let startupFault;
	try {
		for (const source of program.getSourceFiles()) {
			let owner;
			try {
				owner = packageOf(source.fileName);
			} catch {
				startupFault = "Cannot read package metadata. Check the project's package.json files.";
				break;
			}
			if (owner?.json.name !== "@trioforge/crucible") continue;
			const module = checker.getSymbolAtLocation(source);
			if (!module) continue;
			for (const symbol of checker.getExportsOfModule(module)) {
				const target = unalias(symbol);
				if (!symbols.has(target)) symbols.set(target, symbol.name);
			}
		}
	} catch (error) {
		startupFault = describe(error);
	}
	const api = (symbol) => symbols.get(unalias(symbol));
	const modifiers = (node, flags) => (ts.getCombinedModifierFlags(node) & flags) !== 0;
	const bases = (type) => (type.target ?? type).getBaseTypes?.() ?? [];
	const instanceType = (node) => {
		const located = checker.getTypeAtLocation(node);
		const signature =
			ts.isClassExpression(node) && checker.getSignaturesOfType(located, ts.SignatureKind.Construct)[0];
		return signature ? checker.getReturnTypeOfSignature(signature) : located;
	};
	const kindOf = (type, seen = new Set()) => {
		if (seen.has(type)) return undefined;
		seen.add(type);
		const name = api(type.getSymbol());
		if (name === "Subsystem") return "subsystem";
		if (name === "CrucibleBehaviour") return "behaviour";
		for (const base of bases(type)) {
			const kind = kindOf(base, seen);
			if (kind) return kind;
		}
	};
	const diagnosticSink = extras.addDiagnostic;
	let report;
	let networkProgram;
	let dataProgram;
	return (context) => (source) => {
		if (source.isDeclarationFile) return source;
		let used = false;
		report ??= createReporter(ts, diagnosticSink ?? ((d) => context.addDiagnostic(d)));
		if (startupFault) {
			report(source, "Crucible", startupFault);
			return source;
		}
		const isGlobal = (decl) => {
			if (!ts.isExternalModule(decl.getSourceFile())) return true;
			for (let p = decl.parent; p; p = p.parent)
				if (ts.isModuleDeclaration(p) && p.flags & ts.NodeFlags.GlobalAugmentation) return true;
			return false;
		};
		const runtime = (name) => {
			used = true;
			return f.createPropertyAccessExpression(f.createIdentifier("__crucible"), name);
		};
		const call = (expression, args, types) => f.createCallExpression(expression, types, args);
		const str = (s) => f.createStringLiteral(s);
		const array = (values) => f.createArrayLiteralExpression(values);
		const object = (values) =>
			f.createObjectLiteralExpression(
				Object.entries(values).map(([key, value]) => f.createPropertyAssignment(str(key), value)),
				true,
			);
		const globalType = (type, at, seen = new Set()) => {
			if (seen.has(type)) return;
			seen.add(type);
			for (const symbol of [type.aliasSymbol, type.getSymbol()]) {
				for (const decl of symbol?.declarations ?? []) {
					if (
						program.isSourceFileDefaultLibrary(decl.getSourceFile()) ||
						/node_modules[\\/]@rbxts[\\/]/.test(decl.getSourceFile().fileName)
					)
						continue;
					if (isGlobal(decl))
						report(
							at,
							"CRU010",
							"Type comes from global declarations; import a module type for incremental correctness.",
						);
				}
			}
			for (const part of type.isUnion() ? type.types : []) globalType(part, at, seen);
		};
		const globalReferences = (node, at, seen = new Set()) => {
			if (!node || seen.has(node)) return;
			seen.add(node);
			if (ts.isTypeReferenceNode(node)) {
				const symbol = unalias(checker.getSymbolAtLocation(node.typeName));
				for (const decl of symbol?.declarations ?? []) {
					if (ts.isTypeAliasDeclaration(decl)) {
						if (isGlobal(decl))
							report(
								at,
								"CRU010",
								"Attribute and dependency aliases must come from module declarations.",
							);
						globalReferences(decl.type, at, seen);
					}
				}
			}
			ts.forEachChild(node, (child) => globalReferences(child, at, seen));
		};
		const id = (type, at) => {
			globalType(type, at);
			const symbol = type.getSymbol();
			const decl = symbol?.declarations?.find((d) => ts.isClassDeclaration(d) || ts.isInterfaceDeclaration(d));
			if (!decl) return undefined;
			const value = typeId(decl.getSourceFile().fileName, symbol.name, program, ts);
			if (!value)
				report(
					at,
					"Crucible",
					"Cannot compute a type id. The declaring package needs a name and a mirrored source/declaration layout.",
				);
			return value;
		};
		const allTarget = (type) => (api(type.getSymbol()) === "All" ? checker.getTypeArguments(type)[0] : undefined);
		const injectable = (type, at, all = false) => {
			const target = allTarget(type);
			if (target && !all) return object({ all: injectable(target, at, true) });
			globalType(type, at);
			const symbol = type.getSymbol();
			const isNominal =
				symbol?.declarations?.some((d) => ts.isClassDeclaration(d) || ts.isInterfaceDeclaration(d)) &&
				checker.getDeclaredTypeOfSymbol(symbol) === (type.target ?? type);
			const engineInstance = checker.resolveName("Instance", at, ts.SymbolFlags.Type, false);
			const instance =
				engineInstance && checker.isTypeAssignableTo(type, checker.getDeclaredTypeOfSymbol(engineInstance));
			const generic =
				type.objectFlags & ts.ObjectFlags.Reference &&
				checker.getTypeArguments(type).length > 0 &&
				!(all && !genericTarget(type, "target"));
			const concreteBehaviour =
				kindOf(type) === "behaviour" &&
				symbol?.declarations?.some((d) => ts.isClassDeclaration(d) && !modifiers(d, ts.ModifierFlags.Abstract));
			if (!isNominal || instance || type.isUnion() || generic || (!all && concreteBehaviour)) {
				report(
					at,
					"CRU001",
					concreteBehaviour
						? "Inject All<X> or call X.get(instance) for a concrete behaviour."
						: "Constructor parameter must be a plain class, interface or All<T>.",
				);
				return str("invalid");
			}
			const value = id(type, at);
			return str(value ?? "invalid");
		};
		const check = (type, at) => {
			globalType(type, at);
			const t = (name) => f.createPropertyAccessExpression(runtime("t"), name);
			if (type.flags & ts.TypeFlags.Boolean) return t("boolean");
			if (type.isUnion()) {
				const parts = type.types.filter((p) => !(p.flags & ts.TypeFlags.Undefined));
				const booleanPair =
					parts.some((p) => p.flags & ts.TypeFlags.BooleanLiteral && p.intrinsicName === "true") &&
					parts.some((p) => p.flags & ts.TypeFlags.BooleanLiteral && p.intrinsicName === "false");
				const checks = parts
					.filter((p) => !(booleanPair && p.flags & ts.TypeFlags.BooleanLiteral))
					.map((p) => check(p, at));
				if (booleanPair) checks.push(t("boolean"));
				return checks.length === 1 ? checks[0] : call(t("union"), checks);
			}
			if (type.flags & ts.TypeFlags.String) return t("string");
			if (type.flags & ts.TypeFlags.Number) return t("number");
			if (type.isStringLiteral()) return call(t("literal"), [str(type.value)]);
			if (type.isNumberLiteral())
				return call(t("literal"), [
					type.value < 0
						? f.createPrefixUnaryExpression(ts.SyntaxKind.MinusToken, f.createNumericLiteral(-type.value))
						: f.createNumericLiteral(type.value),
				]);
			if (type.flags & ts.TypeFlags.BooleanLiteral)
				return call(t("literal"), [type.intrinsicName === "true" ? f.createTrue() : f.createFalse()]);
			const name = type.getSymbol()?.name;
			const engineType = name && checker.resolveName(name, at, ts.SymbolFlags.Type, false);
			if ((name === "Vector3" || name === "Color3") && engineType === type.getSymbol())
				return t(name.toLowerCase());
			report(
				at,
				"CRU007",
				"Attribute fields support string, number, boolean, Vector3, Color3 and literal unions.",
			);
			return t("string");
		};
		const isMacro = (node, name) => {
			if (!ts.isCallExpression(node)) return false;
			const declaration = checker.getResolvedSignature(node)?.declaration;
			return (
				api(checker.getSymbolAtLocation(node.expression)) === name ||
				(declaration?.name && api(checker.getSymbolAtLocation(declaration.name)) === name)
			);
		};
		const attributes = (type) => {
			const result = {};
			for (const prop of checker.getPropertiesOfType(type)) {
				const decl = prop.valueDeclaration;
				if (
					!decl ||
					!ts.isPropertyDeclaration(decl) ||
					!decl.initializer ||
					!isMacro(decl.initializer, "attribute")
				)
					continue;
				const required = decl.initializer.arguments.length === 0;
				let valueCheck = check(checker.getTypeOfSymbolAtLocation(prop, decl), decl);
				if (!required)
					valueCheck = call(f.createPropertyAccessExpression(runtime("t"), "optional"), [valueCheck]);
				result[prop.name] = object({
					name: str(prop.name[0].toUpperCase() + prop.name.slice(1)),
					check: valueCheck,
					required: required ? f.createTrue() : f.createFalse(),
				});
			}
			return result;
		};
		const registration = (node) => {
			const type = instanceType(node);
			const kind = kindOf(type);
			if (
				!kind ||
				modifiers(node, ts.ModifierFlags.Abstract | ts.ModifierFlags.Ambient) ||
				node.flags & ts.NodeFlags.Ambient
			)
				return;
			let ctor;
			let current = type;
			const constructors = new Set();
			while (current && !constructors.has(current)) {
				constructors.add(current);
				const declaration = current.getSymbol()?.declarations?.find(ts.isClassDeclaration);
				ctor = declaration?.members.find(ts.isConstructorDeclaration);
				if (ctor) break;
				current = bases(current)[0];
			}
			if (
				!ts.isClassDeclaration(node) ||
				!node.name ||
				!ts.isSourceFile(node.parent) ||
				node.typeParameters?.length ||
				(ctor && modifiers(ctor, ts.ModifierFlags.Private | ts.ModifierFlags.Protected))
			) {
				report(
					node,
					"CRU003",
					"Registered classes must be named, non-generic, module-level classes with public constructors.",
				);
				return;
			}
			const staticType = checker.getTypeOfSymbolAtLocation(checker.getSymbolAtLocation(node.name), node);
			const construct = checker.getSignaturesOfType(staticType, ts.SignatureKind.Construct)[0];
			const deps = (construct?.parameters ?? []).map((param) => {
				const decl = param.valueDeclaration ?? node;
				globalReferences(decl.type, decl);
				if (decl.questionToken || decl.dotDotDotToken || decl.initializer)
					report(decl, "CRU001", "Optional, defaulted and rest parameters are not injectable.");
				return injectable(checker.getTypeOfSymbolAtLocation(param, node), decl);
			});
			const provides = new Set();
			const seen = new Set();
			const provide = (current) => {
				if (
					seen.has(current) ||
					["Subsystem", "CrucibleBehaviour", "CrucibleObject"].includes(api(current.getSymbol()))
				)
					return;
				seen.add(current);
				if (current !== type) {
					const value = id(current, node);
					if (value) provides.add(value);
				}
				for (const base of bases(current)) provide(base);
				for (const decl of current.getSymbol()?.declarations ?? []) {
					for (const clause of decl.heritageClauses ?? [])
						if (clause.token === ts.SyntaxKind.ImplementsKeyword)
							for (const target of clause.types) provide(checker.getTypeAtLocation(target));
				}
			};
			provide(type);
			const data = {
				kind: str(kind),
				id: str(id(type, node) ?? "invalid"),
				deps: array(deps),
				provides: array([...provides].map(str)),
			};
			if (kind === "behaviour") {
				const memberType = (name) => {
					const prop = checker.getPropertyOfType(type, name);
					return prop && checker.getTypeOfSymbolAtLocation(prop, node);
				};
				const tag = memberType("tag");
				if (!tag?.isStringLiteral())
					report(node, "CRU002", "A concrete behaviour tag must be exactly one string literal.");
				data.tag = str(tag?.isStringLiteral() ? tag.value : "invalid");
				const instance = memberType("instance");
				const parts = instance?.isUnion() ? instance.types : [instance];
				const instances = checker.resolveName("Instances", node, ts.SymbolFlags.Type, false);
				const allowed = instances && checker.getDeclaredTypeOfSymbol(instances);
				data.instance = array(
					parts.map((part) => {
						const name = part?.getSymbol()?.name;
						const expected = name && allowed && checker.getPropertyOfType(allowed, name);
						if (
							!expected ||
							checker.getTypeOfSymbolAtLocation(expected, node).getSymbol() !== part.getSymbol()
						)
							report(node, "CRU008", "Instance type must be an Instances class or union of them.");
						return str(name ?? "Instance");
					}),
				);
				const fields = attributes(type);
				if (Object.keys(fields).length) data.attributes = object(fields);
			}
			return f.createExpressionStatement(
				call(runtime("defineClass"), [f.createIdentifier(node.name.text), object(data)]),
			);
		};
		const boot = (node) => {
			let options = node.arguments[0];
			while (
				options &&
				(ts.isParenthesizedExpression(options) ||
					ts.isAsExpression(options) ||
					ts.isTypeAssertionExpression(options) ||
					ts.isSatisfiesExpression(options))
			)
				options = options.expression;
			if (!options || !ts.isObjectLiteralExpression(options)) {
				report(node, "CRU006", "boot requires literal load globs.");
				return node;
			}
			const loadIndex = options.properties.findLastIndex(
				(prop) => prop.name && (prop.name.text ?? prop.name.getText()) === "load",
			);
			if (loadIndex < 0) report(options, "CRU006", "boot requires a literal load property.");
			const properties = options.properties.map((prop, index) => {
				if (ts.isSpreadAssignment(prop) || (prop.name && ts.isComputedPropertyName(prop.name))) {
					if (index > loadIndex)
						report(prop, "CRU006", "A spread or computed key after load can replace its literal globs.");
					return prop;
				}
				if (!prop.name || (prop.name.text ?? prop.name.getText()) !== "load") return prop;
				if (!ts.isPropertyAssignment(prop) || !ts.isArrayLiteralExpression(prop.initializer)) {
					report(prop, "CRU006", "load must be an array of string literals.");
					return prop;
				}
				const entries = prop.initializer.elements.map((entry) => {
					if (!ts.isStringLiteral(entry)) {
						report(entry, "CRU006", "load entry must be a string literal.");
						return entry;
					}
					const root = path.dirname(
						program.getCompilerOptions().configFilePath ??
							path.join(program.getCurrentDirectory(), "tsconfig.json"),
					);
					let resolver;
					try {
						const compiler =
							Object.keys(require.cache).find((file) =>
								/roblox-ts[\\/]out[\\/]Project[\\/]functions[\\/]compileFiles\.js$/.test(file),
							) ?? require.resolve("roblox-ts/package.json", { paths: [root, __dirname] });
						const { RojoResolver } = createRequire(compiler)("@roblox-ts/rojo-resolver");
						const rojoPath = config.rojo ?? RojoResolver.findRojoConfigFilePath(root).path;
						resolver = rojoPath && RojoResolver.fromPath(path.resolve(root, rojoPath));
					} catch {
						report(entry, "CRU006", "Cannot load the running compiler's Rojo resolver or project mapping.");
						return entry;
					}
					const globAt = entry.text.search(/[?*{[]/);
					const prefix =
						globAt < 0
							? path.dirname(entry.text) + "/"
							: entry.text.slice(0, entry.text.lastIndexOf("/", globAt) + 1);
					const suffix = entry.text.slice(prefix.length).replace(/\.tsx?$/, "");
					const opts = program.getCompilerOptions();
					const inputRoot = sourceRoot(opts, [program.getCommonSourceDirectory()]);
					const relative = inputRoot && path.relative(inputRoot, path.resolve(root, prefix));
					const mapped =
						relative !== undefined &&
						!relative.startsWith("..") &&
						!path.isAbsolute(relative) &&
						resolver?.getRbxPathFromFilePath(path.resolve(opts.outDir, relative));
					if (!mapped) {
						report(entry, "CRU006", "load prefix is outside the Rojo mapping.");
						return entry;
					}
					return str(`crucible-path:${[...mapped, suffix].join("/")}`);
				});
				return f.updatePropertyAssignment(prop, prop.name, array(entries));
			});
			return f.updateCallExpression(node, node.expression, node.typeArguments, [
				f.updateObjectLiteralExpression(options, properties),
			]);
		};

		const resolvedBases = (type) => type.getBaseTypes?.() ?? bases(type);

		const concreteBase = (base, seen = new Set()) => {
			if (seen.has(base)) return false;
			seen.add(base);
			if (base.isIntersection()) return base.types.some((part) => concreteBase(part, seen));
			if (
				kindOf(base) &&
				!api(base.getSymbol()) &&
				base
					.getSymbol()
					?.declarations?.some(
						(decl) => ts.isClassDeclaration(decl) && !modifiers(decl, ts.ModifierFlags.Abstract),
					)
			)
				return true;
			return resolvedBases(base).some((parent) => concreteBase(parent, seen));
		};
		const genericTarget = (type, mode, seen = new Set()) => {
			if (!type || seen.has(type)) return false;
			seen.add(type);
			if (type.isUnionOrIntersection()) return type.types.some((part) => genericTarget(part, mode, seen));
			if (["Subsystem", "CrucibleBehaviour", "CrucibleObject"].includes(api(type.getSymbol()))) return false;
			const args = type.objectFlags & ts.ObjectFlags.Reference ? checker.getTypeArguments(type) : [];
			if (args.length) {
				const target = type.target ?? type;
				const tag = checker.getPropertyOfType(target, "tag");
				const tagType = tag && checker.getTypeOfSymbolAtLocation(tag, tag.valueDeclaration ?? source);
				const parameters = target.typeParameters ?? [];
				if (
					args.some(
						(arg, i) =>
							kindOf(target) !== "behaviour" ||
							parameters[i] !== tagType ||
							(mode !== "provides" && arg.flags !== ts.TypeFlags.String),
					)
				)
					return true;
			}
			const baseMode = type.getSymbol()?.declarations?.some(ts.isClassDeclaration) ? "provides" : mode;
			if (resolvedBases(type).some((base) => genericTarget(base, baseMode, seen))) return true;
			for (const decl of type.getSymbol()?.declarations ?? [])
				for (const clause of decl.heritageClauses ?? [])
					if (clause.token === ts.SyntaxKind.ImplementsKeyword)
						for (const target of clause.types)
							if (genericTarget(checker.getTypeAtLocation(target), mode, seen)) return true;
			return false;
		};
		const hooks = new Set();
		for (const [symbol, name] of symbols) {
			if (!["Subsystem", "CrucibleBehaviour", "CrucibleObject"].includes(name)) continue;
			for (const decl of symbol.declarations ?? [])
				for (const member of decl.members ?? [])
					if (
						ts.isMethodDeclaration(member) &&
						modifiers(member, ts.ModifierFlags.Protected) &&
						!modifiers(member, ts.ModifierFlags.Static)
					)
						hooks.add(member.name.getText());
		}
		const near = (a, b) => {
			if (Math.abs(a.length - b.length) > 1) return false;
			let i = 0,
				j = 0,
				edits = 0;
			while (i < a.length && j < b.length) {
				if (a[i] === b[j]) {
					i++;
					j++;
					continue;
				}
				if (++edits > 1) return false;
				if (a.length >= b.length) i++;
				if (b.length >= a.length) j++;
			}
			return edits + a.length - i + b.length - j === 1;
		};
		const promise = (type, seen = new Set()) => {
			if (!type || seen.has(type)) return false;
			seen.add(type);
			return (
				type.getSymbol()?.name === "Promise" ||
				(type.isUnionOrIntersection() && type.types.some((part) => promise(part, seen))) ||
				resolvedBases(type).some((base) => promise(base, seen))
			);
		};
		const sideRoot = packageOf(source.fileName)?.dir ?? path.parse(path.resolve(source.fileName)).root;
		const side = (node) =>
			path
				.relative(sideRoot, node.getSourceFile().fileName)
				.replaceAll("\\", "/")
				.split("/")
				.slice(0, -1)
				.reverse()
				.find((part) => ["server", "client", "shared"].includes(part));
		let network;
		let data;
		const inspect = (node) => {
			network.inspect(node);
			data(node);
			if (
				ts.isImportDeclaration(node) ||
				ts.isExportDeclaration(node) ||
				ts.isImportTypeNode(node) ||
				ts.isExternalModuleReference(node) ||
				(ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword)
			) {
				const specifier =
					node.moduleSpecifier ?? node.argument?.literal ?? node.arguments?.[0] ?? node.expression;
				if (
					specifier &&
					ts.isStringLiteral(specifier) &&
					specifier.text.startsWith("@trioforge/crucible/") &&
					!["@trioforge/crucible/server", "@trioforge/crucible/client"].includes(specifier.text)
				)
					report(node, "CRU004", "Import only the root, server or client Crucible entries.");
			}
			if (ts.isClassExpression(node) || ts.isClassDeclaration(node)) {
				const type = instanceType(node);
				if (resolvedBases(type).some((base) => concreteBase(base)))
					report(node, "CRU013", "Extend an abstract Crucible base, not a concrete registered class.");
				if (kindOf(type)) {
					if (!modifiers(node, ts.ModifierFlags.Abstract) && genericTarget(type, "provides"))
						report(node, "CRU014", "Provided types must not instantiate non-tag generic parameters.");
					for (const clause of node.heritageClauses ?? [])
						if (clause.token === ts.SyntaxKind.ImplementsKeyword)
							for (const target of clause.types)
								if (genericTarget(checker.getTypeAtLocation(target), "provides"))
									report(
										node,
										"CRU014",
										"Provided interfaces must not instantiate generic parameters.",
									);
					const seen = new Set();
					const members = (current) => {
						if (seen.has(current) || api(current.getSymbol())) return;
						seen.add(current);
						for (const decl of current.getSymbol()?.declarations ?? [])
							for (const member of decl.members ?? []) {
								const name = member.name && checker.getSymbolAtLocation(member.name)?.name;
								if (!name) continue;
								const at = decl === node ? member : node;
								if (side(node) !== "client" && (name === "preRender" || name === "renderPriority"))
									report(at, "CRU011", "preRender and renderPriority require a client class.");
								if (side(node) !== "server" && name === "onShutdown")
									report(at, "CRU016", "onShutdown requires a server class.");
							}
						for (const base of resolvedBases(current)) members(base);
					};
					members(type);
					for (const member of node.members) {
						if (!ts.isMethodDeclaration(member)) continue;
						const name = checker.getSymbolAtLocation(member.name)?.name;
						if (!name) continue;
						if (
							!hooks.has(name) &&
							(["onInit", "onStart", "onTick", "onPhysics", "onRender"].includes(name) ||
								[...hooks].some(
									(hook) => hook.toLowerCase() === name.toLowerCase() || near(name, hook),
								))
						)
							report(
								member,
								"CRU009",
								"Method resembles a lifecycle hook. Use its exact Crucible declaration name.",
							);
						const signature = checker.getSignatureFromDeclaration(member);
						if (
							hooks.has(name) &&
							(modifiers(member, ts.ModifierFlags.Async) ||
								(signature && promise(checker.getReturnTypeOfSignature(signature))))
						)
							report(
								member,
								"CRU012",
								"Crucible hooks must be synchronous and must not return a Promise.",
							);
					}
				}
			}
			if (ts.isTypeReferenceNode(node)) {
				const type = checker.getTypeFromTypeNode(node);
				const target = allTarget(type);
				if (target && genericTarget(target, "target"))
					report(node, "CRU014", "All targets must not instantiate generic parameters except string tags.");
			}
			if (isMacro(node, "inject")) {
				const own = node.typeArguments?.[0];
				const type = own && checker.getTypeFromTypeNode(own);
				const target = type && (allTarget(type) ?? type);
				let inside = false;
				for (let parent = node.parent; parent; parent = parent.parent)
					if (
						(ts.isClassDeclaration(parent) || ts.isClassExpression(parent)) &&
						kindOf(instanceType(parent)) &&
						!modifiers(parent, ts.ModifierFlags.Abstract)
					)
						inside = true;
				const symbol = target?.getSymbol();
				if (
					inside ||
					!symbol?.declarations?.some(
						(decl) => ts.isClassDeclaration(decl) || ts.isInterfaceDeclaration(decl),
					) ||
					checker.getDeclaredTypeOfSymbol(symbol) !== (target.target ?? target)
				)
					report(
						node,
						"CRU005",
						"inject requires a class, interface or All target outside registered classes.",
					);
				if (target && genericTarget(target, "target"))
					report(
						node,
						"CRU014",
						"inject targets must not instantiate generic parameters except string tags.",
					);
			}
			ts.forEachChild(node, inspect);
		};

		const visit = (node) => {
			const networkNode = network.visit(node);
			if (networkNode) return networkNode;
			if (isMacro(node, "attribute")) {
				const field = node.parent;
				if (
					!ts.isPropertyDeclaration(field) ||
					field.initializer !== node ||
					modifiers(field, ts.ModifierFlags.Static) ||
					kindOf(instanceType(field.parent)) !== "behaviour"
				) {
					report(node, "CRU015", "attribute() must initialize a behaviour instance field.");
					return node;
				}
				globalReferences(field.type, field);
				for (const type of node.typeArguments ?? []) globalReferences(type, field);
				check(checker.getTypeAtLocation(field), field);
				const typeNode =
					node.typeArguments?.[0] ??
					checker.typeToTypeNode(checker.getTypeAtLocation(field), field, ts.NodeBuilderFlags.NoTruncation);
				return call(
					runtime("attribute"),
					[
						f.createThis(),
						str(checker.getSymbolAtLocation(field.name)?.name ?? field.name.getText()),
						...node.arguments.map((arg) => ts.visitNode(arg, visit)),
					],
					[typeNode],
				);
			}
			if (isMacro(node, "inject")) {
				const own = node.typeArguments?.[0];
				if (!own) return node;
				const type = checker.getTypeFromTypeNode(own);
				const target = allTarget(type);
				const directAll =
					ts.isTypeReferenceNode(own) && api(checker.getSymbolAtLocation(own.typeName)) === "All";
				const typeNode = target
					? directAll
						? own.typeArguments?.[0]
						: f.createConditionalTypeNode(
								own,
								f.createImportTypeNode(
									f.createLiteralTypeNode(str("@trioforge/crucible")),
									undefined,
									f.createIdentifier("All"),
									[
										f.createInferTypeNode(
											f.createTypeParameterDeclaration(
												undefined,
												"__CrucibleItem",
												f.createKeywordTypeNode(ts.SyntaxKind.ObjectKeyword),
											),
										),
									],
								),
								f.createTypeReferenceNode("__CrucibleItem"),
								f.createKeywordTypeNode(ts.SyntaxKind.NeverKeyword),
							)
					: own;
				return call(
					runtime(target ? "resolveAll" : "resolve"),
					[str(id(target ?? type, node) ?? "invalid")],
					[typeNode ?? own],
				);
			}
			if (isMacro(node, "boot")) return boot(node);
			const next = ts.visitEachChild(node, visit, context);
			if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) {
				const emitted = registration(node);
				if (emitted) return [next, emitted];
			}
			return next;
		};
		try {
			networkProgram ??= createNetwork({ ts, program, checker, api, kindOf, instanceType, modifiers, isMacro });
			network = networkProgram({
				source,
				report,
				runtime,
				side,
				visitInput: (node) => ts.visitNode(node, visit),
			});
			dataProgram ??= createDataChecks({ ts, program, checker, api });
			data = dataProgram({ report, side });
			inspect(source);
			const next = ts.visitEachChild(source, visit, context);
			if (!used) return next;
			const imports = f.createImportDeclaration(
				undefined,
				f.createImportClause(false, undefined, f.createNamespaceImport(f.createIdentifier("__crucible"))),
				str("@trioforge/crucible/emit"),
			);
			return f.updateSourceFile(next, [imports, ...next.statements]);
		} catch (error) {
			report(source, "Crucible", describe(error));
			return source;
		}
	};
};
