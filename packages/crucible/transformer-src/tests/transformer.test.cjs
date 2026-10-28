const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");
const ts = require("typescript");
const transform = require("../index.cjs");
const { typeId } = require("../type-id.cjs");

const repo = process.env.CRUCIBLE_TEST_REPO ?? path.resolve(__dirname, "../../../..");
const packageRoot = path.resolve(__dirname, "../..");
const scratch = path.join(repo, "build/transformer-tests", String(process.pid));
fs.rmSync(scratch, { recursive: true, force: true });
const game = path.join(scratch, "game");
const installed = path.join(game, "node_modules/@trioforge/crucible");
fs.mkdirSync(path.join(game, "src/server"), { recursive: true });
fs.cpSync(path.join(__dirname, "fixtures/api"), installed, { recursive: true });
fs.mkdirSync(path.join(game, "node_modules/@rbxts"), { recursive: true });
for (const name of ["types", "compiler-types"])
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules/@rbxts", name),
		path.join(game, "node_modules/@rbxts", name),
		"junction",
	);
const write = (file, value) => fs.writeFileSync(file, typeof value === "string" ? value : JSON.stringify(value));
write(path.join(installed, "package.json"), {
	name: "@trioforge/crucible",
	version: "0.0.0",
	types: "index.d.ts",
	main: "init.luau",
	exports: { ".": { types: "./index.d.ts", default: "./init.luau" }, "./transformer": "./transformer/index.js" },
});
fs.cpSync(path.join(packageRoot, "transformer"), path.join(installed, "transformer"), { recursive: true });
for (const file of ["init.luau", "emit.luau", "server.luau"]) write(path.join(installed, file), "return {}\n");
write(path.join(game, "package.json"), { name: "fixture-game", private: true });
write(path.join(game, "tsconfig.json"), {
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
write(path.join(game, "default.project.json"), {
	name: "Transformer fixture",
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
		},
		ServerScriptService: { $className: "ServerScriptService", TS: { $path: "out/server" } },
	},
});
const main = path.join(game, "src/server/main.ts");
const positive = fs.readFileSync(path.join(__dirname, "fixtures/game.ts"), "utf8");

function run(text = positive) {
	write(main, text);
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, "tsconfig.json"),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	const program = ts.createProgram(config.fileNames, config.options);
	const diagnostics = [];
	const result = ts.transform(program.getSourceFile(main), [
		transform(
			program,
			{ rojo: path.join(game, "default.project.json") },
			{ ts, addDiagnostic: (d) => diagnostics.push(d) },
		),
	]);
	const output = ts.createPrinter().printFile(result.transformed[0]);
	result.dispose();
	return { program, output, diagnostics };
}

