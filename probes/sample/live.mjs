import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { connect, text } from "../modules/mcp.mjs";
import { checkFlow, markerLines } from "./evidence.mjs";
import { playFlow } from "./flow.mjs";

const PLACE_ID = 101113691364993;
const UNIVERSE_ID = 10774131158;
const root = path.resolve(import.meta.dirname, "../..");
const results = path.join(root, "probes/sample/results");
const build = path.join(root, "build/sample-probe");
fs.mkdirSync(results, { recursive: true });
fs.mkdirSync(build, { recursive: true });
const [suffix = "r1"] = process.argv.slice(2);
if (!/^[a-z0-9-]+$/.test(suffix)) throw Error("Usage: node probes/sample/live.mjs [result-suffix]");
const label = `live-${suffix}`;
const runId = randomUUID();
const PRESSES = 3;
// The G1 test experience's GlobalEvent config is {"multiplier":2,"endsAt":0}.
const EXPECTED_GAIN = 2;
const excludeIds = new Set((process.env.G1_EXCLUDE_STUDIO_IDS || "").split(",").filter(Boolean));
const ps = (script) =>
	execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8" }).trim();
const quote = (value) => `'${value.replaceAll("'", "''")}'`;

// The sample's services become folders in a model that the open session moves into its own services.
const sample = JSON.parse(
	fs.readFileSync(path.join(root, "examples/sample-game/default.project.json"), "utf8").replace(/^﻿/, ""),
);
const importTree = { $className: "Folder" };
(function absolutize(node) {
	for (const [key, value] of Object.entries(node)) {
		if (key === "$path") node[key] = path.resolve(root, "examples/sample-game", value);
		else if (value && typeof value === "object") absolutize(value);
	}
})(sample.tree);
importTree.ReplicatedStorage = { ...sample.tree.ReplicatedStorage, $className: "Folder" };
importTree.ServerScriptService = { ...sample.tree.ServerScriptService, $className: "Folder" };
importTree.StarterPlayerScripts = {
	...sample.tree.StarterPlayer.StarterPlayerScripts,
	$className: "Folder",
};
const importProject = path.join(build, `${label}.project.json`);
const importModel = path.join(build, `${label}.rbxm`);
fs.writeFileSync(importProject, JSON.stringify({ name: "S1Import", tree: importTree }, null, 2));
execFileSync("rojo", ["build", importProject, "-o", importModel], { stdio: "inherit" });

const exe = ps(
	`Get-ChildItem -Path "$env:LOCALAPPDATA\\Roblox\\Versions\\*\\RobloxStudioBeta.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName`,
);
const logDir = path.join(process.env.LOCALAPPDATA, "Roblox/logs");
const launchArgs = `-task EditPlace -placeId ${PLACE_ID} -universeId ${UNIVERSE_ID}`;
const commandLine = `"${exe}" ${launchArgs}`;
const sessionFile = path.join(results, `${label}-session.json`);
if (fs.existsSync(sessionFile)) {
	const previous = JSON.parse(fs.readFileSync(sessionFile, "utf8"));
	if (ps(`@(Get-CimInstance Win32_Process -Filter 'ProcessId=${previous.pid}').Count`) !== "0")
		throw Error(`Previous owned PID ${previous.pid} has not exited`);
}

