import { cpSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
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
		writeFileSync(resolve("test/scheduler-mutants.log"), log.join("\n"));
		throw new Error(`Mutant failed: ${name}`);
	}
}
writeFileSync(resolve("test/scheduler-mutants.log"), log.join("\n"));
console.log(`${rules.length}/${rules.length} mutants killed. Scratch restored at ${scratch}`);
