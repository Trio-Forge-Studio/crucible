const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const ts = require("typescript");
const transform = require("../index.cjs");
const { setup } = require("./side-harness.cjs");
const fixture = path.join(__dirname, "fixtures/network");
const repo = process.env.CRUCIBLE_TEST_REPO ?? path.resolve(__dirname, "../../../..");
function project(files, options = {}) {
	const game = setup({ files, compilerOptions: options });
	const rojoFile = path.join(game, "default.project.json");
	const rojo = JSON.parse(fs.readFileSync(rojoFile, "utf8"));
	rojo.tree.ReplicatedStorage.Util = { $path: "out/util" };
	fs.mkdirSync(path.join(game, "out/util"), { recursive: true });
	fs.writeFileSync(rojoFile, JSON.stringify(rojo));
	const pkg = path.join(game, "node_modules/@trioforge/crucible");
	for (const [file, target] of Object.entries({
		"index.d.ts": "out/network.d.ts",
		"server.d.ts": "out/server/network.d.ts",
		"client.d.ts": "out/client/network.d.ts",
		"emit.d.ts": "out/emit/network.d.ts",
		"schema.d.ts": "out/emit/schema.d.ts",
	}))
		fs.copyFileSync(path.join(fixture, file), path.join(pkg, target));
	for (const entry of ["", "server/", "client/", "emit/"]) {
		const target = path.join(pkg, "out", entry, "index.d.ts");
		const text = fs.readFileSync(target, "utf8");
		if (!text.includes('export * from "./network"') && !text.includes("export namespace Network")) {
			if (entry === "server/" || entry === "client/")
				fs.appendFileSync(
					target,
					"\n" + fs.readFileSync(path.join(fixture, entry.slice(0, -1) + ".d.ts"), "utf8"),
				);
			else fs.appendFileSync(target, '\nexport * from "./network";\n');
		}
	}
	fs.cpSync(path.join(__dirname, "..", "..", "transformer"), path.join(pkg, "transformer"), { recursive: true });
	return game;
}
function inspect(game, only) {
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, "tsconfig.json"),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	const program = ts.createProgram(config.fileNames, config.options);
	const diagnostics = [];
	const sources = program
		.getSourceFiles()
		.filter((s) => !s.isDeclarationFile && (!only || s.fileName.endsWith(only)));
	const result = ts.transform(sources, [transform(program, {}, { ts, addDiagnostic: (d) => diagnostics.push(d) })]);
	const output = new Map(
		result.transformed.map((s) => [
			path.relative(game, s.fileName).replaceAll("\\", "/"),
			ts.createPrinter().printFile(s),
		]),
	);
	result.dispose();
	return { program, diagnostics, output };
}
function compile(game, expected = 0) {
	const result = spawnSync(process.execPath, [path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", game], {
		cwd: game,
		encoding: "utf8",
		timeout: 120000,
	});
	const output = result.stdout + result.stderr;
	assert.equal(result.status, expected, output);
	return output;
}
function descriptorExpressions(text) {
	const sf = ts.createSourceFile("emitted.ts", text, ts.ScriptTarget.Latest, true);
	const found = new Map();
	const walk = (node) => {
		if (
			ts.isCallExpression(node) &&
			ts.isPropertyAccessExpression(node.expression) &&
			["toServer", "toClient", "askServer"].includes(node.expression.name.text) &&
			node.expression.expression.getText() === "__crucible"
		)
			found.set(
				node.arguments[0].text,
				node.arguments.slice(1).map((a) => a.getText(sf).replaceAll(/\s+/g, " ")),
			);
		ts.forEachChild(node, walk);
	};
	walk(sf);
	return found;
}
module.exports = { project, inspect, compile, descriptorExpressions, ts, fs, path, repo };