test("registration contract, inherited deps, tags, attributes, provides and symbol identity", () => {
	const { output, diagnostics } = run();
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	const emitted = [];
	const runtime = {
		defineClass: (klass, data) => emitted.push([klass.name, data]),
		attribute() {},
		resolve() {},
		resolveAll() {},
		t: {
			string: "string",
			number: "number",
			boolean: "boolean",
			vector3: "vector3",
			color3: "color3",
			optional: (check) => ({ optional: check }),
			literal: (value) => ({ literal: value }),
			union: (...checks) => ({ union: checks }),
		},
	};
	const js = ts.transpileModule(output, {
		compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
	}).outputText;
	class CrucibleBehaviour {
		isEnabled() {}
	}
	class Subsystem {
		onShutdown() {}
	}
	const stub = { Subsystem, CrucibleBehaviour };
	new Function("require", "exports", "Color3", "Vector3", js)(
		(name) => (name.endsWith("/emit") ? runtime : name.endsWith("/server") ? { boot() {} } : stub),
		{},
		class Color3 {
			R = 0;
		},
		class Vector3 {
			X = 0;
		},
	);
	assert.deepEqual(
		emitted.map(([name]) => name),
		["Store", "Jobs", "Door", "ConstTag", "AliasedTag", "ThroughAlias"],
	);
	const records = Object.fromEntries(emitted);
	const id = (name) => `fixture-game/server/main:${name}`;
	assert.equal(records.Store.kind, "subsystem");
	assert.equal(records.Store.id, id("Store"));
	assert.deepEqual(records.Store.provides, [id("Base"), id("Child"), id("Parent")]);
	assert.deepEqual(records.Jobs.deps, [id("Store"), { all: id("Child") }]);
	assert.deepEqual(records.Door.deps, [id("Store")]);
	assert.equal(records.Door.tag, "Door");
	assert.equal(records.ConstTag.tag, "ConstTag");
	assert.equal(records.AliasedTag.tag, "AliasTag");
	assert.deepEqual(records.ConstTag.instance, ["Instance"]);
	assert.deepEqual([...records.AliasedTag.instance].sort(), ["BasePart", "Model"]);
	assert.equal(records.ThroughAlias.tag, "Alias");
	assert.deepEqual(records.Door.attributes.fleetId, { name: "FleetId", required: true, check: "string" });
	assert.deepEqual(records.Door.attributes.maxSpeed, {
		name: "MaxSpeed",
		required: false,
		check: { optional: "number" },
	});
	assert.deepEqual(records.Door.attributes.label.check, { optional: "string" });
	assert.deepEqual(records.Door.attributes.mode.check, {
		optional: { union: [{ literal: "walk" }, { literal: "drive" }] },
	});
	assert.deepEqual(records.Door.attributes.negative.check, { optional: { literal: -2 } });
	assert.deepEqual(records.Door.attributes.tint.check, { optional: "color3" });
	assert.deepEqual(records.Door.attributes.offset.check, { optional: "vector3" });
	assert.match(output, /__crucible.attribute\(this, "fleetId"\)/);
	assert.match(output, /__crucible.attribute\(this, "maxSpeed", 30\)/);
	assert.match(output, /__crucible.attribute\(this, "label", undefined\)/);
	assert.match(output, /__crucible.resolve<Store>\("fixture-game\/server\/main:Store"\)/);
	assert.match(output, /__crucible.resolveAll<Child>\("fixture-game\/server\/main:Child"\)/);
	assert.match(output, /crucible-path:ServerScriptService\/TS\/\*\*\/\*\{Service,Behaviour\}/);
});

test("diagnostic negative probes and clean controls use exact source lines", () => {
	const header =
		'import { Subsystem, CrucibleBehaviour, attribute, type All } from "@trioforge/crucible";\nimport { boot } from "@trioforge/crucible/server";\n';
	const probes = [
		["CRU001", "export class Bad extends Subsystem { constructor(value: number) { super(); } }"],
		["CRU002", "export class Bad extends CrucibleBehaviour<string> {}"],
		["CRU003", "export class Bad<T> extends Subsystem {}"],
		["CRU006", 'boot({load: ["outside/**/*.ts"]});'],
		["CRU007", 'export class Bad extends CrucibleBehaviour<"Bad"> { field = attribute<never>({} as never); }'],
		["CRU008", 'export class Bad extends CrucibleBehaviour<"Bad", { wrong: true }> {}'],
		[
			"CRU010",
			"declare global { interface GlobalType { readonly value: string; } }\nexport class Bad extends Subsystem { constructor(value: GlobalType) { super(); } }",
			4,
		],
		["CRU015", "const bad = attribute(1);"],
	];
	for (const [code, text, line = 3] of probes) {
		const found = run(header + text).diagnostics;
		assert.ok(
			found.some(
				(d) =>
					d.messageText.startsWith(code) && d.file.getLineAndCharacterOfPosition(d.start).line + 1 === line,
			),
			`${code} on line ${line}: ${found.map((d) => d.messageText)}`,
		);
		assert.ok(found.every((d) => d.category === ts.DiagnosticCategory.Error));
	}
	assert.deepEqual(
		run(
			header +
				'export class Good extends Subsystem {}\nexport class Fine extends CrucibleBehaviour<"Fine"> { field = attribute(1); }',
		).diagnostics,
		[],
	);
	const malformed = run(header + "export class Broken extends Subsystem { constructor(");
	assert.ok(malformed.diagnostics.length || ts.getPreEmitDiagnostics(malformed.program).length);
});

