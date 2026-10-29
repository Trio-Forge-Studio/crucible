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
	[
		"R4 evaluate retains rejection",
		binding,
		"const engine = port();\n\tpair.rejected = undefined;",
		"const engine = port();",
		"Y4 j first",
	],
	[
		"R4 queue clears every reason",
		binding,
		'if (pair.rejected === "accepts") pair.rejected = undefined;',
		"pair.rejected = undefined;",
		"Y2 wrong class",
	],
	["R4 scope recorded as accepts", binding, 'scopeRejected ? "scope" : "accepts"', '"accepts"', "Y2b default scope"],
	[
		"R4 scope never pending",
		binding,
		'(pair.rejected === "scope" && port().bindingScope(instance))',
		"false",
		"Y2e corrected default scope",
	],

	[
		"M1 queue retains rejection",
		binding,
		'if (pair.rejected === "accepts") pair.rejected = undefined;',
		"",
		"R3 refresh clears",
	],
	[
		"M3 attribute rejection omitted",
		binding,
		'pair.rejected = "attributes";',
		"pair.rejected = undefined;",
		"X7 hook get",
	],
	[
		"M5 queue membership means pending",
		binding,
		"(state === undefined || !isLive(state)) &&",
		"pair.queued || (state === undefined || !isLive(state)) &&",
		"X6 two hook gets",
	],
	["R3 get clears rejection", binding, "pair.rejected = rejected;", "pair.rejected = undefined;", "X3 two hook gets"],
	[
		"R3 corrected attributes stay rejected",
		binding,
		'(pair.rejected === "attributes" && attributesValid)',
		"false",
		"X2 invalid attribute",
	],
	[
		"R3 ignores current attribute checks",
		binding,
		"attributesValid = false;",
		"attributesValid = true;",
		"X7 hook get",
	],
	[
		"R3 required absence allowed",
		binding,
		"value === undefined ? descriptor.required : !descriptor.check(value)",
		"value === undefined ? false : !descriptor.check(value)",
		"R3 attribute pending checks",
	],
	[
		"R3 optional absence rejected",
		binding,
		"value === undefined ? descriptor.required : !descriptor.check(value)",
		"value === undefined ? true : !descriptor.check(value)",
		"R3 attribute pending checks",
	],
	[
		"R3 get runs user accepts",
		binding,
		"const rejected = pair.rejected;",
		"accepted(pair); const rejected = pair.rejected;",
		"R3 accepts rejection",
	],

	[
		"G1 pending ignores rejection",
		binding,
		'(pair.rejected === undefined ||\n\t\t\t\t(pair.rejected === "attributes" && attributesValid) ||\n\t\t\t\t(pair.rejected === "scope" && port().bindingScope(instance)))',
		"true",
		"F1 residual:",
	],
	[
		"N1 parked flag not cleared after restore",
		binding,
		"for (const other of held) other.parked = false;",
		"",
		"PN1 restored parked pair",
	],
	[
		"N2 pending ignores suppression",
		binding,
		"!record.suppressed.has(instance)\n\t\t)",
		"true\n\t\t)",
		"PN2 hook get",
	],
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
		'(state === undefined || !isLive(state)) &&\n\t\t\t(pair.rejected === undefined ||\n\t\t\t\t(pair.rejected === "attributes" && attributesValid) ||\n\t\t\t\t(pair.rejected === "scope" && port().bindingScope(instance))) &&\n\t\t\tport().inWorld(instance) &&\n\t\t\tport().hasTag(instance, record.tag) &&\n\t\t\t!record.suppressed.has(instance)',
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
	[
		"C3 default scope",
		binding,
		"const ok = port().bindingScope(instance);",
		"const ok = true;",
		"default accepts excludes",
	],
	[
		"bound get skips accepts",
		binding,
		"const ok = accepted(pair, have);",
		"const ok = true;",
		"override replaces accepts",
	],
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
	[
		"W3 scope flag not reset before accepts",
		binding,
		"\tscopeRejected = false;\n\tscopeTarget = instance;\n\tif (!accepted(pair)) {",
		"\tscopeTarget = instance;\n\tif (!accepted(pair)) {",
		"Z5b scope rejection at boot",
	],
	[
		"W4 default accepts raises scope flag in scope",
		binding,
		"if (!ok && instance === scopeTarget) scopeRejected = true;",
		"if (instance === scopeTarget) scopeRejected = true;",
		"Z4 user accepts calls default scope",
	],
	[
		"R5 bound branch drops rejection reason",
		binding,
		'if (!ok) pair.rejected = scopeRejected ? "scope" : "accepts";',
		"",
		"Z1 bound pair re-evaluated",
	],
	[
		"R5 default scope flag ignores target",
		binding,
		"if (!ok && instance === scopeTarget) scopeRejected = true;",
		"if (!ok) scopeRejected = true;",
		"Z3 user accepts consults default scope",
	],
	[
		"A1 bound branch never updates scopeTarget",
		binding,
		"\t\tscopeTarget = instance;\n\t\tconst ok = accepted(pair, have);",
		"\t\tconst ok = accepted(pair, have);",
		"Q2 Q1 after another instance was evaluated last",
	],
	[
		"A2 bound branch records accepts for a scope rejection",
		binding,
		'if (!ok) pair.rejected = scopeRejected ? "scope" : "accepts";',
		'if (!ok) pair.rejected = "accepts";',
		"Q1 bound pair moved into ServerStorage, hook moves it back unsignalled, then get",
	],
	[
		"A3 bound branch does not reset the scope flag",
		binding,
		"\t\tscopeRejected = false;\n\t\tscopeTarget = instance;\n\t\tconst ok",
		"\t\tscopeTarget = instance;\n\t\tconst ok",
		"Q3 scope rejection then a bound user accepts rejection in one reconcile, then get",
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
		assert.ok(
			(tested.stdout + tested.stderr).match(/FAIL binding(?:-pending)?\.luau/),
			`${name} must fail its named spec`,
		);
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
