import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { connect, text } from "../modules/mcp.mjs";
import { createPlaceName, selectStudio } from "../modules/session.mjs";
import { ownsProcess } from "../../test/studio/session.mjs";
import { aggregate, renderTable } from "./aggregate.mjs";

const root = path.resolve(import.meta.dirname, "../..");
const bench = path.join(root, "examples/bench");
const build = path.join(root, "build/bench");
const results = path.join(import.meta.dirname, "results");
const VARIANTS = ["rbxtsc", "compiler", "compiler-native"];
const WARMUP_FRAMES = 300;

const args = process.argv.slice(2);
const option = (name, fallback) => {
	const index = args.indexOf(`--${name}`);
	return index === -1 ? fallback : args[index + 1];
};
const reps = Number(option("reps", "5"));
const variants = option("variants", VARIANTS.join(",")).split(",");
if (!Number.isInteger(reps) || reps < 1 || variants.some((v) => !VARIANTS.includes(v)))
	throw Error(`Usage: node probes/bench/run.mjs [--reps n] [--variants ${VARIANTS.join(",")}] [--quiet percent]`);

const ps = (script) =>
	execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8" }).trim();
const quote = (value) => `'${value.replaceAll("'", "''")}'`;
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const cpuLoad = () => Number(ps("(Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average"));
const QUIET_PERCENT = Number(option("quiet", "25"));
const QUIET_WAIT_MS = 15 * 60000;

// Other work on the machine lowers the clock of the core Studio runs on, so a launch waits for ten quiet seconds.
async function waitForQuiet() {
	const started = Date.now();
	const window = [];
	while (Date.now() - started < QUIET_WAIT_MS) {
		window.push(cpuLoad());
		if (window.length > 5) window.shift();
		const average = window.reduce((a, b) => a + b, 0) / window.length;
		if (window.length === 5 && average < QUIET_PERCENT)
			return { quiet: true, average, waitedSeconds: (Date.now() - started) / 1000 };
		await delay(2000);
	}
	return { quiet: false, window, waitedSeconds: (Date.now() - started) / 1000 };
}

async function fetchLibMP() {
	const pin = JSON.parse(fs.readFileSync(path.join(import.meta.dirname, "libmp.json"), "utf8"));
	const file = path.join(root, "build/libmp", `LibMP-${pin.version}.luau`);
	if (!fs.existsSync(file) || sha256(fs.readFileSync(file)) !== pin.sha256) {
		const response = await fetch(pin.url);
		if (!response.ok) throw Error(`LibMP download failed: ${response.status}`);
		const bytes = Buffer.from(await response.arrayBuffer());
		if (sha256(bytes) !== pin.sha256) throw Error(`LibMP ${pin.version} does not match its pinned sha256`);
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, bytes);
	}
	return file;
}

function luauFiles(dir) {
	return fs
		.readdirSync(dir, { recursive: true })
		.filter((file) => file.endsWith(".luau"))
		.map((file) => path.join(dir, file));
}

function digest(dir) {
	const hash = createHash("sha256");
	for (const file of luauFiles(dir).sort()) hash.update(path.relative(dir, file)).update(fs.readFileSync(file));
	return hash.digest("hex").slice(0, 16);
}

const BUILDERS = {
	rbxtsc(out) {
		// rbxtsc compiles a scoped package name as a package unless told otherwise.
		const cli = path.join(root, "node_modules/roblox-ts/out/CLI/cli.js");
		execFileSync(process.execPath, [cli, "-p", bench, "--type", "game"], { stdio: "inherit" });
		fs.cpSync(path.join(bench, "out"), out, { recursive: true, filter: (from) => !from.endsWith(".tsbuildinfo") });
	},
	compiler(out) {
		const compile = path.join(root, "cli/target/release/examples/compile.exe");
		if (!fs.existsSync(compile))
			throw Error("Build the compiler first: cargo build --release -p crucible-compiler --example compile");
		execFileSync(compile, [path.join(bench, "tsconfig.ts7.json"), out], { stdio: "inherit" });
	},
	"compiler-native"(out) {
		fs.cpSync(path.join(build, "compiler"), out, { recursive: true });
		for (const file of luauFiles(path.join(out, "shared/workloads"))) {
			if (path.basename(file) !== "init.luau")
				fs.writeFileSync(file, `--!native\n${fs.readFileSync(file, "utf8")}`);
		}
	},
};

function buildVariants() {
	const built = {};
	for (const variant of VARIANTS.filter(
		(v) => variants.includes(v) || (v === "compiler" && variants.includes("compiler-native")),
	)) {
		const out = path.join(build, variant);
		fs.rmSync(out, { recursive: true, force: true });
		BUILDERS[variant](out);
		built[variant] = { out, digest: digest(out) };
		console.log(`built ${variant} ${built[variant].digest}`);
	}
	return built;
}