test("CRU001 rejects every noninjectable constructor shape and accepts abstract behaviours in All", () => {
	const header = 'import { Subsystem, CrucibleBehaviour, type All } from "@trioforge/crucible";\n';
	for (const parameter of [
		"value: number",
		"value: Instance",
		"value: Model | BasePart",
		"value?: Dependency",
		"...value: Dependency[]",
		"value: Generic<string>",
		"value: Concrete",
		"value: All<number>",
	]) {
		const text =
			header +
			'interface Dependency {}\ninterface Generic<T> { value: T; }\nclass Concrete extends CrucibleBehaviour<"Concrete"> {}\nexport class Bad extends Subsystem { constructor(' +
			parameter +
			") { super(); } }";
		assert.ok(
			run(text).diagnostics.some((d) => d.messageText.startsWith("CRU001")),
			parameter,
		);
	}
	const text =
		header +
		'abstract class Abstract extends CrucibleBehaviour<"Abstract"> {}\nexport class Good extends Subsystem { constructor(value: Abstract, all: All<Abstract>) { super(); } }';
	assert.deepEqual(
		run(text).diagnostics.map((d) => d.messageText),
		[],
	);
});

test("CRU003 rejects nested, generic and nonpublic constructors including inherited visibility", () => {
	const header = 'import { Subsystem } from "@trioforge/crucible";\n';
	for (const text of [
		"export class Bad<T> extends Subsystem {}",
		"export function scope() { class Bad extends Subsystem {} }",
		"export class Bad extends Subsystem { protected constructor() { super(); } }",
		"export class Bad extends Subsystem { private constructor() { super(); } }",
		"abstract class Base extends Subsystem { protected constructor() { super(); } }\nexport class Bad extends Base {}",
	])
		assert.ok(
			run(header + text).diagnostics.some((d) => d.messageText.startsWith("CRU003")),
			text,
		);
});

test("CRU002 rejects tag unions and widened aliases but permits open abstract tags", () => {
	const header = 'import { CrucibleBehaviour } from "@trioforge/crucible";\n';
	for (const text of [
		'type Tag = "a" | "b"; export class Bad extends CrucibleBehaviour<Tag> {}',
		'const tag: string = "a"; export class Bad extends CrucibleBehaviour<typeof tag> {}',
	])
		assert.ok(
			run(header + text).diagnostics.some((d) => d.messageText.startsWith("CRU002")),
			text,
		);
	assert.deepEqual(
		run(
			header + "export abstract class Open<Tag extends string> extends CrucibleBehaviour<Tag> {}",
		).diagnostics.map((d) => d.messageText),
		[],
	);
});

test("CRU006 rejects dynamic load shapes and preserves string literal paths with a custom Rojo mapping", () => {
	const header = 'import { boot } from "@trioforge/crucible/server";\n';
	for (const text of [
		'const glob = "src/server/**/*.ts"; boot({load: [glob]});',
		'const load = ["src/server/**/*.ts"]; boot({load});',
		"const options = {load: []}; boot(options);",
	])
		assert.ok(
			run(header + text).diagnostics.some((d) => d.messageText.startsWith("CRU006")),
			text,
		);
	const { output, diagnostics } = run(header + 'boot({"load": ["src/server/main.ts"]});');
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(output, /crucible-path:ServerScriptService\/TS\/main/);
});

test("CRU015 rejects static fields, nested calls, plain classes and subsystem fields", () => {
	const header = 'import { attribute, CrucibleBehaviour, Subsystem } from "@trioforge/crucible";\n';
	for (const text of [
		"class Plain { field = attribute(1); }",
		"class Service extends Subsystem { field = attribute(1); }",
		'class Bad extends CrucibleBehaviour<"Bad"> { static field = attribute(1); }',
		'class Bad extends CrucibleBehaviour<"Bad"> { field = [attribute(1)]; }',
	])
		assert.ok(
			run(header + text).diagnostics.some((d) => d.messageText.startsWith("CRU015")),
			text,
		);
});

