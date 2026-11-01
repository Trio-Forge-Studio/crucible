import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const C = "packages/crucible/src/internal/network/codec.ts";
const S = "packages/crucible/src/internal/network/schema.ts";
const K = "packages/crucible/src/internal/network/catalog.ts";
const L = "packages/crucible/src/internal/network/limits.ts";
const specs = ["network-codec", "network-catalog", "network-fd", "constructs and compiles"];

// Ruling K3: each rule carries a guard, swap and pair operator, or says why the code has no such site.
const rules = [
	[
		"3",
		["guard", K, 'assert(segments[0] !== "Crucible", "reserved message name");', ""],
		["swap", "validName's only multi-argument calls pair a condition with its message"],
		["pair", K, "name.size() > 0 && name.size() <= L.MESSAGE_NAME_MAX_BYTES", "name.size() > 0"],
	],
	[
		"4",
		["guard", K, "!definitions.has(spec.name)", "true"],
		["swap", "registration takes a name and a definition, which differ in type"],
		["pair", K, "definitions.set(spec.name, definition);", ""],
	],
	[
		"5",
		["guard", K, "timeout > 0 && timeout <= L.MAX_ASK_TIMEOUT_SECONDS", "true"],
		["swap", K, "spec.timeout ?? L.DEFAULT_ASK_TIMEOUT_SECONDS", "L.DEFAULT_ASK_TIMEOUT_SECONDS ?? spec.timeout"],
		["pair", K, "spec.output === undefined && spec.timeout === undefined", "spec.output === undefined"],
	],
	[
		"9",
		["guard", K, "ordered.sort();", ""],
		["swap", "seal's calls take one argument; indices.set pairs a string name with a number index"],
		["pair", K, "setNames(ordered);\n\tsealed = true;", "setNames([]);\n\tsealed = true;"],
	],
	[
		"10",
		["guard", K, "!sealed,", "true,"],
		["swap", "the seal check pairs one condition with its message"],
		["pair", K, "setNames(ordered);\n\tsealed = true;", "setNames(ordered);"],
	],
	[
		"11",
		["guard", K, "definitions.size() <= L.MAX_MESSAGES", "true"],
		["swap", K, "definitions.size() <= L.MAX_MESSAGES", "L.MAX_MESSAGES <= definitions.size()"],
		["pair", K, "ordered.size() <= L.MAX_MESSAGES", "true"],
	],
	[
		"12",
		["guard", "attributes and parenting are the session's (unit N2); seal only returns the text"],
		["swap", "the text is one join of one list"],
		["pair", K, 'return names.join("\\n");', 'return names.join(",");'],
	],
	[
		"13",
		["guard", K, "i === 0 || ordered[i - 1] < ordered[i]", "true"],
		["swap", K, "i === 0 || ordered[i - 1] < ordered[i]", "i === 0 || ordered[i] < ordered[i - 1]"],
		["pair", K, "setNames(ordered);\n}", "setNames([]);\n}"],
	],
	[
		"14",
		["guard", K, "if (definition.index === undefined) result.push(definition);", "result.push(definition);"],
		["swap", "an index is looked up by one name"],
		[
			"pair",
			K,
			'key === "index" ? indices.get(spec.name) : undefined',
			'key === "index" ? indices.get("unknown") : undefined',
		],
	],
	[
		"15",
		["guard", K, "index >= names.size()", "index >= names.size() - 1"],
		["swap", K, "index >= names.size()", "names.size() >= index"],
		["pair", K, "sealed = false;\n\tsetNames([]);", "sealed = false;"],
	],
	[
		"57",
		["guard", S, 'typeIs(node, "table") && descriptors.has(node)', "true"],
		["swap", "a builder copies one argument list into one frozen node"],
		["pair", S, "fields: table.freeze(copy)", "fields: fields"],
	],
	[
		"58",
		["guard", C, "value !== value || math.abs(value) === math.huge", "false"],
		["swap", C, "buffer.writef64(this.data, offset, value)", "buffer.writef64(this.data, value, offset)"],
		["pair", C, 'finite(buffer.readf32(this.data, offset), "payload");', ""],
	],
	[
		"59",
		["guard", C, "lead !== 0xed || second < 0xa0", "true"],
		[
			"swap",
			C,
			"buffer.readstring(this.packet, this.take(size), size)",
			"buffer.readstring(this.packet, size, this.take(size))",
		],
		["pair", C, "buffer.writestring(this.data, offset, text)", "undefined"],
	],
	[
		"60",
		["guard", C, "math.abs(math.sqrt(dot(i, i)) - 1) > L.CFRAME_TOLERANCE", "false"],
		["swap", C, "values = [v.X, v.Y, v.Z]", "values = [v.Z, v.Y, v.X]"],
		["pair", C, "frame(values, p);\n\t\t\t\t\tconst result = new CFrame(", "const result = new CFrame("],
	],
	[
		"61",
		["guard", S, '!numericName(name), "numeric property name"', 'true, "numeric property name"'],
		["swap", S, "copy[i - 1][0] < copy[i][0]", "copy[i][0] < copy[i - 1][0]"],
		["pair", C, "object[name] = c.decode(r, `${p}.${name}`)", "object[name] = undefined"],
	],
	[
		"62",
		["guard", "unsupported types are CRU019 in the transformer (unit T1); no runtime node exists for them"],
		["swap", "no runtime site"],
		["pair", "no runtime site"],
	],
	[
		"63",
		[
			"guard",
			S,
			"phase <= 1 && (previousLiteral === undefined || literalLess(previousLiteral, branch.value))",
			"true",
		],
		["swap", S, "literalLess(previousLiteral, branch.value)", "literalLess(branch.value, previousLiteral)"],
		["pair", C, "w.varuint(tag);", ""],
	],
	[
		"64",
		["guard", S, "body.fields.every(([name]) => name !== key)", "true"],
		["swap", "selection compares a value with a literal, which is symmetric"],
		["pair", C, "v[node.key] = node.value;", ""],
	],
	[
		"65",
		["guard", S, "const kind = runtimeKind(branch);", "const kind = tostring(i);"],
		["swap", S, "previousKind < kind", "kind < previousKind"],
		["pair", S, "(discriminant === undefined || discriminant === branch.key)", "true"],
	],
	[
		"66",
		["guard", C, "i > 0 && !less(last, k)", "false"],
		["swap", C, "key.encode(w, k, `${p}.key`)", "key.encode(w, val, `${p}.key`)"],
		["pair", C, "value.encode(w, val, `${p}[]`)", "undefined"],
	],
	[
		"67",
		["guard", C, "if (nilable) return undefined;", "return undefined;"],
		["swap", C, "this.slots[i];", "this.slots[this.slots.size() - 1 - i];"],
		["pair", C, "tostring(this.nextSlot++)", "tostring(this.nextSlot)"],
	],
	[
		"68",
		["guard", C, 'return fail(p, "missing Instance", true);', 'return fail(p, "missing Instance");'],
		["swap", "the client's drop and settle outcomes are the session's (unit N2)"],
		["pair", "the client's drop and settle outcomes are the session's (unit N2)"],
	],
	[
		"69",
		["guard", C, 'if (active.has(object)) fail(path, "cycle");', ""],
		["swap", C, "w.u8(value ? 2 : 1)", "w.u8(value ? 1 : 2)"],
		["pair", C, "active.delete(object);", ""],
	],
	[
		"70",
		["guard", C, "count > L.MAX_COLLECTION_ENTRIES - this.entries", "false"],
		[
			"swap",
			C,
			"buffer.copy(grown, 0, this.data, 0, this.cursor)",
			"buffer.copy(this.data, 0, grown, 0, this.cursor)",
		],
		["pair", C, "this.depth--;", ""],
	],
	[
		"71",
		["guard", C, "i === 0 || byte !== 0", "true"],
		["swap", C, "uint(cursor, buffer.len(packet));", "uint(buffer.len(packet), cursor);"],
		[
			"pair",
			C,
			"this.cursor === buffer.len(this.packet) && this.nextSlot === this.slotCount + 1",
			"this.cursor === buffer.len(this.packet)",
		],
	],
	[
		"71 count minimum",
		["array", C, "r.count(children[0].minBytes, p)", "r.count(0, p)"],
		["map", C, "r.count(key.minBytes + value.minBytes, p)", "r.count(0, p)"],
		["dynamic", C, "r.count(tag === 5 ? 1 : 2, path)", "r.count(0, path)"],
		["record", C, "r.count(tag === 5 ? 1 : 2, path)", "r.count(tag === 5 ? 1 : 1, path)"],
	],
	[
		"72",
		["guard", "the codec reads no sender; the session supplies the engine sender (unit N2)"],
		["swap", "no runtime site"],
		["pair", "no runtime site"],
	],
	[
		"73",
		["guard", C, 'fail(path, "expected finite number");', "fail(path, tostring(value));"],
		["swap", C, 'fail(path, "expected finite number");', 'fail("expected finite number", path);'],
		["pair", "the codec never logs, so nothing pairs with a log call"],
	],
	[
		"Limits",
		["guard", "constants hold no guard; the value mutants below break the parity table"],
		["swap", "constants take no arguments"],
		["pair", "constants pair with nothing"],
		["value", L, "export const MAX_DEPTH = 32;", "export const MAX_DEPTH = 33;"],
		["value", L, "export const CFRAME_TOLERANCE = 0.001;", "export const CFRAME_TOLERANCE = 0.002;"],
		["value", L, "export const MAX_STRING_BYTES = 65_536;", "export const MAX_STRING_BYTES = 65_535;"],
	],
	[
		"Extra",
		["guard", C, "assert(i < 4 || byte <= 15", "assert(true"],
		["guard", C, 'if (b > 1) fail(p, "invalid boolean");', ""],
		["guard", C, "tostring(slot) === key", "true"],
		["guard", C, "++this.visits > L.MAX_VALUE_VISITS", "false"],
		["guard", C, "++this.depth > L.MAX_DEPTH", "++this.depth > L.MAX_DEPTH + 1"],
		["guard", C, "this.slots.size() < L.MAX_INSTANCE_SLOTS", "true"],
		["guard", C, "empty || node.fields.size() > 0", "true"],
		["guard", S, "!admitsNil(node(element))", "true"],
		["guard", C, "math.abs(det - 1) > L.CFRAME_TOLERANCE", "false"],
		["guard", C, 'assert(lead !== 0xe0 || second >= 0xa0, "invalid UTF-8");', ""],
		["guard", C, 'assert(lead !== 0xf4 || second < 0x90, "invalid UTF-8");', ""],
		["guard", C, "count > 0 && count <= buffer.len(bytes) - offset", "count > 0"],
		["guard", C, "value >= 0 && value <= maximum && value % 1 === 0", "true"],
		["guard", C, "buffer.copy(grown, 0, this.data, 0, this.cursor)", "undefined"],
		["pair", C, "w.bytes(key);", ""],
		["pair", C, "const visits = node.kind !== ", "const visits = true || node.kind !== "],
	],
];

