import { cpSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const scratch = mkdtempSync(join(tmpdir(), "crucible-r4-mutants-"));
mkdirSync(join(scratch, "packages/crucible"), { recursive: true });
cpSync("packages/crucible/out", join(scratch, "packages/crucible/out"), { recursive: true });
cpSync("test", join(scratch, "test"), { recursive: true });
cpSync("rokit.toml", join(scratch, "rokit.toml"));
const scheduler = "packages/crucible/out/internal/scheduler.luau";
const engine = "test/lib/engine.luau";
const dense = "packages/crucible/out/internal/dense-list.luau";
const rules = [
	[
		"M1 one binding name for every priority",
		scheduler,
		"port.bindRenderStep(`crucible:{priority}`, priority, run)",
		'port.bindRenderStep("crucible", priority, run)',
		"emptying_one_render_priority_",
	],
	[
		"M2 carry kept when the fixed lane empties",
		scheduler,
		"fixed.disconnect = nil\n\t\t\tself.carry = 0",
		"fixed.disconnect = nil",
		"scripted_dt_",
	],
	["M4 lateUpdate receives the fixed step", scheduler, "late:run(dt)", "late:run(self.step)", "lanes_use_"],
	["M5 update receives the fixed step", scheduler, "update:run(dt)", "update:run(self.step)", "lanes_use_"],
	[
		"M6 preRender receives the fixed step",
		scheduler,
		"return lane:run(dt)",
		"return lane:run(self.step)",
		"lanes_use_",
	],
	["M8 fixed lane never reconnects after emptying", scheduler, "\t\t\tfixed.disconnect = nil\n", "", "scripted_dt_"],
	[
		"M10 untick short-circuits the late lane",
		scheduler,
		"local changed = update.members:remove(s)\n\t\tif (late.members:remove(s) or changed)",
		"if (update.members:remove(s) or late.members:remove(s))",
		"disable_removes_both_",
	],
	[
		"M15 carry cleared on every untick",
		scheduler,
		"\t\tlocal update = self.lanes[2]\n\t\tlocal late = self.lanes[3]\n\t\tlocal changed",
		"\t\tself.carry = 0\n\t\tlocal update = self.lanes[2]\n\t\tlocal late = self.lanes[3]\n\t\tlocal changed",
		"fixed_carry_survives_",
	],
	["fixed receives frame dt", scheduler, "lane:run(self.step)", "lane:run(dt)", "scripted_dt_"],
	[
		"wrong render priority",
		scheduler,
		"port.bindRenderStep(`crucible:{priority}`, priority, run)",
		"port.bindRenderStep(`crucible:{priority}`, 0, run)",
		"S1_bindings_",
	],
	["fixed event", scheduler, "port.onFrame(1, function(dt)", "port.onFrame(5, function(dt)", "lanes_use_"],
	["post event", scheduler, "port.onFrame(2, function(dt)", "port.onFrame(5, function(dt)", "lanes_use_"],
	[
		"late before update",
		scheduler,
		"update:run(dt)\n\t\t\t\t\tlate:run(dt)",
		"late:run(dt)\n\t\t\t\t\tupdate:run(dt)",
		"lanes_use_",
	],
	["descending priorities", engine, "a.priority < b.priority", "a.priority > b.priority", "S1_bindings_"],
	["reverse equal binds", engine, "a.order < b.order", "a.order > b.order", "S1_bindings_"],
	[
		"plain PreRender before bindings",
		engine,
		"local steps = table.clone(e.renderSteps)",
		"frameSignal(FrameEvent.PreRender, dt, true)\n\t\tlocal steps = table.clone(e.renderSteps)",
		"S1_bindings_",
	],
	[
		"oldest plain PreRender first",
		engine,
		"frameSignal(FrameEvent.PreRender, dt, true)",
		"frameSignal(FrameEvent.PreRender, dt, false)",
		"S1_bindings_",
	],
	["server accepts render", scheduler, "if not self.config.renders and", "if false and", "server_refuses_"],
	["no frame blame", scheduler, "within(s, s, 6, caller", "within(s, nil, 6, caller", "throw_or_yield_"],
	["no fixed cap", scheduler, "math.min(whole, MAX_FIXED_STEPS)", "whole", "a long frame", "scheduler-accumulator"],
	["discard fraction", scheduler, "math.max(total - whole * step, 0)", "0", "short frames", "scheduler-accumulator"],
	[
		"no dt clamp",
		scheduler,
		"math.clamp(frameDt, 0, MAX_FRAME_SECONDS)",
		"frameDt",
		"a negative frame",
		"scheduler-accumulator",
	],
	["no pool compaction", dense, "self:compactIfSparse()", "-- no compaction", "pool_rule_all_"],
	[
		"no untick",
		scheduler,
		"function Scheduler:remove(s)",
		"function Scheduler:remove(s)\n\t\tif true then return end",
		"disable_during_",
	],
	[
		"wrong S1 cycle",
		engine,
		"FrameEvent.PreAnimation, FrameEvent.PreSimulation",
		"FrameEvent.PreSimulation, FrameEvent.PreAnimation",
		"S1_client_",
	],
];
const log = [];
let failures = 0;
for (const [name, file, from, to, prefix, suite = "scheduler"] of rules) {
	const path = join(scratch, file);
	const original = readFileSync(path, "utf8");
	if (!original.includes(from)) throw new Error(`Mutant anchor missing: ${name}`);
	writeFileSync(path, original.replaceAll(from, to));
	writeFileSync(
		join(scratch, "probe.luau"),
		`local tests = require("./test/${suite}")\nlocal found = false\nfor name, fn in tests do\n if string.sub(name, 1, ${prefix.length}) == ${JSON.stringify(prefix)} then\n found = true\n local ok, err = pcall(fn)\n if ok then error("SURVIVED " .. name) end\n print("KILLED " .. name .. " " .. tostring(err))\n end\nend\nassert(found, "spec not found")\n`,
	);
	const run = spawnSync("lune", ["run", "probe"], { cwd: scratch, encoding: "utf8", timeout: 180000 });
	writeFileSync(path, original);
	if (readFileSync(path, "utf8") !== original) throw new Error("Restore failed");
	const passed = run.status === 0 && run.stdout.includes("KILLED ");
	log.push(`${passed ? "PASS" : "FAIL"} ${name}\n${run.stdout}${run.stderr}`);
	console.log(`${passed ? "PASS" : "FAIL"} ${name}`);
	if (!passed) {
		failures += 1;
	}
}
writeFileSync(join(scratch, "scheduler-mutants.log"), log.join("\n"));
console.log(`${rules.length - failures}/${rules.length} mutants killed. Scratch restored at ${scratch}`);
if (failures > 0) process.exitCode = 1;
