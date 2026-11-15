import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const repo = process.cwd();
const src = "packages/crucible/src/";
const SS = src + "internal/network/server-session.ts";
const CS = src + "internal/network/client-session.ts";
const K = src + "internal/network/catalog.ts";
const SN = src + "internal/network/session.ts";
const EN = src + "emit/network.ts";
const specs = ["network-rates", "network-asks", "network-peers"];

// [name, file, before, after]. Each breaks one part of the per-player rate, and a spec must fail for it.
const mutants = [
	["the bucket is never checked", SS, "\t\tif (bucket.tokens >= 1) {", "\t\tif (true as boolean) {"],
	["a request spends nothing", SS, "\t\t\tbucket.tokens -= 1;\n", ""],
	[
		"a refill passes the capacity",
		SS,
		"bucket.tokens = math.min(capacity, bucket.tokens + (now - bucket.at) * rate);",
		"bucket.tokens = bucket.tokens + (now - bucket.at) * rate;",
	],
	[
		"a refill ignores the rate",
		SS,
		"bucket.tokens + (now - bucket.at) * rate);",
		"bucket.tokens + (now - bucket.at));",
	],
	["the bucket holds one request", SS, "const capacity = math.max(1, rate);", "const capacity = 1;"],
	["the refill time never moves", SS, "\t\t\tbucket.at = now;\n", ""],
	[
		"a drop is not counted",
		SS,
		"\t\tthis.limitedCounts.set(definition, (this.limitedCounts.get(definition) ?? 0) + 1);\n",
		"",
	],
	["every drop is reported", SS, "\t\tif (!bucket.reported) {", "\t\tif (true as boolean) {"],
	[
		"players share one bucket",
		SS,
		"\t\tlet bucket = peer.buckets.get(definition);",
		"\t\tlet bucket = this.order[0].buckets.get(definition);",
	],
	[
		"an event is never limited",
		SS,
		"\t\tif (!this.admit(peer, definition)) {",
		"\t\tif (!event && !this.admit(peer, definition)) {",
	],
	[
		"a malformed body skips the allowance",
		SS,
		"\t\tif (!this.admit(peer, definition)) {",
		"\t\tif (!pcall(openBody, packet as buffer, sidecar as Record<string, Instance>, header)[0]) return;\n\t\tif (!this.admit(peer, definition)) {",
	],
	[
		"a limited ask answers overloaded",
		SS,
		"return refuse(ErrorReason.Limited);",
		"return refuse(ErrorReason.Overloaded);",
	],
	[
		"the rate spends a token before the nonce check",
		SS,
		"\t\tconst nonce = event ? 0 : header.nonce;\n",
		"\t\tconst nonce = event ? 0 : header.nonce;\n\t\tif (!event && nonce <= peer.lastRequestNonce) this.admit(peer, definition);\n",
	],
	[
		"the client reads limited as unavailable",
		CS,
		'"overloaded", "invalid", "limited"];',
		'"overloaded", "invalid", "unavailable"];',
	],
	["a definition drops its rate", K, "\t\t\t\ttimeout,\n\t\t\t\trate,\n", "\t\t\t\ttimeout,\n"],
	["end keeps the allowances", SN, "\t\tpeer.buckets.clear();\n", ""],
	[
		"define accepts any rate",
		K,
		'typeIs(rate, "number") && rate > 0 && rate <= L.MAX_RATE_PER_SECOND',
		'typeIs(rate, "number")',
	],
	["toServer drops its rate", EN, "input: payload, rate })", "input: payload })"],
	["askServer drops its rate", EN, "\t\ttimeout,\n\t\trate,\n", "\t\ttimeout,\n"],
];

const missing = mutants.filter(([, file, before]) => {
	const text = fs.readFileSync(path.join(repo, file), "utf8");
	return text.split(before).length !== 2;
});
assert.deepEqual(
	missing.map(([name]) => name),
	[],
	"every mutant pattern occurs once in its file",
);
if (process.argv.includes("--check")) {
	console.log(`${mutants.length} patterns found`);
	process.exit(0);
}

function run(cwd, command, args) {
	const result = spawnSync(command, args, { cwd, encoding: "utf8", timeout: 600000 });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}

// A Lune run that printed no summary never ran its specs, so it decides nothing; run it again.
function specRun(cwd) {
	for (let attempt = 1; ; attempt++) {
		const result = run(cwd, "lune", ["run", "test", ...specs]);
		if (/\d+ passed, \d+ failed/.test(result.stdout)) return result;
		assert.ok(attempt < 3, `lune printed no summary three times\n${result.stdout}${result.stderr}`);
	}
}

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-rate-mutants-"));
try {
	for (const name of ["tsconfig.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	for (const name of [".lune", "test"])
		fs.cpSync(path.join(repo, name), path.join(scratch, name), { recursive: true });
	fs.cpSync(path.join(repo, "packages/crucible"), path.join(scratch, "packages/crucible"), {
		recursive: true,
		filter: (p) => !p.split(path.sep).includes("node_modules"),
	});
	fs.mkdirSync(path.join(scratch, "examples/sample-game"), { recursive: true });
	const links = ["node_modules", "packages/crucible/node_modules", "build", "examples/sample-game/node_modules"];
	for (const name of links) fs.symlinkSync(path.join(repo, name), path.join(scratch, name), "junction");
	const originals = new Map(mutants.map(([, file]) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const build = () => run(scratch, process.execPath, [path.join(repo, "test/lib/compile.mjs"), "packages/crucible"]);
	let built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	const baseline = specRun(scratch);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	let killed = 0;
	for (const [name, file, before, after] of mutants) {
		const original = originals.get(file);
		fs.writeFileSync(path.join(scratch, file), original.replace(before, after));
		built = build();
		assert.equal(built.status, 0, `${name} must compile\n${built.stdout}${built.stderr}`);
		const tested = specRun(scratch);
		assert.equal(tested.status, 1, `${name} survived\n${tested.stdout}${tested.stderr}`);
		const failure = (tested.stdout + tested.stderr).match(/FAIL (network[^\n]*)/);
		assert.ok(failure, `${name} must fail a spec assertion`);
		console.log(`KILLED ${name} | ${path.basename(file)} | first failure: ${failure[1]}`);
		killed++;
		fs.writeFileSync(path.join(scratch, file), original);
	}
	built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	console.log(`${killed}/${mutants.length} rate mutants killed; restored build 0`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
