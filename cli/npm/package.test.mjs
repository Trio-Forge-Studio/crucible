import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { PLATFORMS, cliVersion, platformPackageName, stage } from "./stage.mjs";

const cli = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const host = `${process.platform}-${process.arch}`;
const binary =
	process.env.CRUCIBLE_BINARY ??
	join(cli, "target", "debug", process.platform === "win32" ? "crucible.exe" : "crucible");
const supported = PLATFORMS.some(({ os, cpu }) => `${os}-${cpu}` === host);

let root;
let shim;
let platform;

function run(command, args, cwd) {
	const line = [command, ...args.map((arg) => `"${arg}"`)].join(" ");
	return spawnSync(line, { cwd, encoding: "utf8", shell: true });
}

function pack(directory, destination) {
	const result = run("npm", ["pack", "--json", "--pack-destination", destination], directory);
	assert.equal(result.status, 0, result.stderr);
	return join(destination, JSON.parse(result.stdout)[0].filename);
}

function project(name, tarballs) {
	const directory = join(root, name);
	mkdirSync(directory);
	writeFileSync(join(directory, "package.json"), JSON.stringify({ name, private: true }));
	const result = run("npm", ["install", "--save-dev", "--no-audit", "--no-fund", ...tarballs], directory);
	assert.equal(result.status, 0, result.stderr);
	return directory;
}

before(() => {
	root = mkdtempSync(join(tmpdir(), "crucible-npm-"));
	const staged = stage(join(root, "staged"), supported ? { [host]: binary } : {});
	mkdirSync(join(root, "tarballs"));
	[shim, platform] = staged.map((directory) => pack(directory, join(root, "tarballs")));
});

after(() => rmSync(root, { recursive: true, force: true }));

test("the shim depends on every platform package at the CLI version", () => {
	const manifest = JSON.parse(readFileSync(join(root, "staged", "crucible", "package.json"), "utf8"));
	assert.equal(manifest.version, cliVersion());
	assert.deepEqual(
		manifest.optionalDependencies,
		Object.fromEntries(PLATFORMS.map((entry) => [platformPackageName(entry), cliVersion()])),
	);
});

test("npx crucible runs the installed platform binary", { skip: !supported && `no package for ${host}` }, () => {
	const app = project("app", [shim, platform]);
	const version = run("npx", ["crucible", "--version"], app);
	assert.equal(version.status, 0, version.stderr);
	assert.equal(version.stdout.trim(), `crucible ${cliVersion()} (rojo 7.7.1)`);

	const found = run("npx", ["crucible", "oc", "find", "asset"], app);
	assert.equal(found.status, 0, found.stderr);
	assert.ok(JSON.parse(found.stdout).length > 0, "oc find returns operations through the shim");

	const guide = run("npx", ["crucible", "docs", "logging"], app);
	assert.equal(guide.status, 0, guide.stderr);
	assert.equal(
		guide.stdout,
		readFileSync(join(cli, "..", "docs", "logging.md"), "utf8"),
		"docs logging prints the guide",
	);

	const unknown = run("npx", ["crucible", "oc", "call", "no-such.operation"], app);
	assert.equal(unknown.status, 2, "the shim forwards the binary's exit code");
	assert.equal(JSON.parse(unknown.stdout).error.kind, "usage");
});

test(
	"the Open Cloud Node example runs with a project devDependency",
	{ skip: !supported && `no package for ${host}` },
	() => {
		const app = project("node-example", [shim, platform]);
		const markdown = readFileSync(join(cli, "..", "docs", "oc.md"), "utf8");
		const example = markdown.match(/^```javascript\n([\s\S]*?)^```$/m);
		assert.ok(example, "Open Cloud guide has a Node example");
		const script = join(app, "example.mjs");
		writeFileSync(script, example[1]);
		const result = spawnSync(process.execPath, [script], { cwd: app, encoding: "utf8" });
		assert.equal(result.status, 0, result.stderr);
		assert.ok(JSON.parse(result.stdout).length > 0, "example returns operations through the packed shim");
	},
);

test("the shim names the missing platform package", () => {
	const app = project("bare", [shim]);
	const result = run("npx", ["crucible", "--version"], app);
	assert.equal(result.status, 1);
	assert.match(result.stderr, new RegExp(`@trioforge/crucible-cli-${host} is not installed`));
});

test(
	"an installed create produces a standalone game whose check passes",
	{ skip: !supported && `no package for ${host}` },
	() => {
		const runtimeDir = join(cli, "..", "packages", "crucible");
		const runtime = pack(runtimeDir, join(root, "tarballs"));
		const contents = run("npm", ["pack", "--dry-run", "--json"], runtimeDir);
		assert.equal(contents.status, 0, contents.stderr);
		const files = JSON.parse(contents.stdout)[0].files.map(({ path }) => path);
		for (const file of [
			"out/init.luau",
			"out/index.d.ts",
			"transformer/index.js",
			"server/package.json",
			"client/package.json",
		]) {
			assert.ok(files.includes(file), `runtime tarball includes ${file}`);
		}
		const app = project("creator", [shim, platform]);
		const game = join(root, "my-game");
		const created = run("npx", ["crucible", "create", "my-game", game, "--no-install"], app);
		assert.equal(created.status, 0, created.stderr);
		assert.equal(created.stdout.trim(), game);
		const manifestPath = join(game, "package.json");
		const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
		assert.equal(manifest.dependencies["@trioforge/crucible"], cliVersion());
		assert.equal(manifest.devDependencies.crucible, cliVersion());
		assert.equal(manifest.scripts.check, "vp check && crucible build");
		manifest.dependencies["@trioforge/crucible"] = runtime;
		manifest.devDependencies.crucible = shim;
		manifest.devDependencies[platformPackageName(PLATFORMS.find(({ os, cpu }) => `${os}-${cpu}` === host))] =
			platform;
		writeFileSync(manifestPath, `${JSON.stringify(manifest, null, "\t")}\n`);
		const installed = run("npm", ["install", "--no-audit", "--no-fund"], game);
		assert.equal(installed.status, 0, installed.stderr);
		const checked = run("npm", ["run", "check"], game);
		assert.equal(checked.status, 0, `${checked.stdout}\n${checked.stderr}`);
		assert.match(checked.stdout, /vp check/);
		assert.ok(
			readFileSync(join(game, "build", "start.rbxl"))
				.subarray(0, 8)
				.equals(Buffer.from("<roblox!")),
		);
		writeFileSync(join(game, "src", "server", "main.server.ts"), "const x=1;\n");
		const unformatted = run("npm", ["run", "check"], game);
		assert.notEqual(unformatted.status, 0, "the generated check actually rejects Vite+ formatting or lint errors");
	},
);
