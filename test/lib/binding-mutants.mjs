import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-binding-mutants-"));
const binding = "packages/crucible/src/internal/binding.ts";
const behaviour = "packages/crucible/src/crucible-behaviour.ts";
const mutants = [
	["G1 pending ignores rejection", binding, "!pair.rejected &&", "true &&", "F1 residual:"],
	[
		"N1 parked flag not cleared after restore",
		binding,
		"for (const other of held) other.parked = false;",
		"",
		"PN1 restored parked pair",
	],
	["N2 pending ignores suppression", binding, "!record.suppressed.has(instance))", "true)", "PN2 hook get"],
	[
		"N4 restore re-adds unparked pairs",
		binding,
		"held.filter((other) => other.parked)",
		"held",
		"PN4 refreshed parked pair",
	],
	[
		"N5 queue leaves parked flag set",
		binding,
		"\t\tpair.parked = false;\n\t\tpair.queued = false;",
		"\t\tpair.queued = false;",
		"PN4 refreshed parked pair",
	],
	[
		"V1 Roblox StarterPlayer exclusion",
		"packages/crucible/src/internal/roblox-port.ts",
		'\n\t\tgame.GetService("StarterPlayer"),',
		"",
		"Roblox binding port maps",
	],
	[
		"V4 wrong class warns only once per record",
		binding,
		"if (!pair.wrongClass)",
		"if (!pair.wrongClass && wrongClasses.get(record) === undefined)",
		"wrong class warns once",
	],
	["V5 warning reset", binding, "pair.warnings.delete(field);", "", "invalid valid same invalid"],
	[
		"V6 parked pairs lost",
		binding,
		"queued = [...held.filter((other) => other.parked), ...queued];",
		"queued = [...queued];",
		"outside get reconciles only",
	],
	["V8 reset pair not requeued", binding, "if (pair.reset) queue(pair);", "", "RemoveTag then AddTag and Parent nil"],
	[
		"F1 false pending warning",
		binding,
		"state !== undefined\n\t\t\t\t? !isLive(state)\n\t\t\t\t: pair.queued ||\n\t\t\t\t\t(!pair.rejected &&\n\t\t\t\t\t\tport().inWorld(instance) &&\n\t\t\t\t\t\tport().hasTag(instance, record.tag) &&\n\t\t\t\t\t\t!record.suppressed.has(instance))",
		"state === undefined || !isLive(state)",
		"untagged instance does not warn pending",
	],
	[
		"F2 parked refresh ignored",
		binding,
		"if (pair.parked)",
		"if (pair.parked && !pair.queued)",
		"outside get runs a parked pair",
	],
	[
		"F8 record id in class warning",
		binding,
		'const className = record.id.split(":").pop() ?? record.id;',
		"const className = record.id;",
		"wrong class warns once",
	],
	["no leave-world admission", binding, "engine.inWorld(instance)", "true", "Parent nil fires removed"],
	["no tag admission", binding, "engine.hasTag(instance, record.tag)", "true", "AddTag then get outside"],
	[
		"Roblox tag mapping",
		"packages/crucible/src/internal/roblox-port.ts",
		"collection.GetTagged(tag)",
		"[]",
		"Roblox binding port maps",
	],
	[
		"public root export",
		"packages/crucible/src/index.ts",
		'export { CrucibleBehaviour, type BehaviourClass } from "./crucible-behaviour";',
		"",
		"rule 3 and 4",
	],
	[
		"instance assignment",
		behaviour,
		"this.instance = state.instance as I;",
		"this.instance = undefined as unknown as I;",
		"rule 3 and 4",
	],
	["disable request", behaviour, "enabled ? Want.Enabled : Want.Disabled", "Want.Enabled", "rule 3 and 4"],
	["C1 all pairs", binding, "queued = [pair];", "queued = queued;", "outside get reconciles only"],
	["C1 no outside reconcile", binding, "drain();\n\t\tqueued =", "queued =", "AddTag then get outside"],
	["C1 no pending warning", binding, "if (!getWarnings.has(site))", "if (false)", "get inside a hook never"],
	["C3 default scope", binding, "return port().bindingScope(instance);", "return true;", "default accepts excludes"],
	["bound get skips accepts", binding, "!accepted(pair, have)", "false", "override replaces accepts"],
	[
		"C4 remove suppression",
		binding,
		"record.suppressed.add(state.instance);",
		"record.suppressed.delete(state.instance);",
		"remove and fault suppression",
	],
	[
		"C4 fault suppression",
		binding,
		"if (state.phase === Phase.Faulted && !pair.reset) record.suppressed.add(state.instance);",
		"",
		"remove and fault suppression",
	],
	[
		"C4 no suppression reset",
		binding,
		"record.suppressed.delete(instance);",
		"record.suppressed.add(instance);",
		"remove and fault suppression",
	],
	[
		"required missing",
		binding,
		"value === undefined ? !descriptor.required",
		"value === undefined ? true",
		"required absent and any present",
	],
	["present invalid", binding, ": descriptor.check(value)", ": true", "required absent and any present"],
	[
		"field update",
		binding,
		"(state.object as Record<string, unknown>)[field] = value;",
		"(state.object as Record<string, unknown>)[field] = undefined;",
		"valid attribute writes update",
	],
	[
		"warning dedup",
		binding,
		"if (pair.warnings.get(field) === condition) return;",
		"",
		"valid attribute writes update",
	],
	[
		"admission watchers",
		binding,
		"\n\tconnect(pair);",
		"\n\tif (false) connect(pair);",
		"admission installs watchers",
	],
	["reconnect snapshot", binding, "applyAttribute(state, field);\n}", "{ }\n}", "reconnect snapshots fields"],
	["running pair queued", binding, "if (pair.running) return;", "", "accepts refreshing and getting"],
	[
		"claim cleanup",
		binding,
		"if (worker !== undefined) clearClaim(worker);",
		"",
		"caller clears the constructing worker",
	],
	[
		"claim thread scope",
		"packages/crucible/src/internal/construction-claim.ts",
		"return claims.get(coroutine.running());",
		"let found: ObjectState | undefined; for (const [, state] of claims) found = state; return found;",
		"yielding constructor clears its claim",
	],
	[
		"wrong class admission",
		binding,
		"!record.instanceClasses.some((name) => engine.isA(instance, name))",
		"record.instanceClasses.size() < 0",
		"wrong class warns once",
	],
	["wrong class warning dedup", binding, "if (!pair.wrongClass)", "if (true)", "wrong class warns once"],
	["ancestry watcher", binding, "queue(pair);\n\t\t\tschedule();", "schedule();", "default accepts excludes"],
	[
		"no admission recheck after accepts",
		binding,
		"\n\tif (\n\t\tpair.reset ||\n\t\t!engine.inWorld(instance) ||\n\t\t!engine.hasTag(instance, record.tag) ||\n\t\trecord.suppressed.has(instance)\n\t)\n\t\treturn undefined;",
		"",
		"binding rereads tag changes",
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
	const originals = new Map(mutants.map(([, file]) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const build = () =>
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	assert.equal(build().status, 0, "baseline build");
	const baseline = run("lune", ["run", "test", "binding"]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	let completed = 0;
	for (const [index, [name, file, before, after, target]] of mutants.entries()) {
		if (index < Number(process.argv[2] ?? 0)) continue;
		const original = originals.get(file);
		assert.ok(original.includes(before), `${name} pattern exists`);
		fs.writeFileSync(path.join(scratch, file), original.replaceAll(before, after));
		const compiled = build();
		assert.equal(compiled.status, 0, name + " build failed\n" + compiled.stdout + compiled.stderr);
		const tested = run("lune", ["run", "test", target]);
		assert.notEqual(tested.status, 0, `${name} survived\n${tested.stdout}${tested.stderr}`);
		assert.ok((tested.stdout + tested.stderr).includes("FAIL binding.luau"), `${name} must fail its named spec`);
		console.log(`KILLED ${name} | ${target} | build 0, spec ${tested.status}`);
		completed++;
		fs.writeFileSync(path.join(scratch, file), original);
	}
	assert.equal(build().status, 0, "restored build");
	console.log(
		`${completed}/${mutants.length - Number(process.argv[2] ?? 0)} selected source mutants killed; restored build 0`,
	);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
