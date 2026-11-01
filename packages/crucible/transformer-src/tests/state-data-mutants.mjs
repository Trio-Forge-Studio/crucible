import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const source = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(source, "../../..");
const scratch = path.join(repo, "build/state-data-mutants");
const R = ["replicated rewrites", "replicated.test.cjs"];
const RS = ["replicated state lowering", "replicated.test.cjs"];
const RD = ["replicated depth", "replicated.test.cjs"];
const RC = ["replicated declarations take", "replicated.test.cjs"];
const RG = ["replicated declarations report CRU010", "replicated.test.cjs"];
const RK = ["K9 the build rejects a replicated", "replicated.test.cjs"];
const P1 = ["P1 data", "data.test.cjs"];
const DD = ["data declarations take", "data.test.cjs"];
const C27 = ["CRU027 checks", "data.test.cjs"];
const C25 = ["CRU025 rejects", "data.test.cjs"];
const CF = ["configs rewrite", "data.test.cjs"];
const DK = ["K9 the build rejects a profileStore", "data.test.cjs"];
const LP = ["transformer limits equal", "data.test.cjs"];
const PM = ["persist rejections", "data.test.cjs"];
const PD = ["persist depth", "data.test.cjs"];
const N = "network.cjs";
const D = "declarations.cjs";
const C = "data.cjs";
const E = "tests/fixtures/state-data/emit.d.ts";
// [name, before, after, [test, suite], file]
const mutations = [
	["rule 40 shared side guard", "side(node) !== spec.side", "false", RC, N],
	[
		"rule 1 profile store side swap",
		'label: "profile stores",\n\t\tside: "shared",',
		'label: "profile stores",\n\t\tside: "server",',
		DD,
		D,
	],
	[
		"rule 1 optional type argument short-circuit",
		'spec.typeArguments === "optional" ? typeArguments > 1 : typeArguments !== spec.typeArguments',
		"typeArguments !== spec.typeArguments",
		P1,
		N,
	],
	["rule 1 extra argument guard", "if (node.arguments.length > spec.arguments)", "if (false)", DD, N],
	[
		"rule 78 rewrite drops the initial value",
		"...spec.passed.map((i) => node.arguments[i] && visitInput(node.arguments[i])).filter(Boolean),",
		"",
		R,
		N,
	],
	[
		"rule 78 K9 rewrite drops the type argument",
		"return f.createCallExpression(runtime(kind), node.typeArguments, [",
		"return f.createCallExpression(runtime(kind), undefined, [",
		RK,
		N,
	],
	[
		"rule 112 K9 rewrite drops the type argument",
		"return f.createCallExpression(runtime(kind), node.typeArguments, [",
		"return f.createCallExpression(runtime(kind), undefined, [",
		DK,
		N,
	],
	["rule 41 separate replicated index", "kinds[d.kind].names === spec.names", "true", RC, N],
	["rule 41 replicated index swap", 'names: "Replicated",', 'names: "Message",', RC, D],
	["rule 41 duplicate guard", "if (other) {", "if (false && other) {", RC, N],
	["rule 2 name length guard", "Buffer.byteLength(name.text) > spec.maxBytes", "false", DD, N],
	[
		"rule 2 store byte limit swap",
		'maxBytes: dataLimits.STORE_NAME_MAX_BYTES,\n\t\ttypeArguments: "optional",',
		'maxBytes: dataLimits.CONFIG_NAME_MAX_BYTES,\n\t\ttypeArguments: "optional",',
		DD,
		D,
	],
	["rule 2 config index split", 'names: "Config",', 'names: "Profile message",', DD, D],
	[
		"rule 134 view name check",
		"of viewNameProblems(node.arguments[0]?.text, view))",
		'of viewNameProblems(node.arguments[0]?.text, { kind: "nil" }))',
		P1,
		N,
	],
	[
		"rule 134 view names judged on the replica",
		"viewNameProblems(node.arguments[0]?.text, view)",
		"viewNameProblems(node.arguments[0]?.text, result[0])",
		P1,
		N,
	],
	[
		"rule 134 view prefix swap",
		"const prefix = `${dataLimits.VIEW_NAME_PREFIX}${store}.`;",
		"const prefix = `${dataLimits.LIVE_NAME_PREFIX}${store}.`;",
		DD,
		D,
	],
	["rule 134 view name without its prefix", "const name = prefix + field;", "const name = field;", DD, D],
	["rule 42 Instance guard", "if (state)\n\t\t\t\treturn fail(", "if (false)\n\t\t\t\treturn fail(", RS, N],
	[
		"rule 42 Instance message swap",
		'? "Instances cannot be saved or replicated; store a stable id"\n\t\t\t\t\t\t: "replicated state cannot hold Instances; replicate a stable id",',
		'? "replicated state cannot hold Instances; replicate a stable id"\n\t\t\t\t\t\t: "Instances cannot be saved or replicated; store a stable id",',
		RS,
		N,
	],
	["rule 42 unknown guard", "return state\n\t\t\t\t\t\t? fail(", "return false\n\t\t\t\t\t\t? fail(", RS, N],
	[
		"rule 42 string key guard",
		"if (state && !(type.isUnion() ? type.types : [type]).every((t) => t.isStringLiteral()))",
		"if (false)",
		RC,
		N,
	],
	[
		"rule 42 absent map values",
		"if (state && value) t = checker.getNonNullableType(t);",
		"if (false) t = checker.getNonNullableType(t);",
		RS,
		N,
	],
	["rule 42 flat intersection", "const flat = state && plainIntersection(type);", "const flat = false;", RS, N],
	[
		"rule 42 intersections as variant objects",
		"(part.flags & flags.Object || (state && plainIntersection(part))) &&",
		"part.flags & flags.Object &&",
		PM,
		N,
	],
	["rule 42 depth limit swap", "state: reactiveLimits.SYNC_VALUE_DEPTH,", "state: limits.MAX_DEPTH,", RD, D],
	["rule 42 depth guard", "if (depth > depthLimit) return fail(", "if (false) return fail(", RD, N],
	["rule 1 empty array guard", 'if (c === "array" && state && args[0].flags & flags.Never)', "if (false)", PM, N],
	["rule 7 datatype guard", "if (e && persist)", "if (false)", P1, N],
	[
		"rule 7 store root guard",
		'if (spec.store && value.kind !== "object" && value.kind !== "map")',
		"if (false)",
		P1,
		N,
	],
	[
		"rule 7 store root map swap",
		'value.kind !== "object" && value.kind !== "map"',
		'value.kind !== "object" && value.kind !== "array"',
		P1,
		N,
	],
	["rule 7 persist depth swap", "persist: dataLimits.PERSIST_DEPTH,", "persist: limits.MAX_DEPTH,", PD, D],
	[
		"rule 129 profile-store-only guard",
		"if (!spec.handles)\n\t\t\t\t\treturn fail(",
		"if (false)\n\t\t\t\t\treturn fail(",
		PM,
		N,
	],
	["rule 129 nested mark guard", "if (position.marked) return fail(", "if (false) return fail(", PM, N],
	["rule 129 object path guard", "if (!position.path)", "if (false)", PM, N],
	[
		"rule 129 path and mark swap",
		"path: position.path,\n\t\t\t\t\t\t\t\tmarked: position.marked,",
		"path: position.marked,\n\t\t\t\t\t\t\t\tmarked: position.path,",
		P1,
		N,
	],
	[
		"rule 129 union leaves the object path",
		"nilable: position.nilable || missing, path: false };",
		"nilable: position.nilable || missing };",
		DD,
		N,
	],
	[
		"rule 129 optional field keeps its mark",
		"serverOnly: true,\n\t\t\t\t\t\t\t}",
		"serverOnly: false,\n\t\t\t\t\t\t\t}",
		PM,
		N,
	],
	["rule 129 mark recorded", "return { ...inner, serverOnly: true };", "return inner;", P1, N],
	["rule 129 replica drops marked fields", "if (child.serverOnly) continue;", "", P1, D],
	["rule 129 no empty replicas", 'if (kept.kind === "object" && !kept.fields.length) continue;', "", P1, D],
	["rule 129 path segment order", "[[...prefix, key]]", "[[key, ...prefix]]", P1, D],
	["rule 112 persist and replica order", "return [result[0], view];", "return [view, result[0]];", P1, N],
	[
		"rule 112 paths read from the persist descriptor",
		"serverOnlyPaths(descriptors[0])",
		"serverOnlyPaths(descriptors[1])",
		P1,
		N,
	],
	[
		"rule 130 reserved name guard",
		"if (spec.handles && dataLimits.RESERVED_HANDLE_NAMES.includes(prop.name))",
		"if (false)",
		P1,
		N,
	],
	[
		"rule 130 reserved names only in profile stores",
		"if (spec.handles && dataLimits.RESERVED_HANDLE_NAMES.includes(prop.name))",
		"if (state && dataLimits.RESERVED_HANDLE_NAMES.includes(prop.name))",
		PM,
		N,
	],
	[
		"rule 113 grant gate",
		"&& marketplace(target.expression) && grants())",
		"&& marketplace(target.expression))",
		C25,
		C,
	],
	["rule 113 MarketplaceService guard", " && marketplace(target.expression)", "", C25, C],
	["rule 113 grant member", 'storeCall(node)?.member === "grant"', 'storeCall(node)?.member === "receive"', C25, C],
	["rule 113 parenthesized target", "const target = unwrap(node.left);", "const target = node.left;", C25, C],
	["rule 132 side guard", "if (side(node) !== required)", "if (false)", C27, C],
	[
		"rule 132 side swap",
		'const required = client ? "client" : "server";',
		'const required = client ? "server" : "client";',
		C27,
		C,
	],
	[
		"rule 132 loaded() pairs with the client",
		'call.member === undefined || call.member === "loaded"',
		"call.member === undefined",
		C27,
		C,
	],
	[
		"rule 132 element access member",
		"if (ts.isElementAccessExpression(expression) && ts.isStringLiteralLike(expression.argumentExpression))",
		"if (false)",
		C27,
		C,
	],
	[
		"rule 132 parenthesized callee",
		"const callee = unwrap(node.expression);",
		"const callee = node.expression;",
		C27,
		C,
	],
	["configs case 12 export check", "const owner = exportSide(node, symbol);", "const owner = undefined;", CF, N],
	[
		"configs case 12 value type lookup",
		"return entryExports.get(unalias(type.aliasSymbol)) ?? entryExports.get(type.getSymbol());",
		"return undefined;",
		CF,
		N,
	],
	[
		"configs case 12 side swap",
		"entryExports.set(unalias(exported), s);",
		'entryExports.set(unalias(exported), s === "server" ? "client" : "server");',
		CF,
		N,
	],
	[
		"rule 5 reverse scan covers every declaration kind",
		"const kind = declarationMacros.find((name) => isMacro(node, name));\n\t\t\t\tif (kind)",
		"const kind = declarationMacros.slice(0, 3).find((name) => isMacro(node, name));\n\t\t\t\tif (kind)",
		RG,
		N,
	],
	["limits parity SYNC_VALUE_DEPTH", "SYNC_VALUE_DEPTH: 28,", "SYNC_VALUE_DEPTH: 29,", LP, D],
	[
		"limits parity reserved names",
		'RESERVED_HANDLE_NAMES: Object.freeze(["set", "update", "insert", "remove"]),',
		'RESERVED_HANDLE_NAMES: Object.freeze(["set", "update", "insert"]),',
		LP,
		D,
	],
	[
		"K9 emit replicated takes any initial value",
		"initial: NoInfer<T>, value: Node): Replicated<T>;",
		"initial: unknown, value: Node): Replicated<T>;",
		RK,
		E,
	],
	["K9 emit profileStore takes any options", "options: ProfileOptions<T>,", "options: unknown,", DK, E],
];
if (process.argv.includes("--list")) {
	console.log(JSON.stringify(mutations.map(([name]) => name)));
	process.exit(0);
}
for (const [name, before, , , file] of mutations)
	assert.ok(fs.readFileSync(path.join(source, file), "utf8").includes(before), `${name} matches source`);
