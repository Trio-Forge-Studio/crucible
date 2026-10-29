const fs = require("node:fs");
const path = require("node:path");
const { createRequire } = require("node:module");
const repo = process.env.CRUCIBLE_TEST_REPO ?? path.resolve(__dirname, "../../../..");
const src = path.resolve(__dirname, "..");
const packageRoot = path.resolve(src, "..");
const req = createRequire(path.join(repo, "packages/crucible/package.json"));
const ts = req("typescript");
const transform = require(path.join(src, "index.cjs"));
const scratchBase = process.env.PROBE_SCRATCH ?? path.join(repo, "build/t2-verify3-probes");
const scratch = path.join(scratchBase, String(process.pid));
fs.rmSync(scratch, { recursive: true, force: true });
process.on("exit", () => fs.rmSync(scratch, { recursive: true, force: true }));
const write = (file, value) => {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, typeof value === "string" ? value : JSON.stringify(value));
};
const made = new Map();
function project(ancestor) {
	if (made.has(ancestor)) return made.get(ancestor);
	const game = path.join(scratch, ancestor, "game");
	const installed = path.join(game, "node_modules/@trioforge/crucible");
	fs.mkdirSync(path.join(game, "src"), { recursive: true });
	for (const entry of ["out", "server", "client", "emit", "package.json", "transformer"])
		fs.cpSync(path.join(packageRoot, entry), path.join(installed, entry), { recursive: true });
	fs.mkdirSync(path.join(game, "node_modules/@rbxts"), { recursive: true });
	for (const name of ["types", "compiler-types"])
		fs.symlinkSync(
			path.join(repo, "packages/crucible/node_modules/@rbxts", name),
			path.join(game, "node_modules/@rbxts", name),
			"junction",
		);
	write(path.join(game, "package.json"), { name: "fixture-game", private: true });
	write(path.join(game, "default.project.json"), {
		name: "fx",
		tree: {
			$className: "DataModel",
			ReplicatedStorage: {
				$className: "ReplicatedStorage",
				rbxts_include: {
					$path: "include",
					node_modules: {
						$className: "Folder",
						"@rbxts": { $path: "node_modules/@rbxts" },
						"@trioforge": { $className: "Folder", crucible: { $path: "node_modules/@trioforge/crucible" } },
					},
				},
				TS: { $path: "out/shared" },
			},
			ServerScriptService: { $className: "ServerScriptService", TS: { $path: "out/server" } },
			StarterPlayer: {
				$className: "StarterPlayer",
				StarterPlayerScripts: { $className: "StarterPlayerScripts", TS: { $path: "out/client" } },
			},
		},
	});
	made.set(ancestor, game);
	return game;
}
const base = () => ({
	extends: path.join(repo, "tsconfig.rbxts.json"),
	compilerOptions: {
		rootDir: "src",
		outDir: "out",
		tsBuildInfoFile: "out/build.tsbuildinfo",
		declaration: false,
		typeRoots: ["node_modules/@rbxts", "node_modules/@trioforge"],
		types: ["types", "compiler-types"],
		plugins: [{ transform: "@trioforge/crucible/transformer" }],
	},
	include: ["src"],
});
function setup({ ancestor = "client", files, compilerOptions = {}, include, serverOut }) {
	const game = project(ancestor);
	const rojoFile = path.join(game, "default.project.json");
	const rojo = JSON.parse(fs.readFileSync(rojoFile, "utf8"));
	rojo.tree.ServerScriptService.TS.$path = serverOut ?? "out/server";
	write(rojoFile, rojo);
	for (const entry of fs.readdirSync(game))
		if (!["node_modules", "package.json", "default.project.json"].includes(entry))
			fs.rmSync(path.join(game, entry), { recursive: true, force: true });
	for (const [rel, text] of Object.entries(files)) write(path.join(game, rel), text);
	const config = base();
	for (const [k, v] of Object.entries(compilerOptions))
		if (v === null) delete config.compilerOptions[k];
		else config.compilerOptions[k] = v;
	if (include) config.include = include;
	write(path.join(game, "tsconfig.json"), config);
	return game;
}
const fmt = (game, d) => ({
	code: d.code === 90000 ? "CRU000" : "CRU" + String(d.code - 90000).padStart(3, "0"),
	file: d.file ? path.relative(game, d.file.fileName).replaceAll("\\", "/") : "",
	line: d.file ? d.file.getLineAndCharacterOfPosition(d.start).line + 1 : 0,
	category: d.category,
	msg: ts.flattenDiagnosticMessageText(d.messageText, " "),
});
function run(opts) {
	const game = setup(opts);
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, "tsconfig.json"),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	const program = ts.createProgram(config.fileNames, config.options);
	const diagnostics = [];
	const sources = program.getSourceFiles().filter((s) => !s.isDeclarationFile);
	let thrown;
	try {
		const result = ts.transform(sources, [
			transform(
				program,
				{ rojo: path.join(game, "default.project.json") },
				{ ts, addDiagnostic: (d) => diagnostics.push(d) },
			),
		]);
		result.dispose();
	} catch (e) {
		thrown = e;
	}
	return {
		game,
		thrown,
		common: program.getCommonSourceDirectory(),
		diags: diagnostics.map((d) => fmt(game, d)),
	};
}
module.exports = { run, setup };
module.exports.transformAt = (game, tsconfig, override = {}) => {
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, tsconfig),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	const program = ts.createProgram(config.fileNames, { ...config.options, ...override });
	const diagnostics = [];
	const result = ts.transform(
		program.getSourceFiles().filter((s) => !s.isDeclarationFile),
		[
			transform(
				program,
				{ rojo: path.join(game, "default.project.json") },
				{ ts, addDiagnostic: (d) => diagnostics.push(d) },
			),
		],
	);
	result.dispose();
	return {
		configFilePath: config.options.configFilePath,
		common: program.getCommonSourceDirectory(),
		diags: diagnostics.map((d) => fmt(game, d)).map((d) => `${d.code}@${d.file}:${d.line}`),
	};
};
