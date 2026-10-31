import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { SHIM, main, unused } from "./registry.mjs";

test("preflight rejects live and unpublished versions", () => {
	unused({}, SHIM, "1.2.3");
	assert.throws(() => unused({ versions: { "1.2.3": {} } }, "@trioforge/crucible", "1.2.3"), /already used/);
	assert.throws(() => unused({ time: { "1.2.3": "past" } }, SHIM, "1.2.3"), /already used/);
});

test("partial staging skips identical published tarballs, stages the shim last, and reports stage failures", async () => {
	const directory = mkdtempSync(join(tmpdir(), "crucible-release-"));
	try {
		mkdirSync(join(directory, "package"));
		const names = [
			SHIM,
			"@trioforge/crucible",
			...["linux-x64", "win32-x64"].map((platform) => `@trioforge/crucible-cli-${platform}`),
		];
		const documents = {};
		const manifests = new Map();
		for (const [index, name] of names.entries()) {
			const tarball = join(directory, `${index}.tgz`);
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
			assert.deepEqual(args.slice(0, 2), ["stage", "publish"]);
			assert.ok(args.includes("--provenance"));
			assert.equal(args.at(-1), "next");
			published.push(manifests.get(args[2]));
		};
		await assert.rejects(
			main("preflight", directory, async (name) => documents[name] ?? {}, publish),
			/already used/,
		);
		assert.deepEqual(published, []);
		await main("stage", directory, async (name) => documents[name] ?? {}, publish);
		assert.equal(published.length, 3);
		assert.equal(published.at(-1), SHIM);
		assert.ok(!published.includes(names[2]));
		published.length = 0;
		await assert.rejects(
			main(
				"stage",
				directory,
				async (name) => documents[name] ?? {},
				(args) => {
					if (manifests.get(args[2]) === "@trioforge/crucible") throw new Error("E409 Conflict");
					publish(args);
				},
			),
			/Not staged by this run: @trioforge\/crucible@1\.2\.3-beta\.1\. If npmjs\.com > Staged Packages/,
		);
		assert.equal(published.length, 2);
		assert.equal(published.at(-1), SHIM);
		documents[names[2]].versions["1.2.3-beta.1"].dist.integrity = "different";
		await assert.rejects(
			main("stage", directory, async (name) => documents[name] ?? {}, publish),
			/artifact differs/,
		);
		await assert.rejects(
			main(
				"preflight",
				directory,
				async () => {
					throw new Error("registry unavailable");
				},
				publish,
			),
			/registry unavailable/,
		);
	} finally {
		rmSync(directory, { recursive: true, force: true });
	}
});
