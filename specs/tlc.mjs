import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const TLA_VERSION = "1.7.4";
const TLA_URL = `https://github.com/tlaplus/tlaplus/releases/download/v${TLA_VERSION}/tla2tools.jar`;
const TLA_SHA256 = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88";

export const root = dirname(fileURLToPath(import.meta.url));
export const tools = join(root, ".tools");
const jar = join(tools, `tla2tools-${TLA_VERSION}.jar`);

function sha256(path) {
	return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export async function ensureJar() {
	if (existsSync(jar) && sha256(jar) === TLA_SHA256) return;
	mkdirSync(tools, { recursive: true });
	console.log(`downloading tla2tools ${TLA_VERSION}`);
	const response = await fetch(TLA_URL);
	if (!response.ok) throw new Error(`${TLA_URL}: HTTP ${response.status}`);
	const bytes = Buffer.from(await response.arrayBuffer());
	const actual = createHash("sha256").update(bytes).digest("hex");
	if (actual !== TLA_SHA256) throw new Error(`tla2tools.jar has SHA-256 ${actual}, expected ${TLA_SHA256}`);
	writeFileSync(jar, bytes);
}

export function findJava() {
	const exe = process.platform === "win32" ? "java.exe" : "java";
	const candidates = [process.env.JAVA, process.env.JAVA_HOME && join(process.env.JAVA_HOME, "bin", exe), exe];
	for (const candidate of candidates.filter(Boolean)) {
		const probe = spawnSync(candidate, ["-version"], { encoding: "utf8" });
		if (probe.status === 0) return candidate;
	}
	throw new Error("no Java found; install Java 11 or later, or set JAVA or JAVA_HOME");
}

const KEYWORDS = new Set([
	"CONSTANT",
	"CONSTANTS",
	"SPECIFICATION",
	"INIT",
	"NEXT",
	"INVARIANT",
	"INVARIANTS",
	"PROPERTY",
	"PROPERTIES",
	"SYMMETRY",
	"CONSTRAINT",
	"CONSTRAINTS",
	"VIEW",
	"CHECK_DEADLOCK",
]);

export function checked(text) {
	const found = { INVARIANT: [], PROPERTY: [] };
	let section = null;
	for (const token of text
		.replace(/\\\*.*$/gm, "")
		.split(/\s+/)
		.filter(Boolean)) {
		if (KEYWORDS.has(token)) section = token.replace(/S$/, "").replace(/IE$/, "Y");
		else if (section in found) found[section].push(token);
	}
	return found;
}

export function configs() {
	const found = [];
	for (const model of readdirSync(root).sort()) {
		const dir = join(root, model);
		if (model.startsWith(".") || !statSync(dir).isDirectory()) continue;
		const modules = readdirSync(dir).filter((name) => /^MC.*\.tla$/.test(name));
		if (modules.length !== 1) throw new Error(`${model}: expected one MC*.tla module, found ${modules.length}`);
		for (const cfg of readdirSync(dir)
			.filter((name) => name.endsWith(".cfg"))
			.sort()) {
			const text = readFileSync(join(dir, cfg), "utf8");
			const expect = /^\\\* expect: (pass|violation (\w+))\s*$/m.exec(text);
			if (!expect) throw new Error(`${model}/${cfg}: missing a "\\* expect:" line`);
			found.push({ model, dir, cfg, text, module: modules[0], expect: expect[2] ?? "pass" });
		}
	}
	return found;
}

function outcome(output) {
	if (output.includes("Model checking completed. No error has been found.")) return "pass";
	const violated = /Error: (?:Invariant|Action property) (\w+) is violated/.exec(output);
	if (violated) return violated[1];
	if (output.includes("Temporal properties were violated")) return "liveness";
	return "error";
}

export function runTlc(java, dir, cfg, module, metaName) {
	const meta = join(tools, "states", metaName);
	rmSync(meta, { recursive: true, force: true });
	const started = Date.now();
	const run = spawnSync(
		java,
		[
			"-XX:+UseParallelGC",
			"-Duser.language=en",
			"-Duser.country=US",
			"-cp",
			jar,
			"tlc2.TLC",
			"-workers",
			"auto",
			"-metadir",
			meta,
			"-config",
			cfg,
			module,
		],
		{ cwd: dir, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
	);
	rmSync(meta, { recursive: true, force: true });
	const output = `${run.stdout}${run.stderr}`;
	let distinct = "?";
	for (const match of output.matchAll(/([\d,]+) distinct states found/g)) distinct = match[1];
	return { result: outcome(output), distinct, seconds: (Date.now() - started) / 1000, output };
}
