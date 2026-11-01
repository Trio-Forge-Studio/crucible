const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawn, spawnSync } = require("node:child_process");
const { setup } = require("./side-harness.cjs");

const repo = path.resolve(__dirname, "../../../..");
const game = setup({
	compilerOptions: { incremental: true },
	files: {
		"src/server/bad.ts": "export const bad = 1;\n",
		"src/server/other.ts": "export const value = 1;\n",
	},
});
const compile = () => {
	const result = spawnSync(
		process.execPath,
		[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", path.join(game, "tsconfig.json"), "--verbose"],
		{ cwd: game, encoding: "utf8" },
	);
	console.log(JSON.stringify({ status: result.status, output: result.stdout + result.stderr }));
	return result;
};
assert.equal(compile().status, 0);
assert.ok(fs.existsSync(path.join(game, "out/build.tsbuildinfo")));
fs.writeFileSync(
	path.join(game, "src/server/bad.ts"),
	'import { boot } from "@trioforge/crucible/server";\nconst glob = "src/server/**/*.ts";\nboot({ load: [glob] });\n',
);
const first = compile();
assert.equal(first.status, 1);
assert.match(first.stdout + first.stderr, /CRU006/);
fs.writeFileSync(path.join(game, "src/server/other.ts"), "export const value = 2;\n");
const second = compile();
assert.equal(second.status, 1, "probe 13: unchanged diagnostic must still fail the build");
assert.match(second.stdout + second.stderr, /CRU006/);
console.log("probe 13 passed");

if (process.argv.includes("--watch")) {
	fs.writeFileSync(path.join(game, "src/server/bad.ts"), "export const bad = 1;\n");
	const child = spawn(
		process.execPath,
		[
			path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"),
			"-p",
			path.join(game, "tsconfig.json"),
			"--watch",
			"--verbose",
		],
		{ cwd: game },
	);
	let output = "";
	let stage = 0;
	process.on("exit", () => child.kill());
	const timeout = setTimeout(() => {
		console.error("watch probe timed out");
		process.exitCode = 2;
		child.kill();
	}, 45000);
	const readOutput = (chunk) => {
		const text = chunk.toString();
		process.stdout.write(text);
		output += text;
		if (!output.includes("Watching for file changes.")) return;
		if (stage === 0) {
			assert.match(output, /Found 0 errors/);
			fs.writeFileSync(
				path.join(game, "src/server/bad.ts"),
				'import { boot } from "@trioforge/crucible/server";\nconst glob = "src/server/**/*.ts";\nboot({ load: [glob] });\n',
			);
		} else if (stage === 1) {
			assert.match(output, /CRU006/);
			fs.writeFileSync(path.join(game, "src/server/other.ts"), "export const value = 3;\n");
		} else {
			clearTimeout(timeout);
			child.kill();
			if (!/CRU006/.test(output) || !/Found 1 error/.test(output)) {
				console.error("probe 13 FAILED: watch forgot the unchanged CRU006 diagnostic");
				process.exitCode = 1;
			}
		}
		stage++;
		output = "";
	};
	const read = (chunk) => {
		try {
			readOutput(chunk);
		} catch (error) {
			console.error(error);
			process.exitCode = 1;
			child.kill();
		}
	};
	child.stdout.on("data", read);
	child.stderr.on("data", read);
	child.on("error", (error) => {
		console.error(error);
		process.exitCode = 2;
	});
	child.on("close", () => {
		clearTimeout(timeout);
		if (stage !== 3) process.exitCode = 2;
	});
}
