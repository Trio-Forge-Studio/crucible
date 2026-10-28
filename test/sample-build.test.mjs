import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";

const repo = path.resolve(import.meta.dirname, "..");

test("A4 transformer source changes invalidate the sample output and rebuild real Luau", () => {
	const scratch = path.join(repo, "build/sample-fingerprint", String(process.pid));
	const sample = path.join(scratch, "examples/sample-game");
	try {
		fs.mkdirSync(sample, { recursive: true });
		for (const entry of ["src", "package.json", "tsconfig.json", "default.project.json", "build.mjs"])
			fs.cpSync(path.join(repo, "examples/sample-game", entry), path.join(sample, entry), { recursive: true });
		fs.copyFileSync(path.join(repo, "tsconfig.rbxts.json"), path.join(scratch, "tsconfig.rbxts.json"));
		fs.symlinkSync(path.join(repo, "node_modules"), path.join(scratch, "node_modules"), "junction");
		fs.symlinkSync(
			path.join(repo, "examples/sample-game/node_modules"),
			path.join(sample, "node_modules"),
			"junction",
		);
		const transformer = path.join(scratch, "packages/crucible/transformer-src");
		fs.mkdirSync(transformer, { recursive: true });
		for (const name of fs.readdirSync(path.join(repo, "packages/crucible/transformer-src"))) {
			if (/\.(cjs|mjs)$/.test(name))
				fs.copyFileSync(
					path.join(repo, "packages/crucible/transformer-src", name),
					path.join(transformer, name),
				);
		}
		const build = () => {
			const result = spawnSync(process.execPath, [path.join(sample, "build.mjs")], { encoding: "utf8" });
			assert.equal(result.status, 0, result.stdout + result.stderr);
		};
		build();
		const receipt = path.join(sample, "out/transformer.sha256");
		const before = fs.readFileSync(receipt, "utf8");
		const entry = path.join(sample, "out/server/main.server.luau");
		assert.match(fs.readFileSync(entry, "utf8"), /crucible-path:ServerScriptService/);
		fs.writeFileSync(entry, "stale compiled output");
		fs.appendFileSync(path.join(transformer, "index.cjs"), "\n\n");
		build();
		assert.notEqual(fs.readFileSync(receipt, "utf8"), before);
		assert.match(fs.readFileSync(entry, "utf8"), /crucible-path:ServerScriptService/);
	} finally {
		fs.rmSync(scratch, { recursive: true, force: true });
	}
});
