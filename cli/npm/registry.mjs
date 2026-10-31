import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const burned = new Set([
	"0.0.0",
	"0.0.1",
	"0.3.0",
	"0.3.1",
	"0.3.2",
	"0.3.3",
	"0.3.4",
	"0.3.5",
	"0.4.0",
	"0.4.1",
	"9.0.0",
]);

export function unused(document, name, version) {
	assert.ok(!(name === "crucible" && burned.has(version)), `${name}@${version} is burned`);
	assert.ok(
		!Object.hasOwn(document.versions ?? {}, version) && !Object.hasOwn(document.time ?? {}, version),
		`${name}@${version} was already used`,
	);
}

export async function registry(name) {
	const response = await fetch(`https://registry.npmjs.org/${encodeURIComponent(name)}`, {
		signal: AbortSignal.timeout(30000),
	});
	if (response.status === 404) return {};
	assert.ok(response.ok, `Registry lookup failed for ${name}: ${response.status}`);
	const document = await response.json();
	assert.ok(document && typeof document === "object", "Invalid registry document");
	return document;
}

export async function main(
	mode,
	directory,
	lookup = registry,
	publish = (args) => execFileSync("npm", args, { stdio: "inherit" }),
) {
	assert.ok(
		["preflight", "publish"].includes(mode) && directory,
		"Usage: registry.mjs <preflight|publish> <tarballs>",
	);
	const packages = readdirSync(directory)
		.filter((file) => file.endsWith(".tgz"))
		.map((file) => {
			const path = join(directory, file);
			const { name, version } = JSON.parse(
				execFileSync("tar", ["-xOf", path, "package/package.json"], { encoding: "utf8" }),
			);
			return { path, name, version };
		})
		.sort((a, b) => Number(a.name === "crucible") - Number(b.name === "crucible") || a.name.localeCompare(b.name));
	assert.equal(packages.length, 5);
	assert.equal(new Set(packages.map(({ version }) => version)).size, 1);
	assert.deepEqual(
		new Set(packages.map(({ name }) => name)),
		new Set([
			"crucible",
			"@trioforge/crucible",
			"@trioforge/crucible-cli-linux-x64",
			"@trioforge/crucible-cli-win32-x64",
			"@trioforge/crucible-cli-darwin-arm64",
		]),
	);
	const documents = await Promise.all(packages.map(({ name }) => lookup(name)));
	for (const [index, { path, name, version }] of packages.entries()) {
		const document = documents[index];
		if (mode === "preflight") {
			unused(document, name, version);
			console.log(`Unused ${name}@${version}`);
			continue;
		}
		const existing = document.versions?.[version];
		if (existing) {
			const integrity = `sha512-${createHash("sha512").update(readFileSync(path)).digest("base64")}`;
			assert.equal(existing.dist?.integrity, integrity, `Published artifact differs for ${name}@${version}`);
			console.log(`Skip identical ${name}@${version}`);
			continue;
		}
		unused(document, name, version);
		publish([
			"publish",
			path,
			"--access",
			"public",
			"--provenance",
			"--ignore-scripts",
			"--tag",
			version.includes("-") ? "next" : "latest",
		]);
	}
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main(...process.argv.slice(2));
