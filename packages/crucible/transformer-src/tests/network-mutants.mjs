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
const mutations = [
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
		"width identity package resolution",
		'packageOf(sf.fileName)?.json.name !== "@trioforge/crucible"',
		"!/[\\\\/]@trioforge[\\\\/]crucible[\\\\/]/.test(sf.fileName)",
		"network width identity",
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
