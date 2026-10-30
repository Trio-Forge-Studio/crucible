import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cliVersion, PLATFORMS, stage } from "./stage.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const [binaryDirectory, outputDirectory, notices] = process.argv.slice(2);
assert.ok(binaryDirectory && outputDirectory && notices, "Usage: node cli/npm/release.mjs <binaries> <out> <notices>");
const version = cliVersion();
if (process.env.GITHUB_EVENT_NAME === "push") {
	assert.equal(process.env.GITHUB_REF, `refs/tags/v${version}`, "release tag must match cli/Cargo.toml");
}
assert.ok(readFileSync(notices, "utf8").includes("Third-party notices"), "generated notices are required");
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
);
const runtime = join(output, "packages", "runtime");
mkdirSync(runtime);
const source = join(root, "packages", "crucible");
const manifest = JSON.parse(readFileSync(join(source, "package.json"), "utf8"));
assert.equal(manifest.private, undefined);
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
	if (pkg.name === "@trioforge/crucible") {
		for (const required of [
			"out/init.luau",
			"out/index.d.ts",
			"transformer/index.js",
			"server/package.json",
			"client/package.json",
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
