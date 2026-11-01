import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const source = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(source, "../../..");
const scratch = path.join(repo, "build/network-mutants");
const descriptors = "network descriptors recheck";
const negative = "network diagnostic catalogue";
const receivers = "network receives rewrite";
const globals = "network duplicate names";
const sides = "network side entry references";
const limits = "network schema boundaries";
const augmentations = "network augmentations";
const mutations = [
	[
		"parenthesized macro callee",
		"while (ts.isParenthesizedExpression(expression.parent)) expression = expression.parent;",
		"",
		"network declaration and receiver aliases",
	],
	[
		"rule 63 typed kind argument swap",
		'Buffer.from(a.runtime ?? ""), Buffer.from(b.runtime ?? "")',
		'Buffer.from(b.runtime ?? ""), Buffer.from(a.runtime ?? "")',
		descriptors,
	],
	[
		"rule 61 variant property name guard",
		'ts.isNumericLiteralName(discriminant) || discriminant.startsWith("__@")',
		"false",
		negative,
	],
	[
		"CRU010 discriminant reference",
		"references((prop.valueDeclaration ?? prop.declarations?.[0])?.type, `${location}.${name}`);",
		"",
		globals,
	],
	["rule 1 const guard", "!(list.flags & ts.NodeFlags.Const)", "false", negative],
	["rule 1 export guard", "!modifiers(statement, ts.ModifierFlags.Export)", "false", negative],
	["rule 1 side guard", 'side(node) !== "shared"', "false", negative],
	[
		"rule 2 absent descriptor",
		'if (type.flags & flags.Undefined) return { kind: "nil" };',
		'if (type.flags & flags.Undefined) return { kind: "dynamic" };',
		descriptors,
	],
	["rule 3 reserved prefix guard", 'name.text.split(".")[0] === "Crucible"', "false", negative],
	["rule 3 length guard", "Buffer.byteLength(name.text) > limits.MESSAGE_NAME_MAX_BYTES", "false", negative],
	["rule 4 duplicate guard", "if (other) {", "if (false && other) {", globals],
	[
		"rule 5 timeout bound",
		"Number(prop.initializer.text) <= limits.MAX_ASK_TIMEOUT_SECONDS",
		"Number(prop.initializer.text) <= 121",
		negative,
	],
	[
		"rule 5 timeout default",
		'let timeout = f.createIdentifier("undefined");',
		"let timeout = f.createNumericLiteral(10);",
		descriptors,
	],
	["rule 16 side guard", "if (s && s !== side(source))", "if (false && s && s !== side(source))", sides],
	[
		"rule 16 side swap",
		"entryFiles.set(canonical(resolved.resolvedFileName), s)",
		'entryFiles.set(canonical(resolved.resolvedFileName), s === "server" ? "client" : "server")',
		sides,
	],
	["rule 23 readonly guard", "!modifiers(field, ts.ModifierFlags.Readonly)", "false", receivers],
	["rule 23 static guard", "modifiers(field, ts.ModifierFlags.Static)", "false", receivers],
	["rule 23 subsystem guard", 'kindOf(instanceType(field.parent)) !== "subsystem"', "false", receivers],
	[
		"rule 23 inherited receiver guard",
		"if ((instanceType(field.parent).getBaseTypes?.() ?? []).some(shadow))",
		"if (false && (instanceType(field.parent).getBaseTypes?.() ?? []).some(shadow))",
		receivers,
	],
	["rule 24 target side guard", "if (side(field) !== required)", "if (false && side(field) !== required)", receivers],
	[
		"rule 24 side swap",
		'kind === "ToClient" ? "client" : "server"',
		'kind === "ToClient" ? "server" : "client"',
		receivers,
	],
	["rule 25 callback resolution", "const fn = callback(node.arguments[1]);", "const fn = undefined;", receivers],
	["rule 25 own return check", "checker.getReturnTypeOfSignature(signature)", "checker.getVoidType()", receivers],
	["rule 25 async and return paired check", "if (signature) {", "if (false && signature) {", receivers],
	["rule 58 width identity", "if (w)\n", "if (false && w)\n", descriptors],
	[
		"rule 59 string node",
		'if (type.flags & flags.String) return { kind: "string" };',
		'if (type.flags & flags.String) return { kind: "dynamic" };',
		descriptors,
	],
	[
		"rule 60 datatype swap",
		"kind: name.toLowerCase(), runtime: name",
		'kind: name === "Vector3" ? "color3" : name.toLowerCase(), runtime: name',
		limits,
	],
	["rule 61 empty guard", "!props.length && omit === undefined", "false", negative],
	[
		"rule 61 numeric name guard",
		"if (ts.isNumericLiteralName(prop.name))",
		"if (false && ts.isNumericLiteralName(prop.name))",
		negative,
	],
	[
		"rule 61 field order",
		".sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)))",
		".sort((a, b) => Buffer.compare(Buffer.from(b.name), Buffer.from(a.name)))",
		descriptors,
	],
	[
		"rule 62 any guard",
		'if (type.flags & flags.Any) return fail(node, location, "any cannot be checked; use unknown");',
		'if (type.flags & flags.Any) return { kind: "dynamic" };',
		negative,
	],
	["rule 62 recursive reset", "active.delete(type);", "", limits],
	[
		"rule 63 literal sort",
		"literals.sort((a, b) => compare(a.value, b.value))",
		"literals.sort((a, b) => compare(b.value, a.value))",
		descriptors,
	],
	[
		"rule 64 variant body pair",
		"body: fields(objects[i], location, position, depth + 2, discriminant)",
		'body: { kind: "object", fields: [] }',
		descriptors,
	],
	[
		"rule 64 discriminant guard",
		"if (discriminant === undefined)",
		"if (false && discriminant === undefined)",
		negative,
	],
	["rule 65 ambiguity guard", "new Set(typed.map((t) => t.runtime)).size !== typed.length", "false", negative],
	["rule 65 branch budget", "branches.length > limits.MAX_UNION_BRANCHES", "false", limits],
	["rule 66 nil element guard", "if (absent(t))", "if (false && absent(t))", negative],
	[
		"rule 66 map key value swap",
		"key: key(args[0], `${location}.key`)",
		"key: key(args[1], `${location}.key`)",
		descriptors,
	],
	[
		"rule 66 tuple order swap",
		"args.map((t, i) => element(t, `${location}[${i}]`))",
		"[...args].reverse().map((t, i) => element(t, `${location}[${i}]`))",
		descriptors,
	],
	["rule 67 instance class", "classes: sorted(classes)", 'classes: ["Player"]', descriptors],
	[
		"rule 68 streaming guard",
		"position.toClient && !position.nilable",
		"false && position.toClient && !position.nilable",
		negative,
	],
	[
		"rule 68 request response flag swap",
		'toClient: kind === "toClient" || i === 1',
		'toClient: kind === "toClient" || i === 0',
		negative,
	],
	["rule 70 schema node budget", "> limits.MAX_SCHEMA_NODES)", "> Infinity)", limits],
	["rule 70 depth budget", "depth > limits.MAX_DEPTH", "false", limits],
	[
		"CRU010 global and merge guards",
		"decls.some(global) || new Set(decls.map((d) => d.getSourceFile().fileName)).size > 1",
		"false",
		globals,
	],
	["CRU010 augmentation paired scan", "for (const declaration of index())", "for (const declaration of [])", globals],
	["CRU023 escape guard", "!ts.isImportSpecifier(p) &&", "false &&", negative],
	[
		"ask request response argument swap",
		"...lowerMessage(node, kind, report).map(emit)",
		"...lowerMessage(node, kind, report).reverse().map(emit)",
		descriptors,
	],
	[
		"rule 6 literal inference",
		"PayloadArgs<NoInfer<P>>",
		"PayloadArgs<P>",
		"K1 K16 R5",
		"tests/fixtures/network/net.d.ts",
		"network-types.test.cjs",
	],
	[
		"rule 6 covariance",
		"readonly payload: (value: P) => P",
		"readonly payload: () => P",
		"network token types",
		"tests/fixtures/network/index.d.ts",
		"network-types.test.cjs",
	],
	[
		"rule 27 callable receiver",
		"export interface Receiver {",
		"export interface Receiver {\n(): void;",
		"network token types",
		"tests/fixtures/network/index.d.ts",
		"network-types.test.cjs",
	],
	[
		"rule 31 sendable ready",
		'readonly [message]: { readonly kind: "client-ready" };',
		'readonly [message]: { readonly kind: "to-server"; readonly payload: (value: undefined) => undefined };',
		"network token types",
		"tests/fixtures/network/index.d.ts",
		"network-types.test.cjs",
	],
	[
		"K9 emit receive widened to an untyped target and callback",
		"export declare function receive<T extends ReceiveTarget>(\n\towner: object,\n\tfield: string,\n\ttarget: T,\n\tcallback: ReceiveCallback<T>,\n): Receiver;",
		"export declare function receive(owner: object, field: string, target: unknown, callback: Callback): Receiver;",
		"K2 K9",
		"tests/fixtures/network/emit.d.ts",
		"network-types.test.cjs",
	],
	[
		"K9 rewrite drops the target",
		"visitInput(node.arguments[0]),\n\t\t\t\t\t\tvisitInput(node.arguments[1]),",
		"visitInput(node.arguments[1]),",
		"K2 K9",
		"network.cjs",
		"network-types.test.cjs",
	],
	[
		"K9 rewrite target callback swap",
		"visitInput(node.arguments[0]),\n\t\t\t\t\t\tvisitInput(node.arguments[1]),",
		"visitInput(node.arguments[1]),\n\t\t\t\t\t\tvisitInput(node.arguments[0]),",
		"K2 K9",
		"network.cjs",
		"network-types.test.cjs",
	],
	[
		"K9 rewrite erases the target type",
		"visitInput(node.arguments[0]),\n\t\t\t\t\t\tvisitInput(node.arguments[1]),",
		"f.createAsExpression(visitInput(node.arguments[0]), f.createKeywordTypeNode(ts.SyntaxKind.AnyKeyword)),\n\t\t\t\t\t\tvisitInput(node.arguments[1]),",
		"K2 K9",
		"network.cjs",
		"network-types.test.cjs",
	],
	[
		"K9 rewrite drops explicit type arguments",
		'runtime("receive"), node.typeArguments,',
		'runtime("receive"), undefined,',
		"K2 K9",
		"network.cjs",
		"network-types.test.cjs",
	],
	[
		"width identity package resolution",
		'packageOf(sf.fileName)?.json.name !== "@trioforge/crucible"',
		"!/[\\\\/]@trioforge[\\\\/]crucible[\\\\/]/.test(sf.fileName)",
		"network width identity",
	],
	[
		"M01 rule 64 per-candidate literal reset hoisted",
		"for (const name of candidates) {\n\t\t\t\t\tconst seen = new Set();",
		"const seen = new Set();\n\t\t\t\tfor (const name of candidates) {",
		descriptors,
	],
	[
		"M02 rule 64 required discriminant guard",
		"if (!prop || prop.flags & ts.SymbolFlags.Optional) return;",
		"if (!prop) return;",
		negative,
	],
	[
		"M03 rule 64 disjoint literal guard",
		"if (list.some((v) => v === undefined || seen.has(`${typeof v}:${v}`))) return;",
		"if (list.some((v) => v === undefined)) return;",
		negative,
	],
	[
		"M04 rule 63 variant sort argument swap",
		"...variants.sort((a, b) => compare(a.value, b.value)),",
		"...variants.sort((a, b) => compare(b.value, a.value)),",
		descriptors,
	],
	[
		"M05 rule 65 table beside variants paired check",
		'(variants.length && typed.some((t) => t.runtime === "table"))',
		"false",
		negative,
	],
	[
		"M06 CRU012 cleanup allowance limited to ClientReady",
		'!(kind === "ClientReady" && t.getCallSignatures().length)',
		"!t.getCallSignatures().length",
		receivers,
	],
	[
		"M07 CRU012 never member",
		"!(t.flags & (flags.Void | flags.Undefined | flags.Never))",
		"!(t.flags & (flags.Void | flags.Undefined))",
		receivers,
	],
	["M08 CRU012 const binding guard", "ts.getCombinedNodeFlags(decl) & ts.NodeFlags.Const &&", "", receivers],
	[
		"M09 rule 1 extra argument guard",
		'if (node.arguments.length > (kind === "askServer" ? 2 : 1))',
		"if (false)",
		negative,
	],
	[
		"M10 rule 63 boolean fold needs both literals",
		"parts.filter((p) => p.flags & flags.BooleanLiteral).length === 2",
		"parts.filter((p) => p.flags & flags.BooleanLiteral).length > 0",
		descriptors,
	],
	[
		"M11 CRU010 declare global reverse scan",
		"(node.flags & ts.NodeFlags.GlobalAugmentation || ts.isStringLiteral(node.name))",
		"ts.isStringLiteral(node.name)",
		globals,
	],
	[
		"M12 rule 66 nonfinite key literal guard",
		'(typeof literal(t) !== "number" || Number.isFinite(literal(t)))',
		"true",
		negative,
	],
	[
		"M13 CRU012 Promise member paired check",
		'(t) => t.getSymbol()?.name === "Promise" && standard(t.getSymbol(), "compiler-types"),',
		"(t) => false,",
		receivers,
	],
	["M14 rule 66 key literal sort argument swap", ".sort(compare)\n", ".sort((a, b) => compare(b, a))\n", descriptors],
	["M15 rule 58 width single property guard", "if (props.length !== 1) return;", "", negative],
	[
		"M16 rule 60 engine package guard",
		'if (!standard(symbol, "types")) return;',
		"if (!symbol) return;",
		descriptors,
	],
	[
		"M17 rule 66 tuple optional flag",
		"(ts.ElementFlags.Optional | ts.ElementFlags.Rest | ts.ElementFlags.Variadic)",
		"(ts.ElementFlags.Rest | ts.ElementFlags.Variadic)",
		negative,
	],
	[
		"M18 rule 68 collection toClient propagation",
		"return lower(t, location, { toClient: position.toClient, nilable: false }, depth + 1);",
		"return lower(t, location, { toClient: false, nilable: false }, depth + 1);",
		negative,
	],
	[
		"M19 CRU024 import type clause",
		"(ts.isLiteralTypeNode(node.parent) && ts.isImportTypeNode(node.parent.parent)) ||",
		"",
		sides,
	],
	["M20 CRU019 field path", "`${location}.${prop.name}`,\n\t\t\t\t\t\t\t{", "location,\n\t\t\t\t\t\t\t{", negative],
	[
		"M21 rule 5 option key name guard",
		'prop.name.getText().replaceAll(\'"\', "").replaceAll("\'", "") !== "timeout" ||',
		"",
		negative,
	],
	[
		"M22 CRU021 shadow guard limited to receivers",
		'(d) => ts.isPropertyDeclaration(d) && d.initializer && isMacro(d.initializer, "receive"),',
		"(d) => ts.isPropertyDeclaration(d),",
		receivers,
	],
	[
		"M24 rule 61 numeric name path",
		'fail(node, `${location}.${prop.name}`, "numeric property names are not supported; use a Map");',
		'fail(node, location, "numeric property names are not supported; use a Map");',
		negative,
	],
	[
		"M25 CRU018 duplicate across kinds",
		"d.node !== node && d.name === name.text",
		"d.name === name.text && d.node !== node && d.kind === kind",
		globals,
	],
	[
		"M26 CRU012 own signature",
		"const signature = fn && checker.getSignatureFromDeclaration(fn);",
		"const signature = fn && checker.getSignaturesOfType(checker.getTypeAtLocation(node.arguments[1]), ts.SignatureKind.Call)[0];",
		receivers,
	],
	[
		"M27 CRU024 resolved symbol walk",
		"for (const decl of symbol?.declarations ?? []) wrongSide(node, decl.getSourceFile().fileName);",
		"",
		sides,
	],
	[
		"CRU010 reverse scan nested namespace walk",
		"...(ts.isModuleDeclaration(statement) ? added(statement.body) : [])",
		"",
		globals,
	],
	["CRU010 reverse scan bodiless module guard", "!body ? [] : ", "", globals],
	[
		"CRU010 reverse scan added declarations only",
		"added(node.body).some((d) => reached.has(checker.getSymbolAtLocation(d.name)))",
		"true",
		augmentations,
	],
	[
		"rule 4 project relative duplicate path",
		'path.relative(root, sf.fileName).replaceAll("\\\\", "/")',
		"sf.fileName",
		globals,
	],
	["K4 undefined field guard", "if (value.flags & flags.Undefined)", "if (false)", negative],
	[
		"K4 undefined field path",
		'`${location}.${prop.name}`,\n\t\t\t\t\t\t\t\t"a field of type undefined',
		'location,\n\t\t\t\t\t\t\t\t"a field of type undefined',
		negative,
	],
	[
		"rule 1 type argument count swap",
		'node.typeArguments?.length !== (kind === "askServer" ? 2 : 1)',
		'node.typeArguments?.length !== (kind === "askServer" ? 1 : 2)',
		descriptors,
	],
];
if (process.argv.includes("--list")) {
	console.log(JSON.stringify(mutations.map(([name]) => name)));
	process.exit(0);
}
for (const [name, before, , , file = "network.cjs"] of mutations)
	assert.ok(fs.readFileSync(path.join(source, file), "utf8").includes(before), `${name} matches source`);
