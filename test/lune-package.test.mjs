import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

const repo = path.resolve(import.meta.dirname, "..");
function run(command, args, cwd) {
	if (command === "pnpm" && process.env.npm_execpath) {
		command = process.env.npm_execpath;
		if (/\.[cm]?js$/.test(command)) {
			args = [command, ...args];
			command = process.execPath;
		}
	}
	const result = spawnSync(command, args, { cwd, encoding: "utf8" });
	assert.equal(result.status, 0, `${command} ${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
	return result.stdout;
}

test("packed Lune entry runs in a pnpm-installed game", { timeout: 180_000 }, () => {
	mkdirSync(path.join(repo, "build"), { recursive: true });
	const game = mkdtempSync(path.join(repo, "build/lune-package-"));
	try {
		run("pnpm", ["pack", "--pack-destination", game], path.join(repo, "packages/crucible"));
		writeFileSync(
			path.join(game, "package.json"),
			JSON.stringify({
				private: true,
				dependencies: {
					"@trioforge/crucible": `file:./trioforge-crucible-${JSON.parse(readFileSync(path.join(repo, "packages/crucible/package.json"), "utf8")).version}.tgz`,
					"@rbxts/charm": "0.11.1",
					"@rbxts/charm-sync": "0.4.1",
				},
			}),
		);
		run("pnpm", ["install", "--ignore-scripts"], game);
		const installed = path.join(game, "node_modules/@trioforge/crucible");
		assert.match(realpathSync(installed), /[\\/]\.pnpm[\\/]/);
		const manifest = JSON.parse(readFileSync(path.join(installed, "package.json"), "utf8"));
		assert.equal(manifest.exports["./lune"], "./lune/init.luau");
		cpSync(path.join(repo, "build/sample-game.rbxl"), path.join(game, "sample.rbxl"));
		cpSync(path.join(repo, "examples/sample-game/out"), path.join(game, "out"), { recursive: true });
		cpSync(path.join(repo, "examples/sample-game/include"), path.join(game, "include"), { recursive: true });
		cpSync(path.join(repo, "examples/sample-game/default.project.json"), path.join(game, "default.project.json"));
		writeFileSync(path.join(game, "sourcemap.json"), run("rojo", ["sourcemap", "default.project.json"], game));
		const specs = readFileSync(path.join(repo, "test/lune-harness.luau"), "utf8")
			.replaceAll("../packages/crucible/lune", "./node_modules/@trioforge/crucible/lune")
			.replaceAll("build/sample-game.rbxl", "sample.rbxl")
			.replace("return {", "local specs = {");
		writeFileSync(
			path.join(game, "spec.luau"),
			`${specs}\nfor _, spec in specs do spec() end\nprint("packed harness passed")\n`,
		);
		assert.match(run("lune", ["run", "spec"], game), /packed harness passed/);
		const mapped = readFileSync(path.join(game, "spec.luau"), "utf8").replaceAll(
			'place = "sample.rbxl"',
			'sourcemap = "sourcemap.json"',
		);
		writeFileSync(path.join(game, "mapped.luau"), mapped);
		assert.match(run("lune", ["run", "mapped"], game), /packed harness passed/);
		const sourcemap = JSON.parse(readFileSync(path.join(game, "sourcemap.json"), "utf8"));
		function typescriptPaths(node) {
			node.filePaths = node.filePaths?.map((file) =>
				file.startsWith("out/") ? file.replace(/^out\//, "src/").replace(/\.luau$/, ".ts") : file,
			);
			for (const child of node.children ?? []) typescriptPaths(child);
		}
		typescriptPaths(sourcemap);
		writeFileSync(path.join(game, "typescript-map.json"), JSON.stringify(sourcemap));
		writeFileSync(
			path.join(game, "typescript.luau"),
			mapped.replaceAll('sourcemap = "sourcemap.json"', 'sourcemap = "typescript-map.json", output = "out"'),
		);
		assert.match(run("lune", ["run", "typescript"], game), /packed harness passed/);
		if (process.platform !== "win32") {
			writeFileSync(
				path.join(game, "case-sensitive.luau"),
				mapped.replaceAll(
					'sourcemap = "sourcemap.json"',
					'sourcemap = "typescript-map.json", sourceRoot = "SRC", output = "out"',
				),
			);
			assert.throws(() => run("lune", ["run", "case-sensitive"], game), /is outside/);
		}
		for (const [name, map] of [
			["compiled", JSON.parse(readFileSync(path.join(game, "sourcemap.json"), "utf8"))],
			["typescript", sourcemap],
		]) {
			for (const separator of process.platform === "win32" ? ["/", "\\"] : ["/"]) {
				const absolute = structuredClone(map);
				function absolutePaths(node) {
					node.filePaths = node.filePaths?.map((file) =>
						(process.platform === "win32" && name === "typescript" && file.startsWith("src/")
							? path.resolve(game, file).toLowerCase()
							: path.resolve(game, file)
						).replaceAll(path.sep, separator),
					);
					for (const child of node.children ?? []) absolutePaths(child);
				}
				absolutePaths(absolute);
				writeFileSync(path.join(game, "absolute-map.json"), JSON.stringify(absolute));
				writeFileSync(
					path.join(game, "absolute.luau"),
					mapped.replaceAll(
						'sourcemap = "sourcemap.json"',
						`sourcemap = "absolute-map.json", root = ${JSON.stringify(game.replaceAll(path.sep, "/"))}, sourceRoot = ${JSON.stringify(path.join(game, "src").replaceAll(path.sep, separator))}, output = "out"`,
					),
				);
				assert.match(
					run("lune", ["run", "absolute"], game),
					/packed harness passed/,
					`${name} absolute ${separator}`,
				);
			}
		}
		const nested = path.join(game, ".lune");
		mkdirSync(nested);
		const nestedSpec = readFileSync(path.join(game, "spec.luau"), "utf8")
			.replaceAll("./node_modules/", "../node_modules/")
			.replaceAll('"sample.rbxl"', '"../sample.rbxl"');
		writeFileSync(path.join(nested, "spec.luau"), nestedSpec);
		assert.match(run("lune", ["run", "spec"], nested), /packed harness passed/);
	} finally {
		rmSync(game, { recursive: true, force: true });
	}
});
