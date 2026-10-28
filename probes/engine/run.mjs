import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import readline from "node:readline";
import { setTimeout as delay } from "node:timers/promises";

const root = path.resolve(import.meta.dirname, "../..");
const build = path.join(root, "build/engine");
fs.mkdirSync(build, { recursive: true });
const [behavior = "Deferred", shutdown = "short"] = process.argv.slice(2);
if (!["Deferred", "Immediate"].includes(behavior) || !["short", "long"].includes(shutdown))
	throw Error("Usage: node probes/engine/run.mjs [Deferred|Immediate] [short|long]");
const label = `${behavior.toLowerCase()}-${shutdown}`;
const results = path.join(root, "probes/engine/results");
fs.mkdirSync(results, { recursive: true });
const ps = (script) =>
	execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8" }).trim();
const quote = (value) => `'${value.replaceAll("'", "''")}'`;
const project = JSON.parse(
	fs.readFileSync(path.join(root, "probes/engine/default.project.json"), "utf8").replace(/^\uFEFF/, ""),
);
project.tree.Workspace.$properties.SignalBehavior = behavior;
project.tree.Workspace.$attributes = { LongClose: shutdown === "long", ProbeSignalBehavior: behavior };
for (const service of ["ServerScriptService", "StarterPlayer"]) {
	const probe =
		service === "ServerScriptService"
			? project.tree[service].Probe
			: project.tree[service].StarterPlayerScripts.Probe;
	probe.$path = path.join(root, "probes/engine", probe.$path);
}
const projectFile = path.join(build, `${label}.project.json`);
const place = path.join(build, `s1-${label}.rbxl`);
fs.writeFileSync(projectFile, JSON.stringify(project, null, 2));
execFileSync("rojo", ["build", projectFile, "-o", place], { stdio: "inherit" });
const exe = ps(
	`Get-ChildItem -Path "$env:LOCALAPPDATA\\Roblox\\Versions\\*\\RobloxStudioBeta.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName`,
);
const commandLine = `"${exe}" "${place}"`;
for (const file of fs.readdirSync(results).filter((file) => file.endsWith("-session.json"))) {
	const previous = JSON.parse(fs.readFileSync(path.join(results, file), "utf8"));
	if (ps(`@(Get-CimInstance Win32_Process -Filter 'ProcessId=${previous.pid}').Count`) !== "0")
		throw Error(`Previous owned PID ${previous.pid} has not exited`);
}
const freePhysicalMemoryKB = Number(ps("(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory"));
console.log(`FreePhysicalMemory=${freePhysicalMemoryKB} KB`);
const pid = Number(
	ps(
		`$probeLaunch = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=${quote(commandLine)}}; if ($probeLaunch.ReturnValue -ne 0) { exit 1 }; $probeLaunch.ProcessId`,
	),
);
const launchedAt = new Date().toISOString();
const identity = { behavior, shutdown, pid, place, launchedAt, freePhysicalMemoryKB };
fs.writeFileSync(path.join(results, `${label}-session.json`), JSON.stringify(identity, null, 2));
console.log(`Launched ${label} PID ${pid}`);
const child = spawn("cmd.exe", ["/c", path.join(process.env.LOCALAPPDATA, "Roblox/mcp.bat")], { windowsHide: true });
child.stderr.on("data", () => {});
let nextId = 0;
const pending = new Map();
readline.createInterface({ input: child.stdout }).on("line", (line) => {
	let response;
	try {
		response = JSON.parse(line);
	} catch {
		return;
	}
	const resolve = pending.get(response.id);
	if (resolve) {
		pending.delete(response.id);
		resolve(response);
	}
});
async function request(method, params, timeout = 45000) {
	const id = ++nextId;
	const response = await new Promise((resolve, reject) => {
		const timer = setTimeout(() => {
			pending.delete(id);
			reject(Error(`${method} timeout`));
		}, timeout);
		pending.set(id, (response) => {
			clearTimeout(timer);
			resolve(response);
		});
		child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
	});
	if (response.error) throw Error(JSON.stringify(response.error));
	return response.result;
}
let studioId;
async function call(name, args = {}, timeout) {
	const result = await request(
		"tools/call",
		{ name, arguments: { ...args, ...(studioId ? { studio_id: studioId } : {}) } },
		timeout,
	);
	if (result.isError) throw Error(JSON.stringify(result));
	return result;
}
function text(result) {
	return result.content
		.filter((item) => item.type === "text")
		.map((item) => item.text)
		.join("\n");
}
function owns() {
	const line = ps(
		`(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}' -ErrorAction SilentlyContinue).CommandLine`,
	);
	return line.toLowerCase().includes(place.toLowerCase()) && line.toLowerCase().includes("robloxstudiobeta.exe");
}
try {
	await request("initialize", {
		protocolVersion: "2025-06-18",
		capabilities: {},
		clientInfo: { name: "crucible-engine-probes", version: "1" },
	});
	child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
	const deadline = Date.now() + 180000;
	while (!studioId && Date.now() < deadline) {
		if (!owns()) throw Error("Owned Studio exited during startup");
		const list = JSON.parse(text(await call("list_roblox_studios"))).studios;
		studioId = list.find((studio) => studio.name === path.basename(place))?.id;
		if (!studioId) await delay(2000);
	}
	if (!studioId)
		throw Error("Own Studio did not connect to MCP within 180 seconds. Check sign-in and Assistant MCP settings.");
	if (!owns()) throw Error("Studio process identity changed");
	identity.studioId = studioId;
	fs.writeFileSync(path.join(results, `${label}-session.json`), JSON.stringify(identity, null, 2));
	console.log(text(await call("get_studio_state")));
	identity.version = text(await call("execute_luau", { datamodel_type: "Edit", code: "return version()" }));
	const logDir = path.join(process.env.LOCALAPPDATA, "Roblox/logs");
	const logs = fs
		.readdirSync(logDir)
		.filter((file) => file.includes("_Studio_") && file.endsWith(".log"))
		.map((file) => ({ file, mtime: fs.statSync(path.join(logDir, file)).mtimeMs }))
		.filter((file) => file.mtime >= Date.parse(launchedAt) - 5000)
		.sort((a, b) => b.mtime - a.mtime);
	const logFile = logs.find(
		({ file }) =>
			fs.readFileSync(path.join(logDir, file), "utf8").includes(place.replaceAll("\\", "/")) ||
			fs.readFileSync(path.join(logDir, file), "utf8").includes(place),
	);
	if (!logFile) throw Error("Cannot identify own Studio log by exact place path");
	identity.nativeLog = logFile.file;
	await call("start_stop_play", { is_start: true });
	const readyDeadline = Date.now() + 60000;
	while (Date.now() < readyDeadline) {
		if (fs.readFileSync(path.join(logDir, logFile.file), "utf8").includes("|ready|")) break;
		await delay(1000);
	}
	let native = fs.readFileSync(path.join(logDir, logFile.file), "utf8");
	if (!native.includes("|ready|")) throw Error("Server probe did not reach ready marker");
	const clientDeadline = Date.now() + 60000;
	while (
		!fs.readFileSync(path.join(logDir, logFile.file), "utf8").includes("frame=35 Bind") &&
		Date.now() < clientDeadline
	)
		await delay(1000);
	if (!fs.readFileSync(path.join(logDir, logFile.file), "utf8").includes("frame=35 Bind"))
		throw Error("Client did not record 35 render frames");
	identity.stopRequestedAt = new Date().toISOString();
	console.log(`Stopping ${label} at ${identity.stopRequestedAt}`);
	await call("start_stop_play", { is_start: false }, 60000);
	await delay(shutdown === "long" ? 45000 : 8000);
	let state;
	const stopDeadline = Date.now() + 55000;
	do {
		state = text(await call("get_studio_state"));
		if (state.includes("Mode: Edit")) break;
		await delay(1000);
	} while (Date.now() < stopDeadline);
	if (!state.includes("Mode: Edit")) throw Error("Stop did not return to edit mode");
	identity.editConfirmedAt = new Date().toISOString();
	fs.writeFileSync(
		path.join(results, `${label}-console.json`),
		JSON.stringify(await call("get_console_output"), null, 2),
	);
	await delay(1000);
	native = fs.readFileSync(path.join(logDir, logFile.file), "utf8");
	const trace =
		native
			.split(/\r?\n/)
			.filter((line) => line.includes("S1|"))
			.join("\n") + "\n";
	fs.writeFileSync(path.join(results, `${label}.log`), trace);
	console.log(`Saved ${trace.split("\n").length - 1} trace lines. ${state}`);
} catch (error) {
	identity.error = String(error);
	throw error;
} finally {
	const logDir = path.join(process.env.LOCALAPPDATA, "Roblox/logs");
	for (const file of fs.readdirSync(logDir).filter((file) => file.includes("_Studio_") && file.endsWith(".log"))) {
		if (fs.statSync(path.join(logDir, file)).mtimeMs < Date.parse(launchedAt) - 5000) continue;
		const native = fs.readFileSync(path.join(logDir, file), "utf8");
		if (native.includes(place) || native.includes(place.replaceAll("\\", "/"))) {
			fs.writeFileSync(path.join(results, `${label}-native.log`), native);
			identity.nativeLog = file;
		}
	}
	if (owns()) {
		ps(
			`$probeProcess = Get-Process -Id ${pid}; $probeProcess.CloseMainWindow() | Out-Null; if (-not $probeProcess.WaitForExit(30000)) { Stop-Process -Id ${pid}; $probeProcess.WaitForExit(10000) | Out-Null }`,
		);
		identity.closed = true;
		console.log(`Closed owned Studio PID ${pid}`);
	}
	identity.processRemaining = ps(`@(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}').Count`) !== "0";
	fs.writeFileSync(path.join(results, `${label}-session.json`), JSON.stringify(identity, null, 2));
	child.stdin.end();
	child.kill();
}
