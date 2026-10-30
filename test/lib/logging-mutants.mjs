import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-logging-mutants-"));
const mutants = [
	[
		"L3 reversed delivery",
		"packages/crucible/src/logging.ts",
		"entries.push(added);",
		"entries.unshift(added);",
		"L3 subscriptions",
	],
	[
		"L1 filter bypass",
		"packages/crucible/src/logging.ts",
		"if (emitting || level < minimum) return;",
		"if (emitting) return;",
		"L1 disabled",
	],
	[
		"L2 stale aggregate",
		"packages/crucible/src/logging.ts",
		"registration.minimum = value;",
		"registration.minimum = registration.minimum;",
		"L2 aggregate",
	],
	[
		"L3 duplicate subscription",
		"packages/crucible/src/logging.ts",
		"if (existing !== undefined) return existing.subscription;",
		"if (existing !== undefined) return { setMinimum() {}, disconnect() {} };",
		"L3 subscriptions",
	],
	[
		"L4 failed sink retained",
		"packages/crucible/src/logging.ts",
		"registration.subscription.disconnect();",
		"{}",
		"L4 throwing",
	],
	[
		"L5 recursion guard removed",
		"packages/crucible/src/logging.ts",
		"if (emitting || level < minimum) return;",
		"if (level < minimum) return;",
		"L5 whole",
	],
	[
		"L6 mutable context",
		"packages/crucible/src/logging.ts",
		"return table.freeze(context);",
		"return context;",
		"L6 frozen",
	],
	[
		"L6 mutable record",
		"packages/crucible/src/logging.ts",
		"const record: LogRecord = table.freeze({",
		"const record: LogRecord = table.clone({",
		"L6 frozen",
	],
	[
		"L7 escape disabled",
		"packages/crucible/src/logging.ts",
		'if (pair === "{{" || pair === "}}")',
		"if (false as boolean)",
		"L7 renderer",
	],
	["L8 caller traceback removed", "packages/crucible/src/logging.ts", 'debug.traceback("", 3)', '""', "L8 game"],
	[
		"L9 mandatory diagnostics silenced",
		"packages/crucible/src/internal/diagnostics.ts",
		"loggingPort().writeInternal(level, message)",
		"undefined",
		"L9 private",
	],
	[
		"L10 fallback retry",
		"packages/crucible/src/internal/log-output.ts",
		"fallback = true;",
		"fallback = false;",
		"L10 native",
	],
	["L11 length bound removed", "packages/crucible/src/logging.ts", "source.size() <= 256", "true", "L11 names"],
	[
		"L12 preboot default disabled",
		"packages/crucible/src/logging.ts",
		"loggingPort().isStudio() ? LogLevel.Debug : LogLevel.Info",
		"LogLevel.Off",
		"L12 module",
	],
];
const L = "packages/crucible/src/logging.ts";
const O = "packages/crucible/src/internal/log-output.ts";
const D = "packages/crucible/src/internal/diagnostics.ts";
const verifierMutants = [
	[
		"1",
		"boundary filter off by one level",
		[[L, "if (emitting || level < minimum) return;", "if (emitting || level + 1 < minimum) return;"]],
		"L1 disabled",
	],
	[
		"2",
		"disconnect skips recompute",
		[[L, "registrations.delete(sink);\n\t\t\t\tderive();", "registrations.delete(sink);"]],
		"L2 aggregate",
	],
	[
		"2",
		"live default is Debug",
		[[L, "loggingPort().isStudio() ? LogLevel.Debug : LogLevel.Info", "LogLevel.Debug"]],
		"L2 aggregate",
	],
	[
		"3",
		"disconnect not idempotent",
		[[L, "disconnect() {\n\t\t\t\tif (!registration.connected) return;\n", "disconnect() {\n"]],
		"L3 subscriptions",
	],
	[
		"4",
		"sink report drops traceback on throw",
		[
			[
				L,
				'runtimeDiagnostic("error", `Log sink failed: ${tostring(err)}`);',
				'runtimeDiagnostic("error", `Log sink failed: ${status === CallResult.Threw ? tostring(err).split("\\n")[0] : tostring(err)}`);',
			],
		],
		"L4 throwing",
	],
	[
		"4",
		"sink report drops traceback always",
		[
			[
				L,
				'runtimeDiagnostic("error", `Log sink failed: ${tostring(err)}`);',
				'runtimeDiagnostic("error", `Log sink failed: ${tostring(err).split("\\n")[0]}`);',
			],
		],
		"L4 throwing",
	],
	[
		"5",
		"guard released before fanout",
		[
			[
				L,
				"for (const registration of accepted) {",
				"emitting = false;\n\t\t\tfor (const registration of accepted) {",
			],
		],
		"L5 whole",
	],
	[
		"6",
		"instance short name in Roblox adapter",
		[
			[
				O,
				"describeInstance: (instance) => instance.GetFullName(),",
				"describeInstance: (instance) => instance.Name,",
			],
		],
		"L6 frozen",
	],
	[
		"6",
		"non-primitive kept raw",
		[[L, "\t\t\t\t\t: tostring(value);", "\t\t\t\t\t: (value as never);"]],
		"L6 frozen",
	],
	[
		"7",
		"missing key rendered empty",
		[
			[
				L,
				"value === undefined ? template.sub(index, close) : tostring(value)",
				'value === undefined ? "" : tostring(value)',
			],
		],
		"L7 renderer",
	],
	[
		"8",
		"traceback level points inside logging",
		[[L, 'debug.traceback("", 3)', 'debug.traceback("", 1)']],
		"L8 game",
	],
	[
		"8",
		"game error thrown synchronously",
		[
			[
				O,
				'task.spawn(() => error(`${text}\\n${record.traceback ?? ""}`, 0));',
				'error(`${text}\\n${record.traceback ?? ""}`, 0);',
			],
		],
		"L8 game",
	],
	[
		"9",
		"diagnostics routed to game port",
		[
			[
				D,
				"loggingPort().writeInternal(level, message)",
				'loggingPort().writeGame({ sequence: 0, time: 0, source: "Crucible", level: level === "error" ? 3 : 2, template: message, context: {}, message } as never)',
			],
		],
		"L9 private",
	],
	[
		"10",
		"logger recreated per write",
		[[O, 'gameLogger ??= service.GetLogger("Game");', 'gameLogger = service.GetLogger("Game");']],
		"L10 native",
	],
	[
		"10",
		"Info drops context",
		[[O, "logger.Info(escape(text), record.context);", "logger.Info(escape(text));"]],
		"L10 native",
	],
	[
		"10",
		"Crucible warn unescaped",
		[
			[
				O,
				"() => crucibleLogger!.Warn(escape(`[Crucible] ${message}`), {}),",
				"() => crucibleLogger!.Warn(`[Crucible] ${message}`),",
			],
		],
		"L10 native",
	],
	[
		"11",
		"segment check removed",
		[
			[
				L,
				'for (const segment of source.split(".")) assert(segment.size() > 0, "Invalid logger name segment");',
				"",
			],
		],
		"L11 names",
	],
];
for (const [rule, name, [[file, before, after]], target] of verifierMutants) {
	mutants.push([`L${rule} ${name}`, file, before, after, target]);
}
mutants.push(
	[
		"L1 filter drops only Debug",
		"packages/crucible/src/logging.ts",
		"if (emitting || level < minimum) return;",
		"if (emitting || (level === LogLevel.Debug && level < minimum)) return;",
		"L1 disabled",
	],
	[
		"L2 added sink replaces aggregate",
		"packages/crucible/src/logging.ts",
		"minimum = math.min(minimum, added.minimum);",
		"minimum = added.minimum;",
		"L2 aggregate",
	],
	[
		"L10 Debug through Info",
		O,
		"logger.Output(escape(text), record.context)",
		"logger.Info(escape(text), record.context)",
		"L10 native",
	],
	["L10 Crucible warning through Game", O, "crucibleLogger!.Warn(", "gameLogger!.Warn(", "L10 native"],
);
mutants.push([
	"L10 warning context omitted",
	O,
	"Warn(escape(`[Crucible] ${message}`), {})",
	"Warn(escape(`[Crucible] ${message}`))",
	"L10 native",
]);
function run(command, args) {
	const result = spawnSync(command, args, { cwd: scratch, encoding: "utf8", timeout: 180000 });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}
try {
	fs.mkdirSync(path.join(scratch, "build"), { recursive: true });
	fs.copyFileSync(path.join(repo, "build/sample-game.rbxl"), path.join(scratch, "build/sample-game.rbxl"));
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
	const baseline = run("lune", ["run", "test", "logging"]);
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
		assert.equal(tested.status, 1, `${name} survived\n${tested.stdout}${tested.stderr}`);
		assert.ok((tested.stdout + tested.stderr).includes("FAIL logging"), `${name} must fail its named spec`);
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