const mutants = [];
const absent = [];
for (const [rule, ...operators] of rules)
	for (const [operator, file, before, after] of operators)
		if (before === undefined) absent.push(`${rule} ${operator}: not applicable, ${file}`);
		else mutants.push([`${rule} ${operator}`, file, before, after]);

function run(cwd, command, args) {
	const result = spawnSync(command, args, { cwd, encoding: "utf8", timeout: 300000 });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-network-mutants-"));
try {
	for (const name of ["tsconfig.rbxts.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	for (const name of [".lune", "test"])
		fs.cpSync(path.join(repo, name), path.join(scratch, name), { recursive: true });
	fs.cpSync(path.join(repo, "packages/crucible"), path.join(scratch, "packages/crucible"), {
		recursive: true,
		filter: (p) => !p.split(path.sep).includes("node_modules"),
	});
	for (const name of ["node_modules", "packages/crucible/node_modules", "build"])
		fs.symlinkSync(path.join(repo, name), path.join(scratch, name), "junction");
	const originals = new Map(mutants.map(([, file]) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const build = () =>
		run(scratch, process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	for (const [name, file, before] of mutants)
		assert.ok(originals.get(file).includes(before), `${name} pattern exists: ${before}`);
	let built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	const baseline = run(scratch, "lune", ["run", "test", ...specs]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	for (const line of absent) console.log(line);
	let killed = 0;
	for (const [name, file, before, after] of mutants) {
		const original = originals.get(file);
		assert.ok(original.includes(before), `${name} pattern exists: ${before}`);
		fs.writeFileSync(path.join(scratch, file), original.replaceAll(before, after));
		built = build();
		assert.equal(built.status, 0, `${name} must compile\n${built.stdout}${built.stderr}`);
		const tested = run(scratch, "lune", ["run", "test", ...specs]);
		assert.equal(tested.status, 1, `${name} survived\n${tested.stdout}${tested.stderr}`);
		const failure = (tested.stdout + tested.stderr).match(/FAIL (network[^\n]*)/);
		assert.ok(failure, `${name} must fail a network assertion`);
		console.log(`KILLED ${name} | ${path.basename(file)} | first failure: ${failure[1]}`);
		killed++;
		fs.writeFileSync(path.join(scratch, file), original);
	}
	built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	console.log(
		`${killed}/${mutants.length} source mutants killed; ${absent.length} operators not applicable; restored build 0`,
	);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
