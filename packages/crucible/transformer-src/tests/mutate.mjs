import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const source = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(source, "../../..");
const scratch = path.join(repo, "build/transformer-mutants");
fs.rmSync(scratch, { recursive: true, force: true });
const registration = "registration contract";
const mutants = [
	[
		"mixin-walk-dropped",
		"index.cjs",
		"if (base.isIntersection()) return base.types.some((part) => concreteBase(part, seen));",
		"if (base.isIntersection()) return false;",
		"T2 CRU013 direct concrete mixin",
	],
	[
		"mixin-first-layer-only",
		"index.cjs",
		"return resolvedBases(base).some((parent) => concreteBase(parent, seen));",
		"return false;",
		"T2 CRU013 nested concrete mixins",
	],
	[
		"mixin-every-call-concrete",
		"index.cjs",
		"if (resolvedBases(type).some((base) => concreteBase(base)))",
		"if (resolvedBases(type).some((base) => concreteBase(base)) || node.heritageClauses?.some((clause) => clause.token === ts.SyntaxKind.ExtendsKeyword && clause.types.some((target) => ts.isCallExpression(target.expression))))",
		"T2 CRU013 abstract and unregistered mixins stay legal",
	],

	["registration", "index.cjs", "!kind ||", "true ||", registration],
	["kind", "index.cjs", "kind: str(kind)", 'kind: str("broken")', registration],
	[
		"abstract",
		"index.cjs",
		"ts.ModifierFlags.Abstract | ts.ModifierFlags.Ambient",
		"ts.ModifierFlags.Ambient",
		registration,
	],
	["generic-base", "index.cjs", "(type.target ?? type).getBaseTypes?.()", "type.getBaseTypes?.()", registration],
	[
		"symbol-identity",
		"index.cjs",
		"const api = (symbol) => symbols.get(unalias(symbol));",
		"const api = (symbol) => unalias(symbol)?.name;",
		"API identity",
	],
	["deps", "index.cjs", "deps: array(deps)", "deps: array([])", registration],
	["provides", "index.cjs", "provides: array([...provides].map(str))", "provides: array([])", registration],
	[
		"tag",
		"index.cjs",
		'data.tag = str(tag?.isStringLiteral() ? tag.value : "invalid")',
		'data.tag = str("broken")',
		registration,
	],
	[
		"instance",
		"index.cjs",
		"const parts = instance?.isUnion() ? instance.types : [instance]",
		"const parts = [instance]",
		registration,
	],
	[
		"attributes",
		"index.cjs",
		"const required = decl.initializer.arguments.length === 0",
		"const required = false",
		registration,
	],
	[
		"attribute-rewrite",
		"index.cjs",
		"str(checker.getSymbolAtLocation(field.name)?.name ?? field.name.getText())",
		'str("broken")',
		registration,
	],
	["inject", "index.cjs", 'runtime(target ? "resolveAll" : "resolve")', 'runtime("resolve")', registration],
	["boot", "index.cjs", "crucible-path:", "broken-path:", registration],
	["capitalization", "index.cjs", "prop.name[0].toUpperCase() + prop.name.slice(1)", "prop.name", registration],
	[
		"undefined-check",
		"index.cjs",
		"type.types.filter((p) => !(p.flags & ts.TypeFlags.Undefined))",
		"type.types",
		registration,
	],
	["type-id", "type-id.cjs", "if (/\\.d\\.tsx?$/.test(file))", "if (false)", "package declaration ids"],
	[
		"exports-types",
		"type-id.cjs",
		'typesCondition(owner.json.exports?.["."] ?? owner.json.exports)',
		"[]",
		"package declaration ids",
	],
	[
		"behaviour-provides-stop",
		"index.cjs",
		'["Subsystem", "CrucibleBehaviour", "CrucibleObject"]',
		'["Subsystem", "CrucibleObject"]',
		registration,
	],
	[
		"boolean-check",
		"index.cjs",
		'if (type.flags & ts.TypeFlags.Boolean) return t("boolean");',
		'if (type.flags & ts.TypeFlags.Boolean) return t("string");',
		registration,
	],
	[
		"alias-injection",
		"index.cjs",
		'f.createTypeReferenceNode("__CrucibleItem")',
		"own",
		"rbxtsc consumer plugin resolves",
	],
	["nested-all", "index.cjs", "target && !all", "target", "CRU001 rejects every"],
	[
		"class-expression",
		"index.cjs",
		"ts.isClassDeclaration(node) || ts.isClassExpression(node)",
		"ts.isClassDeclaration(node)",
		"CRU003 rejects nested",
	],
	[
		"missing-id",
		"diagnostics.cjs",
		"addDiagnostic({",
		'code === "Crucible" ? undefined : addDiagnostic({',
		"type id failures",
	],
	["spread-load", "index.cjs", "if (index > loadIndex)", "if (false)", "CRU006 rejects dynamic"],
	[
		"multiple-roots",
		"index.cjs",
		"sourceRoot(opts, [program.getCommonSourceDirectory()])",
		"opts.rootDir ?? opts.rootDirs?.[0]",
		"rootDirs globs",
	],
	["global-resolver", "index.cjs", "Object.keys(require.cache)", "[]", "global compiler resolver"],
	[
		"duplicate-diagnostics",
		"diagnostics.cjs",
		"if (seen.has(key)) return;",
		"if (false) return;",
		"diagnostics deduplicate",
	],
	["abstract-import", "index.cjs", "if (!used) return next;", "return next;", "review regressions"],
	["untouched-import", "index.cjs", "let used = false;", "let used = true;", "review regressions"],
	[
		"root-id-collision",
		"type-id.cjs",
		"root = sourceRoot(",
		"root = ((options) => options.rootDir ?? options.rootDirs?.[0])(",
		"rootDirs globs",
	],
	[
		"boolean-union",
		"index.cjs",
		'if (booleanPair) checks.push(t("boolean"));',
		'if (booleanPair) checks.push(t("string"));',
		registration,
	],
	["wrapped-options", "index.cjs", "ts.isParenthesizedExpression(options) ||", "false ||", "CRU006 rejects dynamic"],
	[
		"class-expression-field",
		"index.cjs",
		"kindOf(instanceType(field.parent))",
		"kindOf(checker.getTypeAtLocation(field.parent))",
		"CRU003 rejects nested",
	],

	["attribute-type-dropped", "index.cjs", "[typeNode],", "undefined,", "rbxtsc Van attribute reads"],
	[
		"check-colon-call",
		"tests/fixtures/api/emit.d.ts",
		"readonly optional: (check: Check) => Check;",
		"optional(check: Check): Check;",
		"rbxtsc Van attribute reads",
	],
	[
		"constructor-type-accepted",
		"index.cjs",
		"checker.getDeclaredTypeOfSymbol(symbol) === (type.target ?? type)",
		"true",
		"CRU001 rejects every",
	],
	[
		"ambient-registered",
		"index.cjs",
		"modifiers(node, ts.ModifierFlags.Abstract | ts.ModifierFlags.Ambient) ||\n\t\t\t\tnode.flags & ts.NodeFlags.Ambient",
		"modifiers(node, ts.ModifierFlags.Abstract)",
		"ambient classes emit",
	],
	[
		"global-alias-ignored",
		"index.cjs",
		"if (ts.isTypeAliasDeclaration(decl)) {\n\t\t\t\t\t\tif (isGlobal(decl))",
		"if (ts.isTypeAliasDeclaration(decl)) {\n\t\t\t\t\t\tif (false)",
		"CRU010 rejects global primitive aliases",
	],
	["keep-ts-extension", "index.cjs", '.replace(/\\.tsx?$/, "")', "", "CRU006 rejects dynamic"],
	[
		"N1-own-type-arg-ignored",
		"index.cjs",
		"node.typeArguments?.[0] ??",
		"",
		"attribute rewrite preserves a written alias type argument",
	],
	[
		"N2-abstract-typeof-accepted",
		"index.cjs",
		"checker.getDeclaredTypeOfSymbol(symbol) === (type.target ?? type)",
		"(checker.getDeclaredTypeOfSymbol(symbol) === (type.target ?? type) || symbol.declarations.some((d) => modifiers(d, ts.ModifierFlags.Abstract)))",
		"CRU001 rejects every",
	],
	[
		"N3-namespace-ambient-registered",
		"index.cjs",
		" ||\n\t\t\t\tnode.flags & ts.NodeFlags.Ambient",
		"",
		"ambient classes emit",
	],
	[
		"N10-enclosing-dropped",
		"index.cjs",
		"checker.typeToTypeNode(checker.getTypeAtLocation(field), field,",
		"checker.typeToTypeNode(checker.getTypeAtLocation(field), undefined,",
		"rbxtsc inferred attribute aliases resolve outside the emitting file scope",
	],

	[
		"cru005-constructor-type",
		"index.cjs",
		"checker.getDeclaredTypeOfSymbol(symbol) !== (target.target ?? target)",
		"false",
		"T2 exact-line",
	],
	[
		"cru012-no-bases",
		"index.cjs",
		"resolvedBases(type).some((base) => promise(base, seen))",
		"false",
		"T2 exact-line",
	],
	[
		"cru014-no-inherited-implements",
		"index.cjs",
		"for (const target of clause.types)\n\t\t\t\t\t\t\tif (genericTarget(checker.getTypeAtLocation(target), mode, seen)) return true;",
		"for (const target of []) return true;",
		"T2 exact-line",
	],
	[
		"cru005-declared-type-filter",
		"index.cjs",
		"!symbol?.declarations?.some(\n\t\t\t\t\t\t(decl) => ts.isClassDeclaration(decl) || ts.isInterfaceDeclaration(decl),\n\t\t\t\t\t) ||\n\t\t\t\t\tchecker.getDeclaredTypeOfSymbol(symbol) !== (target.target ?? target)",
		"!symbol?.declarations?.length || (symbol.declarations.some((decl) => ts.isClassDeclaration(decl) || ts.isInterfaceDeclaration(decl)) && checker.getDeclaredTypeOfSymbol(symbol) !== (target.target ?? target))",
		"T2 exact-line",
	],
	[
		"cru005-typeof-strict",
		"index.cjs",
		"ts.isClassDeclaration(decl) || ts.isInterfaceDeclaration(decl),",
		"ts.isClassDeclaration(decl) || ts.isInterfaceDeclaration(decl) || true,",
		"T2 exact-line",
	],
	[
		"side-source-root-anchor",
		"index.cjs",
		"const sideRoot = packageOf(source.fileName)?.dir ?? path.parse(path.resolve(source.fileName)).root;",
		"const sideRoot = sourceRoot(program.getCompilerOptions(), [program.getCommonSourceDirectory()]);",
		"T2 folder sides preserve side-named source roots",
	],
	...[
		[
			"v4-named-package-only",
			"((owner) => (owner?.json.name ? owner.dir : undefined))(packageOf(source.fileName)) ?? path.parse(path.resolve(source.fileName)).root",
			"T2 package sides exclude a side-named package root",
		],
		[
			"side-tsconfig-anchor",
			"path.dirname(program.getCompilerOptions().configFilePath ?? path.join(program.getCurrentDirectory(), 'tsconfig.json'))",
			"T2 package sides layouts",
		],
		[
			"side-parent-source-anchor",
			"path.dirname(sourceRoot(program.getCompilerOptions(), [program.getCommonSourceDirectory()]))",
			"T2 package sides layouts",
		],
		[
			"side-above-package-anchor",
			"path.dirname(packageOf(source.fileName)?.dir ?? path.parse(path.resolve(source.fileName)).root)",
			"T2 package sides exclude",
		],
		[
			"side-no-package-source-anchor",
			"packageOf(source.fileName)?.dir ?? program.getCommonSourceDirectory()",
			"T2 package sides without",
		],
	].map(([name, root, test]) => [
		name,
		"index.cjs",
		"const sideRoot = packageOf(source.fileName)?.dir ?? path.parse(path.resolve(source.fileName)).root;",
		`const sideRoot = ${root};`,
		test,
	]),

	["hooks-include-public", "index.cjs", "modifiers(member, ts.ModifierFlags.Protected) &&", "", "T2 exact-line"],
	[
		"side-absolute-path",
		"index.cjs",
		"path\n\t\t\t\t.relative(sideRoot, node.getSourceFile().fileName)",
		"node.getSourceFile().fileName",
		"T2 folder sides ignore directories above the common source root",
	],

	...["CRU001", "CRU002", "CRU003", "CRU006", "CRU007", "CRU008", "CRU010", "CRU015"].map((code) => [
		code,
		"diagnostics.cjs",
		"addDiagnostic({",
		`code === "${code}" ? undefined : addDiagnostic({`,
		"diagnostic negative probes",
	]),
];

