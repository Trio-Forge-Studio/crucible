import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-network-mutants-"));
const C = "packages/crucible/src/internal/network/codec.ts";
const S = "packages/crucible/src/internal/network/schema.ts";
const K = "packages/crucible/src/internal/network/catalog.ts";
const mutants = [
	["9 catalog sorting guard", K, "ordered.sort();", "ordered.sort((a, b) => b < a);"],
	["9 catalog index pair", K, "indices.set(names[i], i)", "indices.set(names[i], names.size() - 1 - i)"],
	["9 seal publication pair", K, "setNames(ordered);\n\tsealed = true;", "setNames([]);\n\tsealed = true;"],
	["10 late declaration guard", K, "!sealed,", "true,"],
	["11 message ceiling guard", K, "definitions.size() <= L.MAX_MESSAGES", "true"],
	["13 adoption order guard", K, "i === 0 || ordered[i - 1] < ordered[i]", "true"],
	["13 adoption paired assignment", K, "setNames(ordered);\n}", "setNames([]);\n}"],
	[
		"14 adoption index lookup",
		K,
		'key === "index" ? indices.get(spec.name) : undefined',
		'key === "index" ? indices.get("unknown") : undefined',
	],
	["14 discard reset", K, "sealed = false;\n\tsetNames([]);", "sealed = false;"],
	["15 bounds guard", K, "index >= names.size()", "index >= names.size() - 1"],
	["57 descriptor copy", S, "table.freeze([name, node] as [string, Node])", "fields[0]"],
	["58 finite guard", C, "value !== value || math.abs(value) === math.huge", "false"],
	["58 width guard", C, "value >= 0 && value <= maximum && value % 1 === 0", "true"],
	["58 rounding guard", C, 'finite(buffer.readf32(this.data, offset), "payload");', "undefined;"],
	[
		"58 swapped f64 offset and value",
		C,
		"buffer.writef64(this.data, offset, value)",
		"buffer.writef64(this.data, value, offset)",
	],
	["58 omitted f64 write", C, "buffer.writef64(this.data, offset, value)", "undefined"],
	["59 surrogate guard", C, "lead !== 0xed || second < 0xa0", "true"],
	[
		"59 swapped string offset and length",
		C,
		"buffer.readstring(this.packet, this.take(size), size)",
		"buffer.readstring(this.packet, size, this.take(size))",
	],
	["59 omitted string bytes", C, "buffer.writestring(this.data, offset, text)", "undefined"],
	["60 row guard", C, "math.abs(dot(i, i) - 1) > L.CFRAME_TOLERANCE", "false"],
	["60 determinant guard", C, "math.abs(det - 1) > L.CFRAME_TOLERANCE", "false"],
	["60 swapped components", C, "values = [v.X, v.Y, v.Z]", "values = [v.Z, v.Y, v.X]"],
	[
		"60 paired reconstruction",
		C,
		"new Vector3(values[0], values[1], values[2])",
		"new Vector3(values[2], values[1], values[0])",
	],
	["61 empty object guard", C, "empty || node.fields.size() > 0", "true"],
	[
		"61 numeric property guard",
		S,
		'typeIs(name, "string") && tostring(tonumber(name)) !== name',
		'typeIs(name, "string")',
	],
	["61 fields order pair", C, "object[name] = c.decode(r, `${p}.${name}`)", "object[name] = undefined"],
	[
		"63 literal ordering guard",
		S,
		"phase <= 1 && (previousLiteral === undefined || literalLess(previousLiteral, branch.value))",
		"true",
	],
	["63 omitted union tag", C, "w.varuint(tag);", "undefined;"],
	["64 omitted discriminant", C, "v[node.key] = node.value;", "undefined;"],
	["65 ambiguous kind guard", S, "const kind = runtimeKind(branch);", "const kind = tostring(i);"],
	["66 nil element guard", S, "!admitsNil(element)", "true"],
	["66 key ordering guard", C, "i > 0 && !less(last, k)", "false"],
	["66 swapped key and value", C, "key.encode(w, k, `${p}.key`)", "key.encode(w, val, `${p}.key`)"],
	["66 omitted map values", C, "value.encode(w, val, `${p}[]`)", "undefined"],
	["67 missing Instance guard", C, "if (nilable) return undefined;", "return undefined;"],
	["67 omitted slot advance", C, "tostring(this.nextSlot++)", "tostring(this.nextSlot)"],
	["67 swapped slot ordering", C, "this.slots[i];", "this.slots[this.slots.size() - 1 - i];"],
	["69 cycle reset", C, "active.delete(object);", "undefined;"],
	["69 cycle guard", C, 'if (active.has(object)) fail(path, "cycle");', "undefined;"],
	["69 swapped boolean tags", C, "w.u8(value ? 2 : 1)", "w.u8(value ? 1 : 2)"],
	["69 omitted record keys", C, "w.bytes(key);", "undefined;"],
	["70 omitted growth copy", C, "buffer.copy(grown, 0, this.data, 0, this.cursor)", "undefined"],
	[
		"70 swapped growth buffers",
		C,
		"buffer.copy(grown, 0, this.data, 0, this.cursor)",
		"buffer.copy(this.data, 0, grown, 0, this.cursor)",
	],
	["70 entry guard", C, "count > L.MAX_COLLECTION_ENTRIES - this.entries", "false"],
	["70 depth guard", C, "++this.depth > L.MAX_DEPTH", "++this.depth > L.MAX_DEPTH + 1"],
	["70 omitted depth reset", C, "this.depth--;", "undefined;"],
	["70 visit guard", C, "++this.visits > L.MAX_VALUE_VISITS", "false"],
	["70 slot guard", C, "this.slots.size() < L.MAX_INSTANCE_SLOTS", "true"],
	["71 minimal varuint guard", C, "i === 0 || byte !== 0", "true"],
	["71 varuint overflow guard", C, "i < 4 || byte <= 15", "true"],
	["71 bool guard", C, 'if (b > 1) fail(p, "invalid boolean");', "undefined;"],
	["71 sidecar key guard", C, "tostring(slot) === key", "true"],
	[
		"71 completion pair",
		C,
		"this.cursor === buffer.len(this.packet) && this.nextSlot === this.slotCount + 1",
		"true",
	],
];

function run(command, args) {
	const result = spawnSync(command, args, { cwd: scratch, encoding: "utf8", timeout: 180000 });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}

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
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	let built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	const baseline = run("lune", ["run", "test", "network"]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	let killed = 0;
	for (const [name, file, before, after] of mutants.slice(Number(process.argv[2] ?? 0))) {
		const original = originals.get(file);
		assert.ok(original.includes(before), `${name} pattern exists`);
		fs.writeFileSync(path.join(scratch, file), original.replaceAll(before, after));
		built = build();
		assert.equal(built.status, 0, `${name} must compile\n${built.stdout}${built.stderr}`);
		const tested = run("lune", ["run", "test", "network"]);
		assert.equal(tested.status, 1, `${name} survived\n${tested.stdout}${tested.stderr}`);
		assert.match(tested.stdout + tested.stderr, /FAIL network/, `${name} must fail a network assertion`);
		console.log(`KILLED ${name} | source build 0 | network specs 1`);
		killed++;
		fs.writeFileSync(path.join(scratch, file), original);
	}
	built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	console.log(`${killed}/${mutants.length - Number(process.argv[2] ?? 0)} source mutants killed; restored build 0`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
