import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { connect, text } from "./mcp.mjs";

const PLACE_ID = 101113691364993;
const UNIVERSE_ID = 10774131158;
const root = path.resolve(import.meta.dirname, "../..");
const results = path.join(root, "probes/modules/results");
const build = path.join(root, "build/modules");
fs.mkdirSync(results, { recursive: true });
fs.mkdirSync(build, { recursive: true });
const suffix = process.argv[2] || "r3";
if (!/^[a-z0-9-]+$/.test(suffix)) throw Error("Usage: node probes/modules/published.mjs [result-suffix]");
const label = `published-${suffix}`;
const excludeIds = new Set((process.env.G1_EXCLUDE_STUDIO_IDS || "").split(",").filter(Boolean));
const ps = (script) =>
	execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8" }).trim();
const quote = (value) => `'${value.replaceAll("'", "''")}'`;

// Buy and OK sit at fixed window pixels in the maximized Studio window. The mouse tool measures
// from the viewport origin, which is the window origin plus (295, 274) at this layout.
const BUY = { window: [927, 618], client: [632, 344] };
const OK = { window: [927, 662], client: [632, 388] };
const isBlue = ([r, , b]) => b > 200 && b - r > 70;

const plays = [
	{ name: "hold", attributes: { ProbeMode: "hold", ProbeHolds: "5,60,120", ProbeOutdatedSeconds: 300 } },
	{ name: "leave-1", attributes: { ProbeMode: "leave", ProbeHolds: "1", ProbeOutdatedSeconds: 1 } },
	{ name: "join-late", attributes: { ProbeMode: "join", ProbeProcessAfter: 15, ProbeOutdatedSeconds: 1 } },
	{ name: "leave-2", attributes: { ProbeMode: "leave", ProbeHolds: "1", ProbeOutdatedSeconds: 1 } },
	{ name: "join-early", attributes: { ProbeMode: "join", ProbeProcessAfter: 0, ProbeOutdatedSeconds: 1 } },
];
const allAttributes = ["ProbeMode", "ProbeHolds", "ProbeOutdatedSeconds", "ProbeProcessAfter"];

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

const identity = { label, placeId: PLACE_ID, universeId: UNIVERSE_ID, plays: [], observations: {} };
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
const probeLines = () =>
	readLog()
		.split(/\r?\n/)
		.filter((line) => line.includes("G1|"));
const pixels = (points, out = "") =>
	Object.fromEntries(
		ps(
			`& powershell.exe -NoProfile -ExecutionPolicy Bypass -File ${quote(path.join(import.meta.dirname, "window.ps1"))} -ProcId ${pid} -Points ${quote(points.map((p) => p.join(",")).join(";"))}${out ? ` -Out ${quote(out)}` : ""}`,
		)
			.split(/\r?\n/)
			.filter((line) => line.includes("="))
			.map((line) => {
				const [key, value] = line.split("=");
				return [key, value.includes(",") ? value.split(",").map(Number) : value];
			}),
	);
async function waitFor(pattern, timeoutMs, since = 0) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const lines = probeLines();
		const index = lines.findIndex((line, i) => i >= since && pattern.test(line));
		if (index >= 0) return { line: lines[index], index };
		await delay(500);
	}
	return undefined;
}
async function click(target) {
	await call("user_mouse_input", {
		datamodel_type: "Client",
		actions: [
			{ action: "moveTo", x: target.client[0], y: target.client[1] },
			{ action: "wait", wait_time_ms: 300 },
			{ action: "mouseButtonClick", mouse_button: "left" },
		],
	});
}
async function waitBlue(target, timeoutMs) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const sample = pixels([target.window]);
		if (isBlue(sample[target.window.join(",")] ?? [0, 0, 0])) return true;
		await delay(1000);
	}
	return false;
}

async function confirm(play, ordinal, grantAfter) {
	const prompted = await waitFor(new RegExp(`prompting ordinal=${ordinal}$`), 60000, play.start);
	if (!prompted) return (play.needsHumanClick = `no prompt for ordinal ${ordinal}`);
	if (!(await waitBlue(BUY, 30000))) return (play.needsHumanClick = `Buy button not found for ordinal ${ordinal}`);
	await click(BUY);
	play.clicks.push({ ordinal, what: "Buy", at: new Date().toISOString() });
	const callback = await waitFor(new RegExp(`callback ordinal=${ordinal} `), 30000, play.start);
	if (!callback) return (play.needsHumanClick = `Buy click did not reach ProcessReceipt for ordinal ${ordinal}`);
	if (grantAfter === undefined) return true;
	const granted = await waitFor(
		new RegExp(`returning PurchaseGranted ordinal=${ordinal} `),
		(grantAfter + 90) * 1000,
		play.start,
	);
	if (!granted) return (play.needsHumanClick = `no grant for ordinal ${ordinal}`);
	if (await waitBlue(OK, 20000)) {
		await click(OK);
		play.clicks.push({ ordinal, what: "OK", at: new Date().toISOString() });
	} else play.dialogNote = `completion dialog not found for ordinal ${ordinal}`;
	return true;
}