if (process.argv.includes("--validate")) process.exit(0);
fs.rmSync(scratch, { recursive: true, force: true });
const receipts = [];
for (const [name, before, after, test, file = "network.cjs", suite = "network.test.cjs"] of mutations) {
	const root = path.join(scratch, String(receipts.length));
	const copy = path.join(root, "transformer-src");
	fs.mkdirSync(root, { recursive: true });
	for (const entry of ["out", "server", "client", "emit", "package.json", "transformer"])
		fs.cpSync(path.join(source, "..", entry), path.join(root, entry), { recursive: true });
	fs.cpSync(source, copy, { recursive: true });
	const target = path.join(copy, file);
	const original = fs.readFileSync(target, "utf8");
	assert.ok(original.includes(before), `${name} matches source`);
	fs.writeFileSync(target, original.replaceAll(before, after));
	const built = spawnSync(process.execPath, [path.join(copy, "build.mjs")], { encoding: "utf8" });
	assert.equal(built.status, 0, built.stderr);
	const result = spawnSync(
		process.execPath,
		["--test", `--test-name-pattern=${test}`, path.join(copy, "tests", suite)],
		{ encoding: "utf8", env: { ...process.env, CRUCIBLE_TEST_REPO: repo }, timeout: 180000 },
	);
	fs.writeFileSync(path.join(root, "result.log"), result.stdout + result.stderr);
	fs.writeFileSync(target, original);
	const killed = result.status === 1 && result.stdout.includes(`✖ ${test}`);
	receipts.push({
		name,
		test,
		killed,
		exitCode: result.status,
		restored: fs.readFileSync(target, "utf8") === original,
	});
	console.log(`${killed ? "KILLED" : "SURVIVED"} ${name}`);
	if (!killed) {
		fs.writeFileSync(path.join(scratch, "receipts.json"), JSON.stringify(receipts, null, 2));
		throw new Error(`${name} survived. See ${root}`);
	}
}
fs.writeFileSync(path.join(scratch, "receipts.json"), JSON.stringify(receipts, null, 2));
console.log(`${receipts.length}/${mutations.length} killed; scratch sources restored.`);