const identity = { label, runId, placeId: PLACE_ID, universeId: UNIVERSE_ID, errors: [] };
const save = () => fs.writeFileSync(sessionFile, JSON.stringify(identity, null, 2));
const mcp = await connect();
let studioId;
let pid;
let nativeLog;
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
const owns = () => {
	const line = ps(
		`(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}' -ErrorAction SilentlyContinue).CommandLine`,
	);
	return line.includes(`-placeId ${PLACE_ID}`) && line.toLowerCase().includes("robloxstudiobeta.exe");
};
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
	const baseline = new Set(JSON.parse(text(await call("list_roblox_studios"))).studios.map((studio) => studio.id));
	identity.freePhysicalMemoryKB = Number(ps("(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory"));
	identity.launchedAt = new Date().toISOString();
	pid = Number(
		ps(
			`$launch = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=${quote(commandLine)}}; if ($launch.ReturnValue -ne 0) { exit 1 }; $launch.ProcessId`,
		),
	);
	identity.pid = pid;
	save();
	console.log(`Launched ${label} PID ${pid} run ${runId}`);

	const logDeadline = Date.now() + 120000;
	while (!nativeLog && Date.now() < logDeadline) {
		if (!owns()) throw Error("Owned Studio exited during startup");
		nativeLog = fs
			.readdirSync(logDir)
			.filter((file) => file.includes("_Studio_") && file.endsWith(".log"))
			.filter((file) => fs.statSync(path.join(logDir, file)).mtimeMs >= Date.parse(identity.launchedAt) - 5000)
			.find((file) => fs.readFileSync(path.join(logDir, file), "utf8").includes(launchArgs));
		if (!nativeLog) await delay(2000);
	}
	if (!nativeLog) throw Error("Cannot identify own Studio log by its launch arguments");
	identity.nativeLog = nativeLog;

	const marker = `S1-IDENT-${pid}-${Date.now()}`;
	const rejected = new Set();
	const connectDeadline = Date.now() + 240000;
	while (!studioId && Date.now() < connectDeadline) {
		if (!owns()) throw Error("Owned Studio exited during startup");
		const studios = JSON.parse(text(await call("list_roblox_studios"))).studios;
		for (const studio of studios) {
			if (baseline.has(studio.id) || excludeIds.has(studio.id) || rejected.has(studio.id)) continue;
			// A print is the only call sent to a candidate before it is proven mine.
			await call("execute_luau", {
				datamodel_type: "Edit",
				code: `print("${marker}") return 1`,
				studio_id: studio.id,
			}).catch(() => undefined);
			await delay(2500);
			if (readLog().includes(marker)) studioId = studio.id;
			else rejected.add(studio.id);
			break;
		}
		if (!studioId) await delay(3000);
	}
	if (!studioId) throw Error("Own Studio did not connect to MCP. Check sign-in and Assistant MCP settings.");
	identity.studioId = studioId;
	identity.rejectedStudioIds = [...rejected];
	save();

	identity.version = await luau(
		"Edit",
		"return version() .. ' placeId=' .. game.PlaceId .. ' gameId=' .. game.GameId",
	);
	console.log(identity.version);
	if (!identity.version.includes(`placeId=${PLACE_ID} gameId=${UNIVERSE_ID}`))
		throw Error("Studio is not on the published test place");

	identity.insert = await luau(
		"Edit",
		`
local ok, model = pcall(function()
	return game:GetService("InsertService"):LoadLocalAsset(${JSON.stringify(importModel)})
end)
if not ok then return "load failed: " .. tostring(model) end
local moved = 0
for _, name in { "ReplicatedStorage", "ServerScriptService" } do
	for _, child in model[name]:GetChildren() do
		child.Parent = game:GetService(name)
		moved += 1
	end
end
for _, child in model.StarterPlayerScripts:GetChildren() do
	child.Parent = game:GetService("StarterPlayer").StarterPlayerScripts
	moved += 1
end
game:GetService("ServerStorage"):SetAttribute("CrucibleLiveData", true)
model:Destroy()
return "moved " .. moved
`,
	);
	console.log(identity.insert);
	if (!identity.insert.startsWith("moved"))
		throw Error(`The sample did not load into the session: ${identity.insert}`);
	save();

	await call("start_stop_play", { is_start: true });
	if (!(await waitFor(/S2 server booted/, 90000))) throw Error("Server did not boot");
	const configured = await waitFor(/\[Sample\.Wallet\] coin multiplier 2/, 60000);
	identity.configApplied = configured;
	if (!configured) identity.errors.push("The GlobalEvent multiplier of 2 never reached the server effect");
	const readings = await playFlow({ luau, call, waitFor }, { runId, presses: PRESSES, expectedGain: EXPECTED_GAIN });
	identity.screenshot = path.join(results, `${label}-client.png`);
	ps(
		`& powershell.exe -NoProfile -ExecutionPolicy Bypass -File ${quote(path.join(root, "probes/modules/window.ps1"))} -ProcId ${pid} -Out ${quote(identity.screenshot)}`,
	);
	identity.readings = readings;
	identity.flow = checkFlow(
		readings.map(({ coins, total, last }) => ({ coins, total, last })),
		EXPECTED_GAIN,
		{ fresh: false },
	);
	fs.writeFileSync(path.join(results, `${label}-hud.json`), JSON.stringify(readings, null, 2));
	console.log(`flow failures: ${JSON.stringify(identity.flow)}`);
	await call("start_stop_play", { is_start: false }, 60000);
	await delay(8000);
} catch (error) {
	identity.errors.push(String(error));
	console.error(String(error));
} finally {
	try {
		if (studioId) {
			await call("get_console_output")
				.then((result) =>
					fs.writeFileSync(path.join(results, `${label}-console.json`), JSON.stringify(result, null, 2)),
				)
				.catch(() => undefined);
		}
	} finally {
		if (nativeLog) {
			const native = readLog()
				.replace(/\r\n/g, "\n")
				.replace(/[ \t]+$/gm, "");
			const own = markerLines(native, runId);
			const faults = native
				.split("\n")
				.filter((line) => /DataStore|ConfigService|Crucible\]|API Services|HttpService/i.test(line));
			fs.writeFileSync(path.join(results, `${label}.log`), [...own, "--- faults", ...faults].join("\n") + "\n");
		}
		if (pid && owns()) {
			identity.closeMethod = ps(
				`$studio = Get-Process -Id ${pid}; $studio.CloseMainWindow() | Out-Null; if (-not $studio.WaitForExit(30000)) { Stop-Process -Id ${pid}; $studio.WaitForExit(10000) | Out-Null; 'Stop-Process' } else { 'CloseMainWindow' }`,
			);
			console.log(`Closed owned Studio PID ${pid} by ${identity.closeMethod}`);
		}
		if (pid)
			identity.processRemaining = ps(`@(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}').Count`) !== "0";
		save();
		mcp.close();
	}
}
if (identity.errors.length > 0 || identity.flow?.length > 0) process.exit(1);