if (process.argv.includes("--validate")) process.exit(0);
const from = Number(process.argv[process.argv.indexOf("--from") + 1]) || 0;
if (!from) fs.rmSync(scratch, { recursive: true, force: true });
const receipts = [];
for (let index = from; index < mutations.length; index++) {
	const [name, before, after, [test, suite], file] = mutations[index];
	const root = path.join(scratch, String(index));
	const copy = path.join(root, "transformer-src");
	fs.rmSync(root, { recursive: true, force: true });
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
		{ encoding: "utf8", env: { ...process.env, CRUCIBLE_TEST_REPO: repo }, timeout: 300000 },
	);
	fs.writeFileSync(path.join(root, "result.log"), result.stdout + result.stderr);
	const killed = result.status === 1 && result.stdout.includes(`✖ ${test}`);
	receipts.push({ index, name, test, killed, exitCode: result.status });
	console.log(`${killed ? "KILLED" : "SURVIVED"} ${index} ${name}`);
	if (!killed) {
		fs.writeFileSync(path.join(scratch, "receipts.json"), JSON.stringify(receipts, null, 2));
		throw new Error(`${name} survived. See ${root}`);
	}
	fs.rmSync(root, { recursive: true, force: true });
}
fs.writeFileSync(path.join(scratch, "receipts.json"), JSON.stringify(receipts, null, 2));
console.log(`${receipts.length}/${mutations.length - from} killed.`);