function placeFor(variant, out, libmp, label) {
	const project = {
		name: `Crucible bench ${variant}`,
		tree: {
			$className: "DataModel",
			ReplicatedStorage: {
				$className: "ReplicatedStorage",
				TS: { $path: path.join(out, "shared") },
				rbxts_include: { $path: path.join(root, "node_modules/roblox-ts/include") },
				LibMP: { $path: libmp },
			},
			ServerScriptService: { $className: "ServerScriptService", TS: { $path: path.join(out, "server") } },
		},
	};
	const places = path.join(build, "places");
	fs.mkdirSync(places, { recursive: true });
	const projectFile = path.join(places, `${label}.project.json`);
	const place = path.join(places, createPlaceName(label));
	fs.writeFileSync(projectFile, JSON.stringify(project, null, 2));
	execFileSync("rojo", ["build", projectFile, "-o", place], { stdio: "inherit" });
	return place;
}

const REPORT = `return game:GetService("ServerScriptService").TS.bench.BenchReport:Invoke()`;
const READY = `local s = game:GetService("ServerScriptService"):FindFirstChild("TS")
local b = s and s:FindFirstChild("bench")
local r = b and b:FindFirstChild("BenchReport")
return if r then r:Invoke() else "{}"`;

async function measure(variant, rep, built, libmp, exe) {
	const label = `${variant}-r${rep}`;
	const place = placeFor(variant, built[variant].out, libmp, label);
	const record = { variant, rep, digest: built[variant].digest, place: path.basename(place) };
	record.gate = await waitForQuiet();
	const pid = Number(
		ps(
			`$launch = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=${quote(`"${exe}" "${place}"`)}}; if ($launch.ReturnValue -ne 0) { exit 1 }; $launch.ProcessId`,
		),
	);
	const owns = () =>
		ownsProcess(
			ps(`(Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}' -ErrorAction SilentlyContinue).CommandLine`),
			place,
		);
	const mcp = await connect();
	let studioId;
	const call = async (name, toolArgs = {}, timeout) => {
		const result = await mcp.request(
			"tools/call",
			{ name, arguments: { ...toolArgs, ...(studioId ? { studio_id: studioId } : {}) } },
			timeout,
		);
		if (result.isError) throw Error(`${name}: ${text(result)}`);
		return result;
	};
	const luau = async (datamodel, code, timeout) =>
		text(await call("execute_luau", { datamodel_type: datamodel, code }, timeout));
	try {
		const deadline = Date.now() + 180000;
		while (!studioId && Date.now() < deadline) {
			if (!owns()) throw Error("Owned Studio exited during startup");
			studioId = selectStudio(JSON.parse(text(await call("list_roblox_studios"))).studios, path.basename(place));
			if (!studioId) await delay(2000);
		}
		if (!studioId) throw Error("Own Studio did not connect to MCP within 180 seconds");
		record.studio = await luau("Edit", "return version()");
		await call("start_stop_play", { is_start: true });
		const warm = Date.now() + 120000;
		const frames = (reply) => {
			try {
				return JSON.parse(reply).frames ?? 0;
			} catch {
				return 0;
			}
		};
		let ready;
		while (frames((ready = await luau("Server", READY).catch(String))) < WARMUP_FRAMES) {
			if (Date.now() > warm) throw Error(`The bench did not reach its warm-up frame count: ${ready}`);
			await delay(1000);
		}
		record.loadBefore = cpuLoad();
		record.capture = await luau(
			"Server",
			fs.readFileSync(path.join(import.meta.dirname, "capture.luau"), "utf8"),
			60000,
		);
		record.clock = JSON.parse(await luau("Server", REPORT));
		record.libmp = JSON.parse(
			await luau("Server", fs.readFileSync(path.join(import.meta.dirname, "summary.luau"), "utf8"), 120000),
		);
		record.loadAfter = cpuLoad();
		await call("start_stop_play", { is_start: false }, 60000);
	} catch (error) {
		record.error = String(error);
		record.console = await call("get_console_output").then(text).catch(String);
	} finally {
		mcp.close();
		if (owns())
			record.closeMethod = ps(
				`$studio = Get-Process -Id ${pid}; $studio.CloseMainWindow() | Out-Null; if (-not $studio.WaitForExit(30000)) { Stop-Process -Id ${pid}; $studio.WaitForExit(10000) | Out-Null; 'Stop-Process' } else { 'CloseMainWindow' }`,
			);
	}
	fs.writeFileSync(path.join(results, `${label}.json`), JSON.stringify(record, null, 2) + "\n");
	console.log(`${label}: ${record.error ?? "ok"} (load ${record.loadBefore}% -> ${record.loadAfter ?? "?"}%)`);
	return record;
}

fs.mkdirSync(results, { recursive: true });
const libmp = await fetchLibMP();
const built = buildVariants();
const exe = ps(
	`Get-ChildItem -Path "$env:LOCALAPPDATA\\Roblox\\Versions\\*\\RobloxStudioBeta.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName`,
);
const records = [];
for (let rep = 1; rep <= reps; rep++) {
	// Rotating the order spreads drift in machine load across every variant.
	const order = variants.map((_, i) => variants[(i + rep - 1) % variants.length]);
	for (const variant of order) records.push(await measure(variant, rep, built, libmp, exe));
}
const summary = aggregate(records);
fs.writeFileSync(path.join(results, "summary.json"), JSON.stringify(summary, null, 2) + "\n");
fs.writeFileSync(path.join(results, "summary.md"), renderTable(summary));
console.log(renderTable(summary));
if (records.some((r) => r.error)) process.exit(1);
