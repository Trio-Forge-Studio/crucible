import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { connect, text } from "../modules/mcp.mjs";
import { createPlaceName, selectStudio } from "../modules/session.mjs";
import { ownsProcess } from "../../test/studio/session.mjs";
import { checkFlow, markerLines } from "./evidence.mjs";
import { playFlow } from "./flow.mjs";

const root = path.resolve(import.meta.dirname, "../..");
const results = path.join(root, "probes/sample/results");
const build = path.join(root, "build/sample-probe");
fs.mkdirSync(results, { recursive: true });
fs.mkdirSync(build, { recursive: true });
const [suffix = "r1"] = process.argv.slice(2);
if (!/^[a-z0-9-]+$/.test(suffix)) throw Error("Usage: node probes/sample/run.mjs [result-suffix]");
const label = `memory-${suffix}`;
const runId = randomUUID();
const PRESSES = 3;
const ps = (script) =>
	execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8" }).trim();
const quote = (value) => `'${value.replaceAll("'", "''")}'`;

const project = JSON.parse(
	fs.readFileSync(path.join(root, "examples/sample-game/default.project.json"), "utf8").replace(/^﻿/, ""),
);
project.tree.Workspace = { $className: "Workspace", $properties: { SignalBehavior: "Deferred" } };
(function absolutize(node) {
	for (const [key, value] of Object.entries(node)) {
		if (key === "$path") node[key] = path.resolve(root, "examples/sample-game", value);
		else if (value && typeof value === "object") absolutize(value);
	}
})(project.tree);
const place = path.join(build, createPlaceName(label));
fs.writeFileSync(path.join(build, `${label}.project.json`), JSON.stringify(project, null, 2));
execFileSync("rojo", ["build", path.join(build, `${label}.project.json`), "-o", place], { stdio: "inherit" });

const exe = ps(
	`Get-ChildItem -Path "$env:LOCALAPPDATA\\Roblox\\Versions\\*\\RobloxStudioBeta.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName`,
);
const sessionFile = path.join(results, `${label}-session.json`);
if (fs.existsSync(sessionFile)) {
	const previous = JSON.parse(fs.readFileSync(sessionFile, "utf8"));
	if (
		ownsProcess(
			ps(`(Get-CimInstance Win32_Process -Filter 'ProcessId=${previous.pid}').CommandLine`),
			previous.place,
		)
	)
		throw Error(`Previous owned PID ${previous.pid} has not exited`);
}
const freePhysicalMemoryKB = Number(ps("(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory"));
const pid = Number(
	ps(
		`$launch = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=${quote(`"${exe}" "${place}"`)}}; if ($launch.ReturnValue -ne 0) { exit 1 }; $launch.ProcessId`,
	),
);
const launchedAt = new Date().toISOString();
const identity = { label, runId, pid, place, launchedAt, freePhysicalMemoryKB, driverPid: process.pid };
const save = () => fs.writeFileSync(sessionFile, JSON.stringify(identity, null, 2));
save();
console.log(`Launched ${label} PID ${pid} run ${runId}`);

const mcp = await connect();
let studioId;
const call = async (name, args = {}, timeout) => {
	const result = await mcp.request(
		"tools/call",
		{ name, arguments: { ...args, ...(studioId ? { studio_id: studioId } : {}) } },
		timeout,
	);
	if (result.isError) throw Error(JSON.stringify(result));
	return result;
};
const luau = async (datamodel, code, timeout) =>
	text(await call("execute_luau", { datamodel_type: datamodel, code }, timeout));
const owns = () =>
	ownsProcess(
		ps(`(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}' -ErrorAction SilentlyContinue).CommandLine`),
		place,
	);
