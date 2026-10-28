import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-boot-mutants-"));
const mutants = [
	[
		"constructor effect",
		"packages/crucible/src/internal/boot.ts",
		"return construct(s);",
		"return {} as CrucibleObject;",
		"constructor that throws rejects",
	],
	[
		"awake effect",
		"packages/crucible/src/internal/boot.ts",
		"if (hook === Hook.Awake) return (s) => hooks().awake(s.object as CrucibleObject, 0);",
		"if (hook === Hook.Awake) return undefined;",
		"Subsystem awake fault rejects",
	],
	[
		"faulted shutdown",
		"packages/crucible/src/internal/boot.ts",
		"const object = record.state?.object;",
		"const object = record.state?.fault === undefined ? record.state?.object : undefined;",
		"throwing shutdown logs",
	],
	[
		"custom fixed rate",
		"packages/crucible/src/internal/boot.ts",
		"options.fixedRate ?? 60",
		"60",
		"fixedRate defaults",
	],
	[
		"frame caller swap",
		"packages/crucible/src/internal/boot.ts",
		"frame[FrameHook.Update](o as CrucibleObject, dt)",
		"frame[FrameHook.LateUpdate](o as CrucibleObject, dt)",
		"ignite wires all protected",
	],
	[
		"membership leave pair",
		"packages/crucible/src/internal/boot.ts",
		"set.leave(s.object!)",
		"set.join(s.object!)",
		"ignite wires all protected",
	],
	[
		"wrong side guard",
		"packages/crucible/src/server/index.ts",
		'if (!game.GetService("RunService").IsServer())',
		"if (false)",
		"side entries enforce",
	],
	[
		"watch disconnect pair",
		"packages/crucible/src/internal/boot.ts",
		"\n\t\tstop();",
		"\n\t\t{}",
		"reviewer A and B rollback",
	],
	[
		"deadline release guard",
		"packages/crucible/src/internal/boot.ts",
		"const expired = port.clock() - startTime >= SHUTDOWN_DEADLINE_SECONDS;",
		"const expired = false;",
		"S1 deadline is 25",
	],
	[
		"shutdown failure output",
		"packages/crucible/src/internal/boot.ts",
		"if (!ok) port.log(LogLevel.Error, `${record.id} onShutdown failed: ${err}`);",
		"",
		"throwing shutdown logs",
	],
	[
		"same typed matcher argument swap",
		"packages/crucible/src/internal/boot.ts",
		"matches(child.Name, segment)",
		"matches(segment, child.Name)",
		"module walk resolves",
	],
	[
		"globstar zero depth",
		"packages/crucible/src/internal/boot.ts",
		'if (segment === "**") walk(node, segments, index + 1);',
		"",
		"module walk resolves",
	],
	[
		"registration order",
		"packages/crucible/src/internal/boot.ts",
		"a.GetFullName() < b.GetFullName()",
		"a.GetFullName() > b.GetFullName()",
		"module walk resolves",
	],
	[
		"rollback admission guard",
		"packages/crucible/src/internal/boot.ts",
		'if (stage === "rollback") return [];',
		"",
		"reviewer A and B rollback",
	],
	[
		"child ownership",
		"packages/crucible/src/internal/boot.ts",
		'if (stage === "booting") owned.add(s);',
		"",
		"reviewer A and B rollback",
	],
	[
		"paired tick untick shortcut",
		"packages/crucible/src/internal/boot.ts",
		"untick: (s) => scheduler.remove(s),",
		"untick: () => {},",
		"reviewer A and B rollback",
	],
	[
		"cap rejection",
		"packages/crucible/src/internal/boot.ts",
		"reject(message);",
		"{}",
		"awake wave and child batch caps",
	],
	[
		"dependency completion",
		"packages/crucible/src/internal/boot.ts",
		"!dependents.get(record)!.every((r) => finished.has(r))",
		"false",
		"shutdown waits for dependents",
	],
	[
		"shutdown started guard",
		"packages/crucible/src/internal/boot.ts",
		"started.has(record) ||",
		"false ||",
		"shutdown waits for dependents",
	],
	[
		"deadline bypass",
		"packages/crucible/src/internal/boot.ts",
		"SHUTDOWN_DEADLINE_SECONDS = 25",
		"SHUTDOWN_DEADLINE_SECONDS = 31",
		"S1 deadline is 25",
	],
	[
		"client BindToClose",
		"packages/crucible/src/internal/boot.ts",
		'if (side.name === "server")',
		'if (side.name !== "server")',
		"throwing shutdown logs",
	],
	[
		"default fixed rate",
		"packages/crucible/src/internal/boot.ts",
		"options.fixedRate ?? 60",
		"options.fixedRate ?? 20",
		"fixedRate defaults",
	],
	[
		"start traceback output",
		"packages/crucible/src/internal/boot.ts",
		"tostring(fault.error)",
		'tostring(fault.error).split("\\n")[0]',
		"start fault output retains",
	],
	[
		"Error output",
		"packages/crucible/src/internal/roblox-port.ts",
		"task.spawn(() => error(`[Crucible] ${message}`, 0))",
		"warn(`[Crucible] ${message}`)",
		"Roblox full port maps",
	],
	[
		"frame event swap",
		"packages/crucible/src/internal/roblox-port.ts",
		"[FrameEvent.PreSimulation]: runService.PreSimulation",
		"[FrameEvent.PreSimulation]: runService.PostSimulation",
		"Roblox full port maps",
	],
	[
		"render unbind pair",
		"packages/crucible/src/internal/roblox-port.ts",
		"runService.UnbindFromRenderStep(name)",
		'runService.UnbindFromRenderStep("wrong")',
		"Roblox full port maps",
	],
	[
		"client booted flag",
		"packages/crucible/src/client/index.ts",
		"SetAttribute(BOOTED_ATTRIBUTE, true)",
		"SetAttribute(BOOTED_ATTRIBUTE, false)",
		"side entries enforce",
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
	const baseline = run("lune", ["run", "test", "boot"]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	let completed = 0;
	for (const [index, [name, file, before, after, target]] of mutants.entries()) {
		if (index < Number(process.argv[2] ?? 0)) continue;
		const original = originals.get(file);
		assert.ok(original.includes(before), `${name} pattern exists`);
		fs.writeFileSync(
			path.join(scratch, file),
			original
				.replaceAll(before, after)
				.replace(
					name === "child ownership"
						? 'if (stage === "booting") for (const s of created) owned.add(s);'
						: "__unused_pattern__",
					"",
				),
		);
		const compiled = build();
		assert.equal(compiled.status, 0, name + " build failed\n" + compiled.stdout + compiled.stderr);
		const tested = run("lune", ["run", "test", target]);
		assert.notEqual(tested.status, 0, `${name} survived\n${tested.stdout}${tested.stderr}`);
		assert.ok((tested.stdout + tested.stderr).includes("FAIL boot"), `${name} must fail its named spec`);
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
