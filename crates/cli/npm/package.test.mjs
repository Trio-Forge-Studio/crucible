import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { once } from "node:events";
import {
	cpSync,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	renameSync,
	rmSync,
	writeFileSync,
} from "node:fs";
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
	join(cli, "../../target", "debug", process.platform === "win32" ? "crucible.exe" : "crucible");
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

function project(name, tarballs, flags = []) {
	const directory = join(root, name);
	mkdirSync(directory);
	writeFileSync(join(directory, "package.json"), JSON.stringify({ name, private: true }));
	const result = run("npm", ["install", "--save-dev", "--no-audit", "--no-fund", ...flags, ...tarballs], directory);
	assert.equal(result.status, 0, result.stderr);
	return directory;
}

before(() => {
	assert.ok(
		existsSync(join(cli, "../..", "packages", "crucible", "out")),
		"Runtime output is missing. Run pnpm build from the repository root first.",
	);
	root = mkdtempSync(join(tmpdir(), "crucible-npm-"));
	const notices = join(root, "THIRD_PARTY_NOTICES");
	writeFileSync(notices, "Third-party notices\nexample-crate 1.0.0 MIT\n");
	const staged = stage(join(root, "staged"), supported ? { [host]: binary } : {}, notices);
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
	assert.equal(manifest.license, "MIT");
	assert.equal(manifest.private, undefined);
	assert.deepEqual(
		manifest.optionalDependencies,
		Object.fromEntries(PLATFORMS.map((entry) => [platformPackageName(entry), cliVersion()])),
	);
});

test("npx crucible runs the installed platform binary", { skip: !supported && `no package for ${host}` }, () => {
	const app = project("app", [shim, platform]);
	const installed = join(app, "node_modules", `@trioforge/crucible-cli-${host}`);
	const schema = JSON.parse(
		readFileSync(join(app, "node_modules", "@trioforge/crucible-cli", "crucible.schema.json"), "utf8"),
	);
	assert.equal(schema.title, "crucible.toml", "the shim ships the crucible.toml schema for editors");
	assert.match(readFileSync(join(installed, "LICENSE"), "utf8"), /^MIT License/);
	assert.match(readFileSync(join(installed, "THIRD_PARTY_NOTICES"), "utf8"), /example-crate 1.0.0 MIT/);
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
		readFileSync(join(cli, "../..", "docs", "logging.md"), "utf8"),
		"docs logging prints the guide",
	);

	const unknown = run("npx", ["crucible", "oc", "call", "no-such.operation"], app);
	assert.equal(unknown.status, 2, "the shim forwards the binary's exit code");
	assert.equal(JSON.parse(unknown.stdout).error.kind, "usage");

	const scope = join(app, "node_modules", "@trioforge");
	for (const name of [`.crucible-cli-${host}-AbCd1234`, `crucible-cli-${host}_pacquet-stage_1_2_3`]) {
		mkdirSync(join(scope, name));
		writeFileSync(join(scope, name, "crucible.exe"), "an old binary");
	}
	mkdirSync(join(scope, "crucible-cli-notes"));
	const cleaned = run("npx", ["crucible", "--version"], app);
	assert.equal(cleaned.status, 0, cleaned.stderr);
	assert.deepEqual(
		readdirSync(scope).sort(),
		["crucible-cli", `crucible-cli-${host}`, "crucible-cli-notes"].sort(),
		"the shim deletes the old binaries npm and pnpm moved aside, and nothing else",
	);
});

test(
	"the Open Cloud Node example runs with a project devDependency",
	{ skip: !supported && `no package for ${host}` },
	() => {
		const app = project("node-example", [shim, platform]);
		const markdown = readFileSync(join(cli, "../..", "docs", "oc.md"), "utf8");
		const example = markdown.match(/^```javascript\n([\s\S]*?)^```$/m);
		assert.ok(example, "Open Cloud guide has a Node example");
		const script = join(app, "example.mjs");
		writeFileSync(script, example[1]);
		const result = spawnSync(process.execPath, [script], { cwd: app, encoding: "utf8" });
		assert.equal(result.status, 0, result.stderr);
		assert.ok(JSON.parse(result.stdout).length > 0, "example returns operations through the packed shim");
	},
);
test("release staging rejects a tag that differs from the Cargo version", () => {
	const result = spawnSync(process.execPath, [join(cli, "npm", "release.mjs"), "missing", "missing", "missing"], {
		encoding: "utf8",
		env: { ...process.env, GITHUB_EVENT_NAME: "push", GITHUB_REF: "refs/tags/v999.0.0" },
	});
	assert.notEqual(result.status, 0);
	assert.match(result.stderr, /release tag must match Cargo.toml/);
});

