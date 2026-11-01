import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-config-mutants-"));
const R = "packages/crucible/src/internal/configs/runtime.ts";
const P = "packages/crucible/src/internal/configs/port.ts";
const C = "packages/crucible/src/configs.ts";
// [rule, operator, name, file, before, after, spec filter]. Operators: guard (delete a guard or reset), swap (two
// same-typed arguments or operands), pair (short-circuit one half of a paired operation), value (a changed result).
const mutants = [
	[
		"1",
		"value",
		"untransformed macro message",
		C,
		'error("Crucible configs need the @trioforge/crucible/transformer plugin");\n}\n\n/** Declares a targeted',
		'error("configs");\n}\n\n/** Declares a targeted',
		"1 92 untransformed",
	],
	["2", "guard", "Crucible segment allowed", R, 'segments[0] !== "Crucible" && ', "", "92 2 3 names"],
	[
		"2",
		"guard",
		"name length off by one",
		R,
		"if (name.size() > L.CONFIG_NAME_MAX_BYTES)",
		"if (name.size() > L.CONFIG_NAME_MAX_BYTES + 1)",
		"92 2 3 names",
	],
	["2", "pair", "name never indexed", R, "\tnames.add(name);\n", "\n", "92 2 3 names"],
	[
		"3",
		"guard",
		"count limit off by one",
		R,
		"if (declarations.size() >= L.MAX_CONFIG_DECLARATIONS)",
		"if (declarations.size() > L.MAX_CONFIG_DECLARATIONS)",
		"92 2 3 names",
	],
	["3", "guard", "registry never seals", R, "\tsealed = true;\n", "\n", "3 104"],
	[
		"4",
		"value",
		"emit form selects the port",
		R,
		"\tassertDescriptor(node);\n",
		"\tassertDescriptor(node);\n\tconfigPort();\n",
		"4 emit forms",
	],
	[
		"92",
		"guard",
		"invalid fallback accepted",
		R,
		"\tif (!ok) {\n\t\tconst f = failure(admitted);",
		"\tif (false as boolean) {\n\t\tconst f = failure(admitted);",
		"92 93 declarations",
	],
	[
		"92",
		"value",
		"fallback kept by reference",
		R,
		"fallback: admitted, global: { value: admitted }",
		"fallback, global: { value: fallback }",
		"1 94",
	],
	[
		"93",
		"guard",
		"read of an unknown player",
		R,
		"\tconst scope = slots.get(player);\n\tif (scope === undefined) return declaration.fallback;\n",
		"\tconst scope = slots.get(player)!;\n",
		"92 93 declarations",
	],
	[
		"93",
		"value",
		"read fetches",
		R,
		"export function readGlobal(declaration: Declaration): unknown {\n",
		"export function readGlobal(declaration: Declaration): unknown {\n\tif (owner !== undefined) configPort().global();\n",
		"92 93 declarations",
	],
	["94", "guard", "no backoff", R, "if (attempt > 0)", "if (false as boolean)", "7 94"],
	[
		"94",
		"swap",
		"retry base and cap swapped",
		R,
		"math.min(L.CONFIG_FETCH_RETRY_CAP_SECONDS, L.CONFIG_FETCH_RETRY_BASE_SECONDS * 2",
		"math.min(L.CONFIG_FETCH_RETRY_BASE_SECONDS, L.CONFIG_FETCH_RETRY_CAP_SECONDS * 2",
		"7 94",
	],
	["94", "pair", "first fetch never applied", R, "\t\t\tcommit([result]);\n", "\n", "1 94"],
	[
		"94",
		"guard",
		"LoadFailed snapshot applied",
		R,
		"\tif (failed(snapshot)) {\n\t\tsnapshot.disconnect();",
		"\tif (false as boolean) {\n\t\tsnapshot.disconnect();",
		"7 94",
	],
	[
		"95",
		"guard",
		"untargeted value check removed",
		R,
		"\tif (scope.phase !== Phase.Loaded) return declaration.global.value;\n",
		"\n",
		"8 95 96",
	],
	[
		"95",
		"swap",
		"fallback and global value swapped",
		R,
		"\tif (scope === undefined) return declaration.fallback;\n\tif (scope.phase !== Phase.Loaded) return declaration.global.value;",
		"\tif (scope === undefined) return declaration.global.value;\n\tif (scope.phase !== Phase.Loaded) return declaration.fallback;",
		"8 95 96",
	],
	["95", "pair", "player cell never stored", R, "\t\t\t\tscope.values.set(declaration, cell);\n", "\n", "8 95 96"],
	[
		"96",
		"guard",
		"global readiness reads a slot",
		R,
		"\tif (player === undefined) return global.ready.value as boolean;\n\treturn slots.get(player)?.ready.value === true;",
		"\treturn slots.get(player!)?.ready.value === true;",
		"1 94",
	],
	["96", "pair", "readiness never set", R, "\t\tscope.ready.value = true;\n", "\n", "8 95 96"],
	[
		"97",
		"guard",
		"missing required field accepted",
		R,
		'else if (!admitsNil(child)) fail(`${path}.${name}`, "a required field is missing");',
		"else if (!admitsNil(child)) {}",
		"5 97 admission",
	],
	[
		"97",
		"guard",
		"text byte limit removed",
		R,
		"if (raw.size() > L.CONFIG_VALUE_MAX_BYTES)",
		"if (false as boolean)",
		"5 97 JSON",
	],
	[
		"97",
		"swap",
		"path and reason swapped",
		R,
		"fail(path, `expected a number, got ${typeOf(value)}`)",
		"fail(`expected a number, got ${typeOf(value)}`, path)",
		"5 97 110",
	],
	["97", "pair", "copy not frozen", R, "\t\treturn table.freeze(copy);", "\t\treturn copy;", "15 97"],
	[
		"97",
		"guard",
		"undeclared fields dropped",
		R,
		"if (!declared.has(key)) copy.set(key, json(",
		"if (false as boolean) copy.set(key, json(",
		"15 97",
	],
	[
		"97",
		"guard",
		"invalid value overwrites",
		R,
		"if (value !== KEEP && !deepEqual(cell.value, value))",
		"if (!deepEqual(cell.value, value))",
		"5 97 110",
	],
	[
		"98",
		"value",
		"UpdateAvailable refreshes",
		R,
		"if (live()) snapshot.updates++;",
		"if (live()) {\n\t\t\t\tsnapshot.updates++;\n\t\t\t\ttask.spawn(refresh);\n\t\t\t}",
		"3 98",
	],
	[
		"98",
		"guard",
		"UpdateAvailable ignored",
		R,
		"if (snapshot.updates !== snapshot.applied || failed(snapshot)) return true;",
		"if (failed(snapshot)) return true;",
		"100 98",
	],
	[
		"99",
		"pair",
		"commit per scope as it answers",
		R,
		"if (result !== undefined) results.push(result);",
		"if (result !== undefined) commit([result]);",
		"99 a refresh commits",
	],
	[
		"99",
		"guard",
		"equal values replaced",
		R,
		"if (value !== KEEP && !deepEqual(cell.value, value))",
		"if (value !== KEEP)",
		"4 99 100",
	],
	[
		"100",
		"guard",
		"no single flight",
		R,
		"if (flight !== undefined) {",
		"if (flight !== undefined && (false as boolean)) {",
		"10 100",
	],
	[
		"100",
		"guard",
		"current snapshots refreshed",
		R,
		" || !stale(snapshot)) return undefined;",
		") return undefined;",
		"10 100",
	],
	[
		"100",
		"pair",
		"no refresh bound",
		R,
		"\t\t\tconfigPort().wait(L.CONFIG_REFRESH_TIMEOUT_SECONDS);\n",
		"\t\t\tconfigPort().wait(math.huge);\n",
		"11 100 a refresh past",
	],
	[
		"100",
		"swap",
		"gone and loaded swapped in the result",
		R,
		"(scope) => scope.phase === Phase.Gone || (scope.phase === Phase.Loaded && !stale(scope.snapshot!))",
		"(scope) => scope.phase === Phase.Loaded || (scope.phase === Phase.Gone && !stale(scope.snapshot!))",
		"11 100 a failed",
	],
	[
		"100",
		"guard",
		"late first fetch kept listening",
		R,
		"\t\t\t\tif (result?.first) result.snapshot.disconnect();\n",
		"\n",
		"14 96 100",
	],
	[
		"100",
		"guard",
		"failed refresh treated as current",
		R,
		"if (!result.ok || failed(snapshot)) {",
		"if (!result.ok) {",
		"11 100 a failed",
	],
	["101", "pair", "leave keeps the scope fetching", R, "\tscope.phase = Phase.Gone;\n", "\n", "9 101"],
	["101", "pair", "leave keeps the snapshot listener", R, "\tscope.snapshot?.disconnect();\n}", "}", "8 95 96"],
	["101", "guard", "late result read after leave", R, "\tif (abandoned(scope)) return undefined;\n", "\n", "9 101"],
	[
		"101",
		"guard",
		"staged scope applied after leave",
		R,
		"if (!apply || scope.phase === Phase.Gone || (result.first",
		"if (!apply || (result.first",
		"101 a leave between",
	],
	[
		"102",
		"guard",
		"no request limit",
		R,
		"if (active < L.CONFIG_REQUEST_CONCURRENCY) {",
		"if (true as boolean) {",
		"102 at most four",
	],
	[
		"102",
		"swap",
		"queue served last in first out",
		R,
		"const waiting = queue.shift();",
		"const waiting = queue.pop();",
		"102 at most four",
	],
	["102", "pair", "slot never returned", R, "\telse active--;", "\telse active += 0;", "102 a queued fetch"],
	[
		"102",
		"guard",
		"queued request for a departed player",
		R,
		"\tif (abandoned(scope)) {\n\t\trelease();",
		"\tif (false as boolean) {\n\t\trelease();",
		"102 a queued fetch",
	],
	["104", "guard", "start does not wait once", R, "\t\tconfigPort().wait(0);\n", "\n", "104 activation"],
	[
		"104",
		"swap",
		"PlayerAdded and PlayerRemoving swapped",
		R,
		"port.onPlayerAdded(join), port.onPlayerRemoving(leave)",
		"port.onPlayerAdded(leave), port.onPlayerRemoving(join)",
		"104 activation",
	],
	[
		"104",
		"guard",
		"players connected without a player config",
		R,
		'\tif (!declarations.some((d) => d.kind === "player")) return;\n',
		"\n",
		"104 activation",
	],
	[
		"104",
		"pair",
		"present players not fetched",
		R,
		"\tfor (const player of port.players()) join(player);\n",
		"\n",
		"8 95 a global",
	],
	[
		"105",
		"guard",
		"Studio always live",
		P,
		'\t\t!game.GetService("RunService").IsStudio() ||\n',
		"\t\ttrue ||\n",
		"105 106",
	],
	[
		"106",
		"guard",
		"CrucibleLiveData ignored",
		P,
		'|| game.GetService("ServerStorage").GetAttribute("CrucibleLiveData") === true',
		"|| false",
		"105 106",
	],
	["107", "guard", "retirement never detected", R, "\tif (ended(owner)) retire();\n", "\n", "107 retirement"],
	[
		"107",
		"pair",
		"player listeners kept",
		R,
		"\tfor (const disconnect of listeners) disconnect();\n",
		"\n",
		"107 retirement",
	],
	["107", "pair", "global snapshot listener kept", R, "\tglobal.snapshot?.disconnect();\n", "\n", "107 retirement"],
	[
		"107",
		"guard",
		"results applied after retirement",
		R,
		"\tconst apply = live();",
		"\tconst apply = owner !== undefined;",
		"107 a fetch that lands",
	],
	["110", "guard", "no dedupe", R, "\tif (scope.diagnosed.has(key)) return;\n", "\n", "7 94"],
	["110", "guard", "dedupe never reset", R, "\tscope.diagnosed = new Set();\n", "\n", "5 97 110"],
	[
		"110",
		"value",
		"diagnostic as error",
		R,
		'runtimeDiagnostic("warning",',
		'runtimeDiagnostic("error",',
		"5 97 110",
	],
	[
		"110",
		"value",
		"value in the reason",
		R,
		"fail(path, `expected a number, got ${typeOf(value)}`)",
		"fail(path, `expected a number, got ${tostring(value)}`)",
		"5 97 110",
	],
	[
		"97",
		"guard",
		"table byte limit removed",
		R,
		"if (bytes > L.CONFIG_VALUE_MAX_BYTES)",
		"if (false as boolean)",
		"5 97 JSON",
	],
	[
		"104",
		"guard",
		"configless game activates",
		R,
		"\t\tif (declarations.size() === 0) return;\n",
		"\n",
		"104 a game without",
	],
];
function run(command, args) {
	const result = spawnSync(command, args, { cwd: scratch, encoding: "utf8", timeout: 300000 });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}
