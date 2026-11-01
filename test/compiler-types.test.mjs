import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { test } from "node:test";

const repo = path.resolve(import.meta.dirname, "..");
const tsrs = path.join(path.dirname(createRequire(import.meta.url).resolve("tsrs/package.json")), "bin/tsrs");
const slash = (file) => file.replaceAll("\\", "/");

function check(project, ...options) {
	const result = spawnSync(
		process.execPath,
		[tsrs, "-p", path.join(repo, project, "tsconfig.ts7.json"), "--pretty", "false", "--listFiles", ...options],
		{ encoding: "utf8" },
	);
	assert.equal(result.error, undefined, `tsrs runs on ${project}`);
	const lines = result.stdout.split(/\r?\n/).filter(Boolean);
	return {
		status: result.status,
		files: lines.filter((line) => !/\(\d+,\d+\): error TS\d+/.test(line)).map(slash),
		errors: lines.filter((line) => /error TS\d+/.test(line)),
		output: result.stdout + result.stderr,
	};
}

for (const [project, sources, imported] of [
	["packages/crucible", "packages/crucible/src/", undefined],
	["examples/sample-game", "examples/sample-game/src/", "packages/crucible/out/index.d.ts"],
]) {
	test(`${project} type-checks under TypeScript 7 with @trioforge/compiler-types`, () => {
		if (imported) assert.ok(fs.existsSync(path.join(repo, imported)), `${imported} exists; run pnpm build first`);
		const result = check(project);
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

test("@rbxts/compiler-types fails the same check with TS2802", () => {
	const result = check("packages/crucible", "--types", "@rbxts/compiler-types,@rbxts/types");
	assert.notEqual(result.status, 0, "the upstream types fail under TypeScript 7");
	assert.ok(
		result.errors.some((line) => line.includes("error TS2802")),
		`the upstream Iterable reports TS2802:\n${result.output}`,
	);
});