test("the shim names the missing platform package", () => {
	// The public registry holds published platform packages, so leave them out.
	const app = project("bare", [shim], ["--omit=optional"]);
	const result = run("npx", ["crucible", "--version"], app);
	assert.equal(result.status, 1);
	assert.match(result.stderr, new RegExp(`@trioforge/crucible-cli-${host} is not installed`));
});

test(
	"an installed create produces a standalone game whose check passes",
	{ skip: !supported && `no package for ${host}` },
	() => {
		const runtimeDir = join(cli, "../..", "packages", "crucible");
		const runtime = pack(runtimeDir, join(root, "tarballs"));
		const compilerTypes = pack(join(cli, "../..", "packages", "compiler-types"), join(root, "tarballs"));
		const contents = run("npm", ["pack", "--dry-run", "--json"], runtimeDir);
		assert.equal(contents.status, 0, contents.stderr);
		const files = JSON.parse(contents.stdout)[0].files.map(({ path }) => path);
		assert.ok(!files.some((file) => file.endsWith(".tsbuildinfo")), "runtime tarball excludes build metadata");
		for (const file of [
			"out/init.luau",
			"out/index.d.ts",
			"out/server/init.luau",
			"out/client/init.luau",
			"out/testing/init.luau",
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
		assert.equal(manifest.devDependencies["@trioforge/crucible-cli"], cliVersion());
		assert.equal(manifest.devDependencies["@trioforge/compiler-types"], cliVersion());
		for (const toolchain of ["roblox-ts", "typescript", "@rbxts/compiler-types"])
			assert.equal(manifest.devDependencies[toolchain], undefined, `the game needs no ${toolchain}`);
		assert.equal(manifest.scripts.check, "vp check && crucible build");
		manifest.dependencies["@trioforge/crucible"] = runtime;
		manifest.devDependencies["@trioforge/crucible-cli"] = shim;
		manifest.devDependencies["@trioforge/compiler-types"] = compilerTypes;
		manifest.devDependencies[platformPackageName(PLATFORMS.find(({ os, cpu }) => `${os}-${cpu}` === host))] =
			platform;
		writeFileSync(manifestPath, `${JSON.stringify(manifest, null, "\t")}\n`);
		const installed = run("npm", ["install", "--no-audit", "--no-fund"], game);
		assert.equal(installed.status, 0, installed.stderr);
		const toolchain = run("npm", ["ls", "typescript", "roblox-ts", "--all", "--json"], game);
		assert.deepEqual(
			JSON.parse(toolchain.stdout).dependencies ?? {},
			{},
			"the game installs no TypeScript or roblox-ts",
		);
		mkdirSync(join(game, "src", "shared"));
		writeFileSync(join(game, "src", "shared", "coins.ts"), "export const coins = 1;\n");
		const serverEntry = join(game, "src", "server", "main.server.ts");
		writeFileSync(
			serverEntry,
			`import { coins } from "../shared/coins";\n${readFileSync(serverEntry, "utf8")}\nprint(coins);\n`,
		);
		writeFileSync(
			join(game, "src", "client", "greeting.tsx"),
			'import Vide from "@rbxts/vide";\n\nexport function Greeting() {\n\treturn (\n\t\t<>\n\t\t\t<textlabel Text="Hello" />\n\t\t</>\n\t);\n}\n',
		);
		const checked = run("npm", ["run", "check"], game);
		assert.equal(checked.status, 0, `${checked.stdout}\n${checked.stderr}`);
		assert.match(checked.stdout, /vp check/);
		assert.ok(existsSync(join(game, "out", "shared", "coins.luau")), "the build emits the shared module");
		assert.match(
			readFileSync(join(game, "out", "client", "greeting.luau"), "utf8"),
			/Vide\.jsx\(Vide\.Fragment/,
			"a component that imports Vide only for JSX passes the check and builds",
		);
		assert.ok(
			readFileSync(join(game, "build", "start.rbxl"))
				.subarray(0, 8)
				.equals(Buffer.from("<roblox!")),
		);
		assert.ok(existsSync(join(game, "include", "RuntimeLib.lua")), "the build writes the runtime library");
		const listing = run("npm", ["ls", "--all", "--json"], game);
		assert.equal(listing.status, 0, listing.stderr);
		const pending = [JSON.parse(listing.stdout)];
		while (pending.length > 0) {
			for (const [name, dependency] of Object.entries(pending.pop().dependencies ?? {})) {
				if (["roblox-ts", "typescript"].includes(name)) {
					assert.deepEqual(dependency, {}, `${name} must not be installed`);
				}
				pending.push(dependency);
			}
		}
		const unusedNames =
			'import VideAlias from "@rbxts/vide";\nimport Vide from "@rbxts/vide";\n\nconst OtherUnused = 1;\n';
		const unusedModule = join(game, "src", "client", "unused.ts");
		const unusedComponent = join(game, "src", "client", "unused-component.tsx");
		writeFileSync(unusedModule, `${unusedNames}\nexport const used = 1;\n`);
		writeFileSync(unusedComponent, `${unusedNames}\nexport function Shown() {\n\treturn <frame />;\n}\n`);
		const unused = run("npm", ["run", "check"], game);
		const report = `${unused.stdout}\n${unused.stderr}`;
		assert.notEqual(unused.status, 0, report);
		const count = (pattern) => report.split(pattern).length - 1;
		assert.equal(
			count("'VideAlias' is imported but never used"),
			2,
			`an unused alias fails in both files\n${report}`,
		);
		assert.equal(
			count("'OtherUnused' is declared but never used"),
			2,
			`another unused name fails in both\n${report}`,
		);
		assert.equal(
			count("'Vide' is imported but never used"),
			1,
			`an unused Vide fails only outside .tsx\n${report}`,
		);
		rmSync(unusedModule);
		rmSync(unusedComponent);
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
	agents: (text) => `${text.trimEnd()}\n\nThe old version had this line.\n`,
};

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
	if (input && process.env.CRUCIBLE_TEST_OLD_TEMPLATE) {
		const old = JSON.parse(input);
		for (const part of old.owned) {
			if (part.path === "tsconfig.json") part.content = OLD_TEMPLATE.tsconfig(part.content);
			if (part.path === "default.project.json") part.content = OLD_TEMPLATE.project(part.content);
			if (part.path === "LICENSE") part.content += "An older license.\\n";
			if (part.path === ".gitattributes") part.content += "*.lua text\\n";
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
	upgradeRegistry ??= launchUpgradeRegistry();
	return upgradeRegistry;
}

function launchUpgradeRegistry() {
	const next = nextPatch(cliVersion());
	const platformName = platformPackageName(PLATFORMS.find(({ os, cpu }) => `${os}-${cpu}` === host));
	const runtimeDir = join(cli, "../..", "packages", "crucible");
	const sources = {
		"@trioforge/crucible-cli": join(root, "staged", "crucible"),
		"@trioforge/crucible": join(root, "upgrade-runtime"),
		"@trioforge/compiler-types": join(root, "upgrade-compiler-types"),
		[platformName]: join(root, "staged", `crucible-cli-${host}`),
	};
	for (const entry of ["package.json", "out"]) {
		cpSync(join(runtimeDir, entry), join(sources["@trioforge/crucible"], entry), {
			recursive: true,
			filter: (source) => !source.endsWith(".tsbuildinfo"),
		});
	}
	for (const entry of ["package.json", "types", "LICENSE"]) {
		cpSync(
			join(cli, "../..", "packages", "compiler-types", entry),
			join(sources["@trioforge/compiler-types"], entry),
			{
				recursive: true,
			},
		);
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
			if (name === "@trioforge/crucible" && version === cliVersion()) {
				delete manifest.peerDependencies["@rbxts/charm-sync"];
				delete manifest.peerDependencies["@rbxts/charm"];
			}
			if (name === "@trioforge/crucible-cli") {
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
	assert.equal(manifest.devDependencies["@trioforge/crucible-cli"], version);
	assert.equal(manifest.dependencies["@trioforge/crucible"], version);
	assert.equal(installedVersion(game, "@trioforge/crucible-cli"), version);
	assert.equal(installedVersion(game, "@trioforge/crucible"), version);
	// npm and pnpm move a folder aside when Windows refuses to delete the binary running in it.
	assert.deepEqual(
		readdirSync(join(game, "node_modules", "@trioforge")).filter(
			(name) => name.startsWith(".") || name.includes("_pacquet-stage_"),
		),
		[],
		"the install replaced the platform package in place",
	);
}

test(
	"upgrade hands off to the next version, which refreshes only untouched template files",
	{ skip: !supported && `no package for ${host}` },
	async () => {
		const setup = await startUpgradeRegistry();
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
		writeFileSync(agentsFile, `${OLD_TEMPLATE.agents(agents)}\n## Notes\n\nKeep this.\n`);
		const license = `${readFileSync(join(game, "LICENSE"), "utf8")}An older license.\n`;
		writeFileSync(join(game, "LICENSE"), license);
		rmSync(join(game, ".gitattributes"));
		writeFileSync(join(game, "old-only.txt"), "old\n");

		const downgrade = run("npx", ["crucible", "upgrade", "0.0.1"], game);
		assert.equal(downgrade.status, 2, downgrade.stderr);
		assert.match(downgrade.stderr, /--allow-downgrade/);

		const upgraded = run("npx", ["crucible", "upgrade"], game, { CRUCIBLE_TEST_OLD_TEMPLATE: "1" });
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
		assert.match(report, /deleted in this project, so they were not recreated:\n {2}\.gitattributes\n\n/);
		assert.match(report, /never rewrites them\. Review the change:\n {2}LICENSE\n\n/);
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
		assert.ok(!existsSync(join(game, ".gitattributes")), "a deleted file is not recreated");
		assert.equal(readFileSync(join(game, "LICENSE"), "utf8"), license, "the license is never rewritten");

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
					`package\\.json is restored to @trioforge/crucible-cli ${next} and @trioforge/crucible ${next}\\. To return node_modules to those versions, run \`npm install\``,
				),
			);
			assert.equal(readFileSync(manifestFile, "utf8"), failing, `${attempt} restores package.json byte for byte`);
			assert.equal(
				installedVersion(game, "@trioforge/crucible-cli"),
				cliVersion(),
				"npm runs preinstall after it reifies",
			);
		}
		delete manifest.scripts.preinstall;
		writeFileSync(manifestFile, `${JSON.stringify(manifest, null, "\t")}\n`);
		const recovered = run("npx", ["crucible", "upgrade", cliVersion(), "--allow-downgrade"], game);
		assert.equal(recovered.status, 0, recovered.stderr);
		assert.ok(
			recovered.stdout.startsWith(`Reinstalled crucible and @trioforge/crucible at ${cliVersion()}.\n`),
			recovered.stdout,
		);
		assert.match(
			recovered.stderr,
			new RegExp(`package\\.json pinned ${next}, which does not match the installed ${cliVersion()}`),
		);
		assert.match(
			recovered.stdout,
			/the old template is unknown, so they were left alone:\n {2}default\.project\.json\n/,
		);
		assertPinned(game, cliVersion());
	},
);

test("upgrade restores Charm Sync to a previous template and the built sync loads", { skip: !supported }, async () => {
	const setup = await startUpgradeRegistry();
	const game = createGame("upgrade-sync-game", setup);
	const file = join(game, "package.json");
	const manifest = JSON.parse(readFileSync(file, "utf8"));
	delete manifest.dependencies["@rbxts/charm-sync"];
	manifest.dependencies["@rbxts/charm"] = "0.10.0";
	writeFileSync(file, JSON.stringify(manifest, null, "\t") + "\n");
	mkdirSync(join(game, "src/shared"));
	writeFileSync(
		join(game, "src/shared/state.ts"),
		'import { replicated } from "@trioforge/crucible";\nexport const value = replicated<number>("Upgrade.Value", 7);\n',
	);
	const installed = run("npm", ["install", "--no-audit", "--no-fund"], game);
	assert.equal(installed.status, 0, installed.stderr);
	assert.ok(!existsSync(join(game, "node_modules/@rbxts/charm-sync")), "previous template has no sync package");
	const upgraded = run("npx", ["crucible", "upgrade", setup.next], game);
	assert.equal(upgraded.status, 0, upgraded.stdout + upgraded.stderr);
	assert.equal(JSON.parse(readFileSync(file, "utf8")).dependencies["@rbxts/charm-sync"], "0.4.1");
	assert.equal(JSON.parse(readFileSync(file, "utf8")).dependencies["@rbxts/charm"], "0.11.1");
	const built = run("npx", ["crucible", "build"], game);
	assert.equal(built.status, 0, built.stdout + built.stderr);
	// CI installs Lune only on Linux, where this load must run.
	const lune = run("lune", ["--version"], game).status === 0;
	if (!lune && !(process.env.CI && process.platform === "linux")) {
		return;
	}
	const loaded = run(
		"lune",
		["run", "test/fixtures/upgrade-sync.luau", join(game, "build/start.rbxl")],
		resolve(cli, "../.."),
	);
	assert.equal(loaded.status, 0, loaded.stdout + loaded.stderr);
	assert.match(loaded.stdout, /upgraded game loads sync/);
});

const LAST_ROBLOX_TS_RELEASE = "0.3.0";
const LAST_ROBLOX_TS_HOSTS = ["linux-x64", "win32-x64"];

test(
	`upgrade moves a game made by ${LAST_ROBLOX_TS_RELEASE} off roblox-ts, and its check passes`,
	{ skip: !LAST_ROBLOX_TS_HOSTS.includes(host) && `${LAST_ROBLOX_TS_RELEASE} has no package for ${host}` },
	async () => {
		const setup = await startUpgradeRegistry();
		const game = join(root, "roblox-ts-game");
		const cli = `@trioforge/crucible-cli@${LAST_ROBLOX_TS_RELEASE}`;
		const created = run("npx", ["--yes", cli, "create", "roblox-ts-game", game, "--no-install"], root);
		assert.equal(created.status, 0, created.stderr);
		const installed = run("npm", ["install", "--no-audit", "--no-fund"], game);
		assert.equal(installed.status, 0, installed.stderr);
		assert.ok(existsSync(join(game, "node_modules", "roblox-ts")), "the old template builds with roblox-ts");
		writeFileSync(join(game, ".npmrc"), `registry=http://127.0.0.1:${setup.port}/\n`);

		const upgraded = run("npx", ["crucible", "upgrade", setup.next], game);
		assert.equal(upgraded.status, 0, `${upgraded.stdout}\n${upgraded.stderr}`);
		assert.ok(
			upgraded.stdout.startsWith(
				`Upgraded crucible and @trioforge/crucible from ${LAST_ROBLOX_TS_RELEASE} to ${setup.next}.\n`,
			),
			upgraded.stdout,
		);
		assert.match(upgraded.stdout, /Refreshed from the new template:\n {2}tsconfig\.json\n/);
		assert.match(
			upgraded.stdout,
			/ {2}package\.json no longer lists roblox-ts, typescript, @rbxts\/compiler-types\n {2}ran `npm install`\n/,
		);
		assert.doesNotMatch(upgraded.stdout, /by hand/);
		const manifest = JSON.parse(readFileSync(join(game, "package.json"), "utf8"));
		assert.equal(manifest.devDependencies["@trioforge/compiler-types"], setup.next);
		const toolchain = run(
			"npm",
			["ls", "typescript", "roblox-ts", "@rbxts/compiler-types", "--all", "--json"],
			game,
		);
		assert.deepEqual(JSON.parse(toolchain.stdout).dependencies ?? {}, {}, "no roblox-ts toolchain is left");
		const checked = run("npm", ["run", "check"], game);
		assert.equal(checked.status, 0, `${checked.stdout}\n${checked.stderr}`);
		assert.ok(existsSync(join(game, "build", "start.rbxl")));
	},
);

test(
	"upgrade reinstalls a hoisted pnpm game under CI despite its frozen lockfile default",
	{ skip: !supported && `no package for ${host}` },
	async () => {
		const setup = await startUpgradeRegistry();
		const game = createGame("pnpm-game", setup);
		// The hoisted layout keeps the platform package at one path, so the install must replace the running binary's folder.
		writeFileSync(join(game, "pnpm-workspace.yaml"), "nodeLinker: hoisted\n");
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
			new RegExp(`crucible-cli@${setup.next.replaceAll(".", "\\.")}`),
		);
	},
);