try {
	for (const name of ["tsconfig.rbxts.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	fs.cpSync(path.join(repo, ".lune"), path.join(scratch, ".lune"), { recursive: true });
	fs.cpSync(path.join(repo, "test"), path.join(scratch, "test"), { recursive: true });
	fs.cpSync(path.join(repo, "packages/crucible"), path.join(scratch, "packages/crucible"), {
		recursive: true,
		filter: (p) => !p.split(path.sep).includes("node_modules"),
	});
	fs.symlinkSync(path.join(repo, "node_modules"), path.join(scratch, "node_modules"), "junction");
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules"),
		path.join(scratch, "packages/crucible/node_modules"),
		"junction",
	);
	const originals = new Map(mutants.map(([, , , file]) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const build = () =>
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	assert.equal(build().status, 0, "baseline build");
	const baseline = run("lune", ["run", "test", "configs.luau"]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	const from = Number(process.argv[2] ?? 0);
	const only = process.argv[3] === undefined ? undefined : new RegExp(process.argv[3]);
	let killed = 0;
	const survivors = [];
	for (const [index, [rule, operator, name, file, before, after, target]] of mutants.entries()) {
		if (index < from || (only !== undefined && !only.test(name))) continue;
		const original = originals.get(file);
		assert.equal(original.split(before).length, 2, `${name}: the pattern occurs exactly once`);
		fs.writeFileSync(path.join(scratch, file), original.replace(before, after));
		const compiled = build();
		const tested = compiled.status === 0 ? run("lune", ["run", "test", `configs.luau: ${target}`]) : undefined;
		const output = tested === undefined ? compiled.stdout + compiled.stderr : tested.stdout + tested.stderr;
		fs.writeFileSync(path.join(scratch, file), original);
		if (tested?.status === 1 && output.includes(`FAIL configs.luau: ${target}`)) {
			console.log(`KILLED rule ${rule} ${operator} | ${name} | ${target}`);
			killed++;
		} else {
			console.log(`SURVIVED rule ${rule} ${operator} | ${name} | ${target}\n${output}`);
			survivors.push(name);
		}
	}
	assert.equal(build().status, 0, "restored build");
	assert.deepEqual(survivors, [], "every mutant is killed");
	console.log(`${killed}/${killed + survivors.length} selected config mutants killed; restored build 0`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
