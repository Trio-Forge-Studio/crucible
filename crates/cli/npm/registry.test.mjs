import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { PLATFORMS, platformPackageName } from "./stage.mjs";
import { SHIM, WORKSPACE_PACKAGES, main, unused } from "./registry.mjs";

test("preflight rejects live and unpublished versions", () => {
	unused({}, SHIM, "1.2.3");
	assert.throws(() => unused({ versions: { "1.2.3": {} } }, "@trioforge/crucible", "1.2.3"), /already used/);
	assert.throws(() => unused({ time: { "1.2.3": "past" } }, SHIM, "1.2.3"), /already used/);
});

test("publication skips identical published tarballs, publishes the shim last, and stops at the first failure", async () => {
	const directory = mkdtempSync(join(tmpdir(), "crucible-release-"));
	const cwd = process.cwd();
	try {
		mkdirSync(join(directory, "package"));
		mkdirSync(join(directory, "packages"));
		// The publish job downloads the tarballs to `packages`, and npm reads `packages/<file>` as a GitHub repository.
		process.chdir(directory);
		const names = [SHIM, ...WORKSPACE_PACKAGES, ...PLATFORMS.map(platformPackageName)];
		const documents = {};
		const manifests = new Map();
		for (const [index, name] of names.entries()) {
			const tarball = join(directory, "packages", `${index}.tgz`);
			writeFileSync(join(directory, "package/package.json"), JSON.stringify({ name, version: "1.2.3-beta.1" }));
			execFileSync("tar", ["-czf", tarball, "-C", directory, "package"]);
			manifests.set(tarball, name);
			if (index === 2)
				documents[name] = {
					versions: {
						"1.2.3-beta.1": {
							dist: {
								integrity: `sha512-${createHash("sha512").update(readFileSync(tarball)).digest("base64")}`,
							},
						},
					},
				};
		}
		const published = [];
		const publish = (args) => {
			assert.equal(args[0], "publish");
			assert.ok(args.includes("--provenance"));
			assert.equal(args.at(-1), "next");
			assert.ok(manifests.has(args[1]), `${args[1]} is not an absolute tarball path`);
			published.push(manifests.get(args[1]));
		};
		await assert.rejects(
			main("preflight", "packages", async (name) => documents[name] ?? {}, publish),
			/already used/,
		);
		assert.deepEqual(published, []);
		await main("publish", "packages", async (name) => documents[name] ?? {}, publish);
		assert.equal(published.length, names.length - 1);
		assert.equal(published.at(-1), SHIM);
		assert.ok(!published.includes(names[2]));
		published.length = 0;
		const order = names.filter((name, index) => name !== SHIM && index !== 2).sort((x, y) => x.localeCompare(y));
		const failing = order.at(-1);
		await assert.rejects(
			main(
				"publish",
				"packages",
				async (name) => documents[name] ?? {},
				(args) => {
					if (manifests.get(args[1]) === failing) throw new Error("E409 Conflict");
					publish(args);
				},
			),
			/E409 Conflict/,
		);
		assert.deepEqual(published, order.slice(0, order.indexOf(failing)));
		documents[names[2]].versions["1.2.3-beta.1"].dist.integrity = "different";
		await assert.rejects(
			main("publish", "packages", async (name) => documents[name] ?? {}, publish),
			/artifact differs/,
		);
		await assert.rejects(
			main(
				"preflight",
				"packages",
				async () => {
					throw new Error("registry unavailable");
				},
				publish,
			),
			/registry unavailable/,
		);
	} finally {
		process.chdir(cwd);
		rmSync(directory, { recursive: true, force: true });
	}
});