const logDir = path.join(process.env.LOCALAPPDATA, "Roblox/logs");
let nativeLog;
const readLog = () => fs.readFileSync(path.join(logDir, nativeLog), "utf8");
async function waitFor(pattern, timeoutMs) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		if (pattern.test(readLog())) return true;
		await delay(500);
	}
	return false;
}
try {
	await mcp.request("initialize", {
		protocolVersion: "2025-06-18",
		capabilities: {},
		clientInfo: { name: "crucible-sample-probe", version: "1" },
	});
	const deadline = Date.now() + 180000;
	while (!studioId && Date.now() < deadline) {
		if (!owns()) throw Error("Owned Studio exited during startup");
		const list = JSON.parse(text(await call("list_roblox_studios"))).studios;
		studioId = selectStudio(list, path.basename(place));
		if (!studioId) await delay(2000);
	}
	if (!studioId) throw Error("Own Studio did not connect to MCP within 180 seconds");
	if (!owns()) throw Error("Studio process identity changed");
	identity.studioId = studioId;
	identity.version = await luau("Edit", "return version()");
	const logs = fs
		.readdirSync(logDir)
		.filter((file) => file.includes("_Studio_") && file.endsWith(".log"))
		.map((file) => ({ file, mtime: fs.statSync(path.join(logDir, file)).mtimeMs }))
		.filter((file) => file.mtime >= Date.parse(launchedAt) - 5000)
		.sort((a, b) => b.mtime - a.mtime);
	nativeLog = logs.find(({ file }) => {
		const native = fs.readFileSync(path.join(logDir, file), "utf8");
		return native.includes(place) || native.includes(place.replaceAll("\\", "/"));
	})?.file;
	if (!nativeLog) throw Error("Cannot identify own Studio log by exact place path");
	identity.nativeLog = nativeLog;
	save();

	await call("start_stop_play", { is_start: true });
	if (!(await waitFor(/S2 server booted/, 90000))) throw Error("Server did not boot");
	const readings = await playFlow({ luau, call, waitFor }, { runId, presses: PRESSES, expectedGain: 1 });
	const steps = readings.map(({ coins, total, last }) => ({ coins, total, last }));
	identity.screenshot = path.join(results, `${label}-client.png`);
	ps(
		`& powershell.exe -NoProfile -ExecutionPolicy Bypass -File ${quote(path.join(root, "probes/modules/window.ps1"))} -ProcId ${pid} -Out ${quote(identity.screenshot)}`,
	);
	identity.flow = checkFlow(steps, 1);
	identity.readings = readings;
	fs.writeFileSync(path.join(results, `${label}-hud.json`), JSON.stringify(readings, null, 2));
	console.log(`flow failures: ${JSON.stringify(identity.flow)}`);
	await call("start_stop_play", { is_start: false }, 60000);
	await delay(8000);
	const state = text(await call("get_studio_state"));
	if (!state.includes("Mode: Edit")) throw Error("Stop did not return to edit mode");
	fs.writeFileSync(
		path.join(results, `${label}-console.json`),
		JSON.stringify(await call("get_console_output"), null, 2),
	);
} catch (error) {
	identity.error = String(error);
	throw error;
} finally {
	await delay(1000);
	if (nativeLog) {
		const native = readLog()
			.replace(/\r\n/g, "\n")
			.replace(/[ \t]+$/gm, "");
		fs.writeFileSync(path.join(results, `${label}.log`), markerLines(native, runId).join("\n") + "\n");
	}
	if (owns()) {
		identity.closeMethod = ps(
			`$studio = Get-Process -Id ${pid}; $studio.CloseMainWindow() | Out-Null; if (-not $studio.WaitForExit(30000)) { Stop-Process -Id ${pid}; $studio.WaitForExit(10000) | Out-Null; 'Stop-Process' } else { 'CloseMainWindow' }`,
		);
		console.log(`Closed owned Studio PID ${pid}`);
	}
	identity.processRemaining = ps(`@(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}').Count`) !== "0";
	save();
	mcp.close();
}
if (identity.flow.length > 0) process.exit(1);
