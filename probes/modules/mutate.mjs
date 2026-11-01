import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
const root = path.resolve(import.meta.dirname, "../..");
const source = fs.readFileSync(new URL("results/deferred-short.log", import.meta.url), "utf8");
const destination = path.join(root, "build/modules/mutated.log");
fs.mkdirSync(path.dirname(destination), { recursive: true });
const cases = [
	["missing-slot", "part=nil keep=intact", "part=unexpected keep=intact"],
	["catalog", "length=66559 intact=true", "length=66558 intact=false"],
	["order", "|3order|start=100", "|3order|start=99"],
	["flush", "|4|defer-PreRender", "|4|wrong-placement"],
	["kick", "|3kick|handler return", "|3kick|missing-return"],
	["config", "|6fetch|ok=false result=ConfigService: Failed to load config", "|6fetch|unexpected"],
];
let failed = false;
for (const [name, before, after] of cases) {
	if (!source.includes(before)) throw Error(`Mutation ${name} did not match`);
	fs.writeFileSync(destination, source.replaceAll(before, after));
	const run = spawnSync(process.execPath, ["--test", "probes/modules/verification.test.mjs"], {
		cwd: root,
		encoding: "utf8",
		env: { ...process.env, S0_TRACE: destination },
	});
	if (run.error || run.status === null) throw run.error || Error("Verifier did not exit");
	console.log(`${name}: verifier exit ${run.status}, ${run.status === 1 ? "rejected" : "MISSED"}`);
	if (run.status !== 1) failed = true;
}
fs.rmSync(destination);
const streaming = fs.readFileSync(new URL("results/deferred-short-r2.log", import.meta.url), "utf8");
for (const [name, before, after] of [
	["stream-visibility", "visible=Workspace.PreviouslyVisiblePart", "initial-visibility-unmeasured"],
	["stream-removal", "previously visible part removed locally", "stream-out-unmeasured"],
	[
		"stream-slot",
		"received local=nil part=PreviouslyVisiblePart keep=intact number=42",
		"received local=nil part=unexpected keep=intact number=42",
	],
]) {
	if (!streaming.includes(before)) throw Error(`Mutation ${name} did not match`);
	fs.writeFileSync(destination, streaming.replaceAll(before, after));
	const run = spawnSync(process.execPath, ["--test", "probes/modules/verification.test.mjs"], {
		cwd: root,
		encoding: "utf8",
		env: { ...process.env, S0_STREAM_TRACE: destination },
	});
	if (run.error || run.status === null) throw run.error || Error("Verifier did not exit");
	console.log(`${name}: verifier exit ${run.status}, ${run.status === 1 ? "rejected" : "MISSED"}`);
	if (run.status !== 1) failed = true;
}
fs.rmSync(destination);
if (failed) process.exitCode = 1;