async function runPlay(config) {
	const play = { name: config.name, clicks: [], attributes: config.attributes };
	identity.plays.push(play);
	await luau(
		"Edit",
		`for _, name in ${JSON.stringify(allAttributes).replace("[", "{").replace("]", "}")} do workspace:SetAttribute(name, nil) end
for name, value in ${luaTable(config.attributes)} do workspace:SetAttribute(name, value) end return 1`,
	);
	play.start = probeLines().length;
	play.startedAt = new Date().toISOString();
	await call("start_stop_play", { is_start: true });
	const ready = await waitFor(/\|ready\|/, 90000, play.start);
	if (!ready) throw Error(`Play ${config.name} did not reach the ready marker`);
	console.log(`play ${config.name} started`);
	if (config.name === "hold") {
		for (const [index, hold] of [5, 60, 120].entries()) {
			const ok = await confirm(play, index + 1, hold);
			if (ok !== true) break;
			await waitFor(new RegExp(`ordinal=${index + 1} promptToReturn=`), 60000, play.start);
		}
		await waitFor(/\|6done\|/, 400000, play.start);
		await delay(5000);
	} else if (config.name.startsWith("leave")) {
		await confirm(play, 1, undefined);
		await delay(100000);
	} else {
		await delay(config.attributes.ProbeProcessAfter ? 50000 : 40000);
	}
	await call("start_stop_play", { is_start: false }, 60000);
	const deadline = Date.now() + 60000;
	let state = "";
	while (Date.now() < deadline) {
		state = text(await call("get_studio_state"));
		if (state.includes("Mode: Edit")) break;
		await delay(1000);
	}
	if (!state.includes("Mode: Edit")) throw Error("Stop did not return to edit mode");
	play.end = probeLines().length;
	play.stoppedAt = new Date().toISOString();
	console.log(`play ${config.name} stopped`);
	await delay(3000);
}

function luaTable(object) {
	return `{ ${Object.entries(object)
		.map(([key, value]) => `${key} = ${typeof value === "string" ? JSON.stringify(value) : value}`)
		.join(", ")} }`;
}

try {
	const baseline = new Set(JSON.parse(text(await call("list_roblox_studios"))).studios.map((studio) => studio.id));
	identity.freePhysicalMemoryKB = Number(ps("(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory"));
	console.log(`FreePhysicalMemory=${identity.freePhysicalMemoryKB} KB`);
	identity.launchedAt = new Date().toISOString();
	pid = Number(
		ps(
			`$probeLaunch = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=${quote(commandLine)}}; if ($probeLaunch.ReturnValue -ne 0) { exit 1 }; $probeLaunch.ProcessId`,
		),
	);
	identity.pid = pid;
	save();
	console.log(`Launched PID ${pid}`);

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

	const marker = `G1-IDENT-${pid}-${Date.now()}`;
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
	if (!studioId)
		throw Error(
			"Own Studio did not connect to MCP. Set G1_EXCLUDE_STUDIO_IDS to known foreign ids and check sign-in.",
		);
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

	const source = fs.readFileSync(path.join(import.meta.dirname, "published.server.luau"), "utf8");
	if (source.includes("]==]")) throw Error("Probe source contains a long-bracket terminator");
	await luau(
		"Edit",
		`local old = game.ServerScriptService:FindFirstChild("G1Probe") if old then old:Destroy() end
local s = Instance.new("Script") s.Name = "G1Probe" s.Source = [==[${source}]==] s.Parent = game.ServerScriptService return #s.Source`,
	);

	for (const config of plays) await runPlay(config);

	identity.finished = true;
} catch (error) {
	identity.error = String(error);
	throw error;
} finally {
	try {
		if (studioId) {
			await luau(
				"Edit",
				`local old = game.ServerScriptService:FindFirstChild("G1Probe") if old then old:Destroy() end
for _, name in ${JSON.stringify(allAttributes).replace("[", "{").replace("]", "}")} do workspace:SetAttribute(name, nil) end return 1`,
			).catch(() => undefined);
			await call("get_console_output")
				.then((result) =>
					fs.writeFileSync(path.join(results, `${label}-console.json`), JSON.stringify(result, null, 2)),
				)
				.catch(() => undefined);
		}
	} finally {
		if (nativeLog) {
			const native = readLog();
			fs.writeFileSync(path.join(results, `${label}-native.log`), native);
			fs.writeFileSync(
				path.join(results, `${label}.log`),
				native
					.split(/\r?\n/)
					.filter((line) => line.includes("G1|"))
					.map((line) => `${line.slice(0, 24)} ${line.slice(line.indexOf("G1|"))}`)
					.join("\n") + "\n",
			);
		}
		if (pid && owns()) {
			identity.closeMethod = ps(
				`$probeProcess = Get-Process -Id ${pid}; $probeProcess.CloseMainWindow() | Out-Null; if (-not $probeProcess.WaitForExit(30000)) { Stop-Process -Id ${pid}; $probeProcess.WaitForExit(10000) | Out-Null; 'Stop-Process' } else { 'CloseMainWindow' }`,
			);
			console.log(`Closed owned Studio PID ${pid} by ${identity.closeMethod}`);
		}
		if (pid)
			identity.processRemaining = ps(`@(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}').Count`) !== "0";
		save();
		mcp.close();
	}
}
