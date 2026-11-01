import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cliVersion, PLATFORMS, stage } from "./stage.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const [binaryDirectory, outputDirectory, notices] = process.argv.slice(2);
assert.ok(binaryDirectory && outputDirectory && notices, "Usage: node cli/npm/release.mjs <binaries> <out> <notices>");
const release = cliVersion();
if (process.env.GITHUB_EVENT_NAME === "push") {
	assert.equal(process.env.GITHUB_REF, `refs/tags/v${release}`, "release tag must match cli/Cargo.toml");
}
// npm refuses even a dry-run publish of a released version, and the Cargo version stays released until the next bump.
let version = release;
if (process.env.GITHUB_EVENT_NAME === "pull_request") {
	assert.ok(process.env.GITHUB_RUN_ID, "pull request staging needs GITHUB_RUN_ID");
	version = `${release}-pr.${process.env.GITHUB_RUN_ID}`;
}
assert.ok(readFileSync(notices, "utf8").includes("rojo "), "generated notices are required");
const output = resolve(outputDirectory);
const packages = stage(
	join(output, "packages"),
	Object.fromEntries(
		PLATFORMS.map(({ os, cpu }) => [
			`${os}-${cpu}`,
			resolve(binaryDirectory, `${os}-${cpu}`, os === "win32" ? "crucible.exe" : "crucible"),
		]),
	),
	resolve(notices),
	version,
);
const runtime = join(output, "packages", "runtime");
mkdirSync(runtime);
const source = join(root, "packages", "crucible");
const manifest = JSON.parse(readFileSync(join(source, "package.json"), "utf8"));
assert.equal(manifest.private, undefined);
assert.equal(manifest.version, release, "runtime and CLI versions must match");
manifest.version = version;
delete manifest.scripts;
delete manifest.devDependencies;
for (const entry of manifest.files) {
	if (entry !== "LICENSE") cpSync(join(source, entry), join(runtime, entry), { recursive: true });
}
copyFileSync(join(root, "LICENSE"), join(runtime, "LICENSE"));
writeFileSync(join(runtime, "package.json"), `${JSON.stringify(manifest, null, "\t")}\n`);
packages.push(runtime);
const tarballs = join(output, "tarballs");
rmSync(tarballs, { recursive: true, force: true });
mkdirSync(tarballs, { recursive: true });
for (const directory of packages) {
	const pkg = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
	assert.equal(pkg.version, version);
	assert.equal(pkg.license, "MIT");
	assert.equal(pkg.private, undefined);
	const packed = spawnSync("npm", ["pack", "--dry-run", "--json"], {
		cwd: directory,
		encoding: "utf8",
		shell: process.platform === "win32",
	});
	assert.equal(packed.status, 0, packed.stderr);
	const files = JSON.parse(packed.stdout)[0].files.map(({ path }) => path);
	assert.ok(files.includes("LICENSE"), `${pkg.name} must ship LICENSE`);
	assert.ok(
		files.every((file) => !file.endsWith(".tsbuildinfo") && !file.endsWith(".tgz")),
		`${pkg.name} must exclude build receipts and tarballs`,
	);
	if (pkg.name === "@trioforge/crucible") {
		for (const required of [
			"out/init.luau",
			"out/index.d.ts",
			"transformer/index.js",
			"server/package.json",
			"client/package.json",
			"emit/package.json",
		]) {
			assert.ok(files.includes(required), `runtime must pack ${required}`);
		}
	} else if (pkg.os) {
		assert.ok(files.includes("THIRD_PARTY_NOTICES"), `${pkg.name} must ship notices`);
	}
	process.stdout.write(`${pkg.name}@${pkg.version}\n${files.join("\n")}\n`);
	const result = spawnSync("npm", ["pack", "--pack-destination", tarballs], {
		cwd: directory,
		stdio: "inherit",
		shell: process.platform === "win32",
	});
	assert.equal(result.status, 0, `pack ${pkg.name}`);
}
