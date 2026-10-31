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
let upgradeRegistry;

function run(command, args, cwd, env = {}) {
	const line = [command, ...args.map((arg) => `"${arg}"`)].join(" ");
	return spawnSync(line, { cwd, encoding: "utf8", shell: true, env: { ...process.env, ...env } });
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

after(async () => {
	await upgradeRegistry?.then(({ registry }) => registry.terminate());
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

function nextPatch(version) {
	const [, major, minor, patch] = /^(\d+)\.(\d+)\.(\d+)/.exec(version);
	return `${major}.${minor}.${Number(patch) + 1}`;
}

test("the next upgrade version is a release above a prerelease CLI version", () => {
	assert.equal(nextPatch("0.1.0"), "0.1.1");
	assert.equal(nextPatch("0.2.0-alpha.1"), "0.2.1");
});

const OLD_TEMPLATE = {
	tsconfig: (text) => text.replace('"strict": true,', '"strict": true,\n\t\t"noImplicitReturns": true,'),
	project: (text) => text.replace('"tree"', '"servePort": 34873,\n\t"tree"'),
	agents: (section) => `${section}\n\nThe old version had this line.`,
};

// The next version reuses this binary, so its shim reports the next version and
// rewrites the old template it receives to stand in for an older release whose
// template and docs differed.
const NEXT_VERSION_SHIM = `#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const { readFileSync } = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "--version") {
	console.log("crucible " + require("../package.json").version + " (rojo 7.7.1)");
} else if (args[0] !== "__upgrade-apply") {
	require("./real.cjs");
} else {
	const OLD_TEMPLATE = {${Object.entries(OLD_TEMPLATE)
		.map(([key, edit]) => `${key}: ${edit}`)
		.join(", ")}};
	let input = readFileSync(0, "utf8");
	if (input) {
		const old = JSON.parse(input);
		for (const part of old.owned) {
			if (part.path === "tsconfig.json") part.content = OLD_TEMPLATE.tsconfig(part.content);
			if (part.path === "default.project.json") part.content = OLD_TEMPLATE.project(part.content);
			if (part.path === "LICENSE") part.content += "An older license.\\n";
			if (part.section === "## Crucible") part.content = OLD_TEMPLATE.agents(part.content);
		}
		old.owned.push({ path: "old-only.txt", section: null, content: "old\\n" });
		old.docs.logging = "changed";
		input = JSON.stringify(old);
	}
	const result = spawnSync(process.execPath, [require.resolve("./real.cjs"), ...args], {
		input,
		stdio: ["pipe", "inherit", "inherit"],
	});
	process.exit(result.status ?? 1);
}
`;

function startUpgradeRegistry() {
	const next = nextPatch(cliVersion());
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
	const registry = new Worker(new URL("./test-registry.mjs", import.meta.url), { workerData: { files, packages } });
	const creator = project("upgrade-creator", [shim, platform]);
	return once(registry, "message").then(([port]) => ({ registry, port, next, creator }));
}

function createGame(name, { port, creator }) {
	const game = join(root, name);
	const created = run("npx", ["crucible", "create", name, game, "--no-install"], creator);
	assert.equal(created.status, 0, created.stderr);
	writeFileSync(join(game, ".npmrc"), `registry=http://127.0.0.1:${port}/\n`);
	return game;
}

function installedVersion(game, name) {
	return JSON.parse(readFileSync(join(game, "node_modules", ...name.split("/"), "package.json"), "utf8")).version;
}

function assertPinned(game, version) {
	const manifest = JSON.parse(readFileSync(join(game, "package.json"), "utf8"));
	assert.equal(manifest.devDependencies.crucible, version);
	assert.equal(manifest.dependencies["@trioforge/crucible"], version);
	assert.equal(installedVersion(game, "crucible"), version);
	assert.equal(installedVersion(game, "@trioforge/crucible"), version);
}

test(
	"upgrade hands off to the next version, which refreshes only untouched template files",
	{ skip: !supported && `no package for ${host}` },
	async () => {
		upgradeRegistry ??= startUpgradeRegistry();
		const setup = await upgradeRegistry;
		const { next } = setup;
		const game = createGame("upgrade-game", setup);
		const installed = run("npm", ["install", "--no-audit", "--no-fund"], game);
		assert.equal(installed.status, 0, installed.stderr);

		const tsconfigFile = join(game, "tsconfig.json");
		const tsconfig = readFileSync(tsconfigFile, "utf8");
		writeFileSync(tsconfigFile, OLD_TEMPLATE.tsconfig(tsconfig));
		const projectFile = join(game, "default.project.json");
		const edited = readFileSync(projectFile, "utf8").replace('"tree"', '"servePort": 34999,\n\t"tree"');
		writeFileSync(projectFile, edited);
		const agentsFile = join(game, "AGENTS.md");
		const agents = readFileSync(agentsFile, "utf8").trimEnd();
		writeFileSync(agentsFile, `${OLD_TEMPLATE.agents(agents)}\n\n## Notes\n\nKeep this.\n`);
		rmSync(join(game, "LICENSE"));
		writeFileSync(join(game, "old-only.txt"), "old\n");

		const downgrade = run("npx", ["crucible", "upgrade", "0.0.1"], game);
		assert.equal(downgrade.status, 2, downgrade.stderr);
		assert.match(downgrade.stderr, /--allow-downgrade/);

		const upgraded = run("npx", ["crucible", "upgrade"], game);
		assert.equal(upgraded.status, 0, upgraded.stderr);
		const report = upgraded.stdout;
		assert.match(
			report,
			new RegExp(`^Upgraded crucible and @trioforge/crucible from ${cliVersion()} to ${next}\\.\n`),
		);
		assert.match(
			report,
			/Refreshed from the new template:\n {2}tsconfig\.json\n {2}AGENTS\.md \(## Crucible\)\n\n/,
		);
		assert.match(report, /this project edited them, so they were left alone:\n {2}default\.project\.json\n\n/);
		assert.match(report, /deleted in this project, so they were not recreated:\n {2}LICENSE\n\n/);
		assert.match(
			report,
			/no longer includes these\. Delete them if this game does not use them:\n {2}old-only\.txt\n/,
		);
		assert.match(report, /\n {2}npx crucible docs logging\n/);
		assert.doesNotMatch(report, /npx crucible docs oc\n/);
		assertPinned(game, next);
		assert.equal(readFileSync(tsconfigFile, "utf8"), tsconfig, "an untouched file takes the new template");
		assert.equal(readFileSync(projectFile, "utf8"), edited, "an edited template file is left alone");
		assert.equal(readFileSync(agentsFile, "utf8"), `${agents}\n\n## Notes\n\nKeep this.\n`);
		assert.ok(!existsSync(join(game, "LICENSE")), "a deleted file is not recreated");

		const checked = run("npm", ["run", "check"], game);
		assert.equal(checked.status, 0, `${checked.stdout}\n${checked.stderr}`);
		const again = run("npx", ["crucible", "upgrade", next], game);
		assert.equal(again.status, 0, again.stderr);
		assert.equal(again.stdout, `crucible is already at ${next}.\n`);

		const manifestFile = join(game, "package.json");
		const manifest = JSON.parse(readFileSync(manifestFile, "utf8"));
		manifest.scripts.preinstall = "exit 1";
		const failing = `${JSON.stringify(manifest, null, "\t")}\n`;
		writeFileSync(manifestFile, failing);
		for (const attempt of ["first", "retry"]) {
			const failed = run("npx", ["crucible", "upgrade", cliVersion(), "--allow-downgrade"], game);
			assert.equal(failed.status, 1, `${attempt}: ${failed.stdout}\n${failed.stderr}`);
			assert.equal(failed.stdout, "", `${attempt} does not report success`);
			assert.match(
				failed.stderr,
				new RegExp(
					`package\\.json is restored to crucible ${next} and @trioforge/crucible ${next}\\. Run \`npm install\``,
				),
			);
			assert.equal(readFileSync(manifestFile, "utf8"), failing, `${attempt} restores package.json byte for byte`);
		}
		delete manifest.scripts.preinstall;
		writeFileSync(manifestFile, `${JSON.stringify(manifest, null, "\t")}\n`);
		const recovered = run("npx", ["crucible", "upgrade", cliVersion(), "--allow-downgrade"], game);
		assert.equal(recovered.status, 0, recovered.stderr);
		assert.match(
			recovered.stdout,
			new RegExp(`(Upgraded|Reinstalled) crucible and @trioforge/crucible .*${cliVersion()}\\.\n`),
		);
		assertPinned(game, cliVersion());
	},
);

test(
	"upgrade reinstalls a pnpm game under CI despite its frozen lockfile default",
	{ skip: !supported && `no package for ${host}` },
	async () => {
		upgradeRegistry ??= startUpgradeRegistry();
		const setup = await upgradeRegistry;
		const game = createGame("pnpm-game", setup);
		const ci = { CI: "true" };
		const installed = run("pnpm", ["install"], game, ci);
		assert.equal(installed.status, 0, `${installed.stdout}\n${installed.stderr}`);
		assert.ok(existsSync(join(game, "pnpm-lock.yaml")));

		const upgraded = run("pnpm", ["exec", "crucible", "upgrade"], game, ci);
		assert.equal(upgraded.status, 0, `${upgraded.stdout}\n${upgraded.stderr}`);
		assert.match(upgraded.stdout, new RegExp(`from ${cliVersion()} to ${setup.next}\\.\n`));
		assert.match(upgraded.stdout, /Run `pnpm run check` next\.\n$/);
		assertPinned(game, setup.next);
		assert.match(
			readFileSync(join(game, "pnpm-lock.yaml"), "utf8"),
			new RegExp(`crucible@${setup.next.replaceAll(".", "\\.")}`),
		);
	},
);