for (const code of ["CRU004", "CRU005", "CRU009", "CRU011", "CRU012", "CRU013", "CRU014", "CRU016"]) {
	mutants.push([
		code,
		"diagnostics.cjs",
		"addDiagnostic({",
		`code === "${code}" ? undefined : addDiagnostic({`,
		code === "CRU016" ? "T2 folder sides" : "T2 exact-line",
	]);
}
const receipts = [];
for (const [name, file, before, after, test] of mutants) {
	const root = path.join(scratch, name);
	const copy = path.join(root, "transformer-src");
	fs.mkdirSync(copy, { recursive: true });
	for (const entry of fs.readdirSync(source)) {
		if (entry === "build") continue;
		fs.cpSync(path.join(source, entry), path.join(copy, entry), { recursive: true });
	}
	const target = path.join(copy, file);
	const original = fs.readFileSync(target, "utf8");
	assert.ok(original.includes(before), `${name} mutation matches the source`);
	fs.writeFileSync(target, original.replace(before, after));
	const built = spawnSync(process.execPath, [path.join(copy, "build.mjs")], { encoding: "utf8" });
	if (built.status !== 0) fs.writeFileSync(target, original);
	assert.equal(built.status, 0, built.stderr);
	const result = spawnSync(
		process.execPath,
		["--test", `--test-name-pattern=${test}`, path.join(copy, "tests/transformer.test.cjs")],
		{ encoding: "utf8", env: { ...process.env, CRUCIBLE_TEST_REPO: repo } },
	);
	fs.writeFileSync(path.join(root, "result.txt"), result.stdout + result.stderr);
	fs.writeFileSync(target, original);
	assert.equal(result.status, 1, `${name} must be killed by ${test}: ${result.stdout}${result.stderr}`);
	assert.match(result.stdout, new RegExp(`✖ ${test}`));
	receipts.push({ name, test, exitCode: result.status, restored: fs.readFileSync(target, "utf8") === original });
	console.log(`Killed ${name} with ${test}; scratch source restored.`);
}
fs.writeFileSync(path.join(scratch, "receipts.json"), JSON.stringify(receipts, null, "\t") + "\n");
