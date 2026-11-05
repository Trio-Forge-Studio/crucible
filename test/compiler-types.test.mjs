import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { test } from "node:test";

const repo = path.resolve(import.meta.dirname, "..");
const tsrs = path.join(path.dirname(createRequire(import.meta.url).resolve("tsrs/package.json")), "bin/tsrs");
const slash = (file) => file.replaceAll("\\", "/");

function check(config, ...options) {
	const result = spawnSync(
		process.execPath,
		[tsrs, "-p", path.join(repo, config), "--pretty", "false", "--listFiles", ...options],
		{ encoding: "utf8" },
	);
	assert.equal(result.error, undefined, `tsrs runs on ${config}`);
	const lines = result.stdout.split(/\r?\n/).filter(Boolean);
	return {
		status: result.status,
		files: lines.filter((line) => !/\(\d+,\d+\): error TS\d+/.test(line)).map(slash),
		errors: lines.filter((line) => /error TS\d+/.test(line)),
		output: result.stdout + result.stderr,
	};
}

for (const [config, sources, imported] of [
	["packages/crucible/tsconfig.json", "packages/crucible/src/", undefined],
	["examples/sample-game/tsconfig.json", "examples/sample-game/src/", "packages/crucible/out/index.d.ts"],
]) {
	test(`${config} type-checks under TypeScript 7 with @trioforge/compiler-types`, () => {
		if (imported) assert.ok(fs.existsSync(path.join(repo, imported)), `${imported} exists; run pnpm build first`);
		const result = check(config);
		assert.deepEqual(result.errors, [], result.output);
		assert.equal(result.status, 0, result.output);
		const prefix = (relative) => slash(path.join(repo, relative));
		assert.ok(
			result.files.includes(prefix("packages/compiler-types/types/Iterable.d.ts")),
			"the program reads the forked Iterable declarations",
		);
		assert.ok(
			result.files.some((file) => file.startsWith(prefix(sources))),
			`the program checks ${sources}`,
		);
		if (imported)
			assert.ok(result.files.includes(prefix(imported)), `the program resolves the runtime through ${imported}`);
	});
}
import { typecheck } from "./lib/typecheck.mjs";

test("every published subpath resolves through exports with bundler resolution and no physical remaps", () => {
	const manifest = JSON.parse(fs.readFileSync(path.join(repo, "packages/crucible/package.json"), "utf8"));
	const source = Object.keys(manifest.exports)
		.map(
			(key, i) =>
				`import * as entry${i} from "@trioforge/crucible${key === "." ? "" : key.slice(1)}";\nexport const use${i} = entry${i};`,
		)
		.join("\n");
	for (const key of Object.keys(manifest.exports).filter((key) => key !== "."))
		assert.equal(fs.existsSync(path.join(repo, "packages/crucible", key)), false, key);
	const result = typecheck(new Map([[path.join(repo, "examples/sample-game/src/subpaths/index.ts"), source]]));
	assert.equal(result.status, 0, result.output);
});
