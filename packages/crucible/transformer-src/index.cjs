const path = require("node:path");
const { createRequire } = require("node:module");
const { packageOf, typeId, sourceRoot } = require("./type-id.cjs");
const { createReporter } = require("./diagnostics.cjs");

module.exports = function transformer(program, config = {}, extras = {}) {
	const ts = extras.ts ?? require("typescript");
	const checker = program.getTypeChecker();
	const f = ts.factory;
	const symbols = new Map();
	const unalias = (s) => s && (s.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(s) : s);
	let invalidMetadata = false;
	try {
		for (const source of program.getSourceFiles()) {
			const owner = packageOf(source.fileName);
			if (owner?.json.name !== "@trioforge/crucible") continue;
			const module = checker.getSymbolAtLocation(source);
			if (!module) continue;
			for (const symbol of checker.getExportsOfModule(module)) {
				const target = unalias(symbol);
				if (!symbols.has(target)) symbols.set(target, symbol.name);
			}
		}
	} catch {
		invalidMetadata = true;
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
	return (context) => (source) => {
		if (source.isDeclarationFile) return source;
		let used = false;
		report ??= createReporter(ts, diagnosticSink ?? ((d) => context.addDiagnostic(d)));
		if (invalidMetadata) {
			report(source, "Crucible", "Cannot read package metadata. Check the project's package.json files.");
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
			const isNominal = symbol?.declarations?.some(
				(d) => ts.isClassDeclaration(d) || ts.isInterfaceDeclaration(d),
			);
			const engineInstance = checker.resolveName("Instance", at, ts.SymbolFlags.Type, false);
			const instance =
				engineInstance && checker.isTypeAssignableTo(type, checker.getDeclaredTypeOfSymbol(engineInstance));
			const generic = type.objectFlags & ts.ObjectFlags.Reference && checker.getTypeArguments(type).length > 0;
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
			if (!kind || modifiers(node, ts.ModifierFlags.Abstract)) return;
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
		const visit = (node) => {
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
				return call(runtime("attribute"), [
					f.createThis(),
					str(checker.getSymbolAtLocation(field.name)?.name ?? field.name.getText()),
					...node.arguments.map((arg) => ts.visitNode(arg, visit)),
				]);
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
			const next = ts.visitEachChild(source, visit, context);
			if (!used) return next;
			const imports = f.createImportDeclaration(
				undefined,
				f.createImportClause(false, undefined, f.createNamespaceImport(f.createIdentifier("__crucible"))),
				str("@trioforge/crucible/emit"),
			);
			return f.updateSourceFile(next, [imports, ...next.statements]);
		} catch (error) {
			report(
				source,
				"Crucible",
				`Cannot transform input: ${String(error instanceof Error ? error.message : error).split(/\r?\n/)[0]}`,
			);
			return source;
		}
	};
};