test("CRU010 rejects global provides and attributes in addition to constructor deps", () => {
	const header = 'import { attribute, CrucibleBehaviour, Subsystem } from "@trioforge/crucible";\n';
	for (const text of [
		"declare global { interface GlobalType {} }\nexport class Bad extends Subsystem implements GlobalType {}",
		'declare global { interface GlobalValue { readonly x: number; } }\nexport class Bad extends CrucibleBehaviour<"Bad"> { field: GlobalValue = attribute<GlobalValue>({x: 1}); }',
	])
		assert.ok(
			run(header + text).diagnostics.some((d) => d.messageText.startsWith("CRU010")),
			text,
		);
});

test("API identity ignores local lookalikes and follows namespace and renamed imports", () => {
	const text =
		'import * as C from "@trioforge/crucible";\nimport { Subsystem as RealSubsystem } from "@trioforge/crucible";\nclass Subsystem { value = 1; }\nclass Fake extends Subsystem {}\nclass CrucibleBehaviour<T> { value?: T; }\nclass FakeBehaviour extends CrucibleBehaviour<"Fake"> {}\nclass Real extends RealSubsystem {}\nclass RealBehaviour extends C.CrucibleBehaviour<"Real"> {}\nfunction inject<T>(): T { return undefined as T; }\nconst untouched = inject<Real>();';
	const { output, diagnostics } = run(text);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	assert.equal((output.match(/__crucible.defineClass\(/g) ?? []).length, 2);
	assert.match(output, /const untouched = inject<Real>\(\)/);
});

test("CRU007 and CRU008 reject engine type lookalikes and allow engine aliases", () => {
	for (const text of [
		'class Model { fake = 1; }\nclass Bad extends CrucibleBehaviour<"Bad", Model> {}',
		'class Vector3 { fake = 1; }\nclass Bad extends CrucibleBehaviour<"Bad"> { field = attribute<never>(new Vector3() as never); }',
	]) {
		const { diagnostics } = run('import { CrucibleBehaviour, attribute } from "@trioforge/crucible";\n' + text);
		assert.ok(diagnostics.some((d) => /CRU00[78]/.test(d.messageText)));
	}
	const text =
		'import { CrucibleBehaviour, attribute } from "@trioforge/crucible";\ntype Body = Model; type Tint = Color3;\nclass Good extends CrucibleBehaviour<"Good", Body> { field = attribute<Tint>(new Color3()); }';
	assert.deepEqual(
		run(text).diagnostics.map((d) => d.messageText),
		[],
	);
});

test("CRU010 rejects non-module declarations", () => {
	const globalFile = path.join(game, "global.d.ts");
	write(globalFile, "interface NonModule { readonly name: string; }");
	try {
		const { diagnostics } = run(
			'/// <reference path="../../global.d.ts" />\nimport { Subsystem } from "@trioforge/crucible";\nexport class Bad extends Subsystem { constructor(value: NonModule) { super(); } }',
		);
		assert.ok(diagnostics.some((d) => d.messageText.startsWith("CRU010")));
	} finally {
		fs.rmSync(globalFile);
	}
});

test("rbxtsc reports Crucible diagnostics without a Node stack on malformed macro input", () => {
	write(
		main,
		'import { boot } from "@trioforge/crucible/server";\nconst glob = "src/server/**/*.ts";\nboot({load: [glob]});',
	);
	fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
	const result = spawnSync(
		process.execPath,
		[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", path.join(game, "tsconfig.json")],
		{ cwd: game, encoding: "utf8" },
	);
	assert.equal(result.status, 1);
	assert.match(result.stdout + result.stderr, /CRU006/);
	assert.doesNotMatch(result.stdout + result.stderr, /at transformer|at visit|Node\.js v/);
});

test("package declaration ids ignore consumer rootDir and respect exports types conditions", () => {
	const fixture = path.join(scratch, "type-id");
	fs.cpSync(path.join(__dirname, "fixtures/type-id"), fixture, { recursive: true });
	const pkg = path.join(fixture, "pkg");
	write(path.join(pkg, "tsconfig.json"), { compilerOptions: { rootDir: "src", outDir: "out" } });
	const src = path.join(pkg, "src/events/handler.ts");
	const program = ts.createProgram([path.join(pkg, "src/index.ts")], {
		rootDir: path.join(pkg, "src"),
		outDir: path.join(pkg, "out"),
		declaration: true,
		emitDeclarationOnly: true,
		noLib: true,
	});
	program.emit();
	const own = typeId(src, "Handler", program, ts);
	assert.equal(own, "@trioforge/fixture/events/handler:Handler");
	const installedPkg = path.join(fixture, "game/node_modules/@trioforge/fixture");
	fs.mkdirSync(installedPkg, { recursive: true });
	fs.copyFileSync(path.join(pkg, "package.json"), path.join(installedPkg, "package.json"));
	fs.cpSync(path.join(pkg, "out"), path.join(installedPkg, "out"), { recursive: true });
	const declaration = path.join(installedPkg, "out/events/handler.d.ts");
	for (const rootDir of [path.join(fixture, "game/src"), path.join(fixture, "game")]) {
		const main = path.join(fixture, "game/src/main.ts");
		const consumer = ts.createProgram([main], {
			rootDir,
			noLib: true,
			types: [],
			module: ts.ModuleKind.CommonJS,
			moduleResolution: ts.ModuleResolutionKind.Node10,
		});
		const checker = consumer.getTypeChecker();
		const uses = consumer.getSourceFile(main).statements.find(ts.isClassDeclaration);
		const imported = checker.getSymbolAtLocation(uses.heritageClauses[0].types[0].expression);
		const resolved = checker.getAliasedSymbol(imported).declarations[0].getSourceFile().fileName;
		assert.equal(path.normalize(resolved), path.normalize(declaration));
		assert.equal(typeId(resolved, "Handler", consumer, ts), own);
	}
	write(path.join(installedPkg, "package.json"), {
		name: "@trioforge/fixture",
		types: "wrong/index.d.ts",
		exports: { ".": { import: { types: "./out/index.d.ts", default: "./out/index.js" } } },
	});
	assert.equal(typeId(declaration, "Handler", program, ts), own);
	const exportsMain = path.join(fixture, "game/src/main.ts");
	const exportsProgram = ts.createProgram([exportsMain], {
		noLib: true,
		types: [],
		rootDir: path.join(fixture, "game"),
		module: ts.ModuleKind.ESNext,
		moduleResolution: ts.ModuleResolutionKind.Bundler,
	});
	const exportsChecker = exportsProgram.getTypeChecker();
	const uses = exportsProgram.getSourceFile(exportsMain).statements.find(ts.isClassDeclaration);
	const imported = exportsChecker.getSymbolAtLocation(uses.heritageClauses[0].types[0].expression);
	const exportsDeclaration = exportsChecker.getAliasedSymbol(imported).declarations[0].getSourceFile().fileName;
	assert.equal(typeId(exportsDeclaration, "Handler", exportsProgram, ts), own);
});

test("rbxtsc consumer plugin resolves and emitted TypeScript rechecks into Luau", () => {
	write(main, positive);
	fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
	const cli = path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js");
	const result = spawnSync(
		process.execPath,
		[cli, "-p", path.join(game, "tsconfig.json"), "--writeTransformedFiles"],
		{ cwd: game, encoding: "utf8" },
	);
	assert.equal(result.status, 0, result.stdout + result.stderr);
	const luau = fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8");
	assert.equal((luau.match(/__crucible.defineClass\(/g) ?? []).length, 6);
	assert.match(luau, /kind = "behaviour"/);
	assert.match(luau, /tag = "Door"/);
	assert.match(luau, /crucible-path:ServerScriptService\/TS/);
});
