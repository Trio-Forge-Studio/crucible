import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { once } from "node:events";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";
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
	assert.ok(
		existsSync(join(cli, "..", "packages", "crucible", "out")),
		"Runtime output is missing. Run pnpm build from the repository root first.",
	);
	root = mkdtempSync(join(tmpdir(), "crucible-npm-"));
	const staged = stage(join(root, "staged"), supported ? { [host]: binary } : {});
	mkdirSync(join(root, "tarballs"));
	[shim, platform] = staged.map((directory) => pack(directory, join(root, "tarballs")));
});

after(() => {
	if (root) rmSync(root, { recursive: true, force: true });
});

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
		assert.ok(!files.some((file) => file.endsWith(".tsbuildinfo")), "runtime tarball excludes build metadata");
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
		rmSync(join(game, "build"), { recursive: true });
		const unformatted = run("npm", ["run", "check"], game);
		assert.notEqual(unformatted.status, 0, "the generated check actually rejects Vite+ formatting or lint errors");
		assert.match(`${unformatted.stdout}\n${unformatted.stderr}`, /format|lint|no-unused-vars/i);
		assert.ok(!existsSync(join(game, "build", "start.rbxl")), "vp check fails before crucible build runs");
	},
);

// The next version reuses this binary, so its shim edits the snapshot to stand in
// for a release whose template and docs changed.
const NEXT_VERSION_SHIM = `#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const args = process.argv.slice(2);
if (args[0] !== "__upgrade-snapshot") {
	require("./real.cjs");
} else {
	const result = spawnSync(process.execPath, [require.resolve("./real.cjs"), ...args], { encoding: "utf8" });
	const snapshot = JSON.parse(result.stdout);
	for (const part of snapshot.owned) {
		if (part.path === "tsconfig.json") {
			part.content = part.content.replace('"strict": true,', '"strict": true,\\n\\t\\t"noImplicitReturns": true,');
		} else if (part.path === "default.project.json") {
			part.content = part.content.replace('"tree"', '"servePort": 34873,\\n\\t"tree"');
		} else if (part.section === "## Crucible") {
			part.content += "\\n\\nThe next version adds this line.";
		}
	}
	snapshot.docs.logging = "changed";
	process.stdout.write(JSON.stringify(snapshot));
}
`;

test(
	"upgrade installs the next version and refreshes only untouched template files",
	{ skip: !supported && `no package for ${host}` },
	async () => {
		const [major, minor, patch] = cliVersion().split(".").map(Number);
		const next = `${major}.${minor}.${patch + 1}`;
		const platformName = platformPackageName(PLATFORMS.find(({ os, cpu }) => `${os}-${cpu}` === host));
		const runtimeDir = join(cli, "..", "packages", "crucible");
		const sources = {
			crucible: join(root, "staged", "crucible"),
			"@trioforge/crucible": join(root, "upgrade-runtime"),
			[platformName]: join(root, "staged", `crucible-cli-${host}`),
		};
		for (const entry of ["package.json", "out", "transformer", "server", "client", "emit"]) {
			cpSync(join(runtimeDir, entry), join(sources["@trioforge/crucible"], entry), {
				recursive: true,
				filter: (source) => !source.endsWith(".tsbuildinfo"),
			});
		}
		const tarballs = join(root, "upgrade-tarballs");
		mkdirSync(tarballs);
		const files = [];
		const packages = {};
		for (const version of [cliVersion(), next]) {
			for (const [name, source] of Object.entries(sources)) {
				const directory = join(root, `upgrade-${version}`, name);
				cpSync(source, directory, { recursive: true });
				const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
				manifest.version = version;
				if (name === "crucible") {
					manifest.optionalDependencies = { [platformName]: version };
					if (version === next) {
						renameSync(join(directory, "bin", "crucible.cjs"), join(directory, "bin", "real.cjs"));
						writeFileSync(join(directory, "bin", "crucible.cjs"), NEXT_VERSION_SHIM);
					}
				}
				writeFileSync(join(directory, "package.json"), JSON.stringify(manifest));
				files.push(pack(directory, tarballs));
				packages[name] = { ...packages[name], [version]: { manifest, file: files.length - 1 } };
			}
		}
		const registry = new Worker(new URL("./test-registry.mjs", import.meta.url), {
			workerData: { files, packages },
		});
		try {
			const [port] = await once(registry, "message");
			const creator = project("upgrade-creator", [shim, platform]);
			const game = join(root, "upgrade-game");
			const created = run("npx", ["crucible", "create", "upgrade-game", game, "--no-install"], creator);
			assert.equal(created.status, 0, created.stderr);
			writeFileSync(join(game, ".npmrc"), `registry=http://127.0.0.1:${port}/\n`);
			const installed = run("npm", ["install", "--no-audit", "--no-fund"], game);
			assert.equal(installed.status, 0, installed.stderr);

			const projectFile = join(game, "default.project.json");
			const edited = readFileSync(projectFile, "utf8").replace('"tree"', '"servePort": 34999,\n\t"tree"');
			writeFileSync(projectFile, edited);
			const agentsFile = join(game, "AGENTS.md");
			writeFileSync(agentsFile, `${readFileSync(agentsFile, "utf8")}\n## Notes\n\nKeep this.\n`);

			const downgrade = run("npx", ["crucible", "upgrade", "0.0.1"], game);
			assert.equal(downgrade.status, 2, downgrade.stderr);
			assert.match(downgrade.stderr, /--allow-downgrade/);

			const upgraded = run("npx", ["crucible", "upgrade"], game);
			assert.equal(upgraded.status, 0, upgraded.stderr);
			const report = upgraded.stdout;
			assert.match(report, new RegExp(`from ${cliVersion()} to ${next}\\.\n`));
			assert.match(
				report,
				/Refreshed from the new template:\n {2}tsconfig\.json\n {2}AGENTS\.md \(## Crucible\)\n\n/,
			);
			assert.match(report, /so they were left alone:\n {2}default\.project\.json\n[^ ]/);
			assert.match(report, /\n {2}npx crucible docs logging\n/);
			assert.doesNotMatch(report, /npx crucible docs oc\n/);

			const manifest = JSON.parse(readFileSync(join(game, "package.json"), "utf8"));
			assert.equal(manifest.devDependencies.crucible, next);
			assert.equal(manifest.dependencies["@trioforge/crucible"], next);
			const runtimeManifest = join(game, "node_modules", "@trioforge", "crucible", "package.json");
			assert.equal(JSON.parse(readFileSync(runtimeManifest, "utf8")).version, next);
			assert.equal(readFileSync(projectFile, "utf8"), edited, "an edited template file is left alone");
			assert.match(
				readFileSync(join(game, "tsconfig.json"), "utf8"),
				/"strict": true,\n\t\t"noImplicitReturns": true,/,
			);
			assert.match(
				readFileSync(agentsFile, "utf8"),
				/IDs, never credentials\.\n\nThe next version adds this line\.\n\n## Notes\n\nKeep this\.\n$/,
			);

			const checked = run("npm", ["run", "check"], game);
			assert.equal(checked.status, 0, `${checked.stdout}\n${checked.stderr}`);
			const again = run("npx", ["crucible", "upgrade", next], game);
			assert.equal(again.status, 0, again.stderr);
			assert.equal(again.stdout, `crucible is already at ${next}.\n`);
		} finally {
			await registry.terminate();
		}
	},
);
