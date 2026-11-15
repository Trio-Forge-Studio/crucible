import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(repo, "build/lune-mutants-"));
const mutants = [
	["service overrides", "place.luau", "if options.services and options.services[name] ~= nil then", "if false then"],
	["module cache", "place.luau", "if loaded[module] == nil then", "if true then"],
	[
		"compiled schema",
		"init.luau",
		"realm.data:profile(declaration.schema,",
		"realm.data:profile({ name = declaration.schema.name, defaults = {}, version = 0 },",
	],
	[
		"load hook",
		"init.luau",
		"return if options.onLoad then options.onLoad(player, value, reason) else value",
		"return value",
	],
];
try {
	fs.cpSync("packages/crucible/lune", path.join(scratch, "packages/crucible/lune"), { recursive: true });
	fs.cpSync("packages/crucible/out", path.join(scratch, "packages/crucible/out"), { recursive: true });
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules"),
		path.join(scratch, "packages/crucible/node_modules"),
		"junction",
	);
	fs.mkdirSync(path.join(scratch, "build"));
	fs.copyFileSync("build/sample-game.rbxl", path.join(scratch, "build/sample-game.rbxl"));
	fs.mkdirSync(path.join(scratch, "test"));
	const source = fs.readFileSync("test/lune-harness.luau", "utf8").replace("return {", "local specs = {");
	fs.writeFileSync(path.join(scratch, "test/verify.luau"), `${source}\nfor _, spec in specs do spec() end\n`);
	function run() {
		return spawnSync("lune", ["run", "test/verify"], { cwd: scratch, encoding: "utf8", timeout: 120_000 });
	}
	const baseline = run();
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	for (const [name, file, before, after] of mutants) {
		const target = path.join(scratch, "packages/crucible/lune", file);
		const original = fs.readFileSync(target, "utf8");
		assert(original.includes(before), `missing mutant anchor ${name}`);
		fs.writeFileSync(target, original.replace(before, after));
		const result = run();
		assert(!result.error, `${name} did not finish`);
		assert.notEqual(result.status, 0, `survived ${name}`);
		console.log(`killed ${name}`);
		fs.writeFileSync(target, original);
	}
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
