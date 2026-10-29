const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test, after } = require("node:test");
const ts = require("typescript");
const transform = require("../index.cjs");
const { typeId } = require("../type-id.cjs");

const repo = process.env.CRUCIBLE_TEST_REPO ?? path.resolve(__dirname, "../../../..");
const packageRoot = path.resolve(__dirname, "../..");
const scratch = path.join(repo, "build/transformer-tests", String(process.pid));
fs.rmSync(scratch, { recursive: true, force: true });
after(() => fs.rmSync(scratch, { recursive: true, force: true }));
const game = path.join(scratch, "client/game");
const installed = path.join(game, "node_modules/@trioforge/crucible");
fs.mkdirSync(path.join(game, "src/server"), { recursive: true });
fs.mkdirSync(installed, { recursive: true });
for (const entry of ["out", "server", "client", "emit", "package.json"])
	fs.cpSync(path.join(packageRoot, entry), path.join(installed, entry), { recursive: true });
fs.mkdirSync(path.join(game, "node_modules/@rbxts"), { recursive: true });
for (const name of ["types", "compiler-types"])
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules/@rbxts", name),
		path.join(game, "node_modules/@rbxts", name),
		"junction",
	);
const write = (file, value) => fs.writeFileSync(file, typeof value === "string" ? value : JSON.stringify(value));
fs.cpSync(path.join(packageRoot, "transformer"), path.join(installed, "transformer"), { recursive: true });
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

test("fixture uses every real package declaration and physical side entry byte for byte", () => {
	assert.equal(
		fs.readFileSync(path.join(installed, "package.json"), "utf8"),
		fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
	);
	for (const name of fs
		.readdirSync(path.join(packageRoot, "out"), { recursive: true })
		.filter((name) => name.endsWith(".d.ts")))
		assert.equal(
			fs.readFileSync(path.join(installed, "out", name), "utf8"),
			fs.readFileSync(path.join(packageRoot, "out", name), "utf8"),
		);
	for (const side of ["server", "client", "emit"])
		assert.equal(
			fs.readFileSync(path.join(installed, side, "package.json"), "utf8"),
			fs.readFileSync(path.join(packageRoot, side, "package.json"), "utf8"),
		);
});

test("real emit literal declaration accepts boolean string and number literals", () => {
	write(
		main,
		'import { t } from "@trioforge/crucible/emit";\nt.literal(true);\nt.literal(false);\nt.literal("walk");\nt.literal(-2);\n',
	);
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, "tsconfig.json"),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	const program = ts.createProgram(config.fileNames, config.options);
	assert.deepEqual(
		program
			.getSemanticDiagnostics(program.getSourceFile(main))
			.map((d) => ts.flattenDiagnosticMessageText(d.messageText, "\n")),
		[],
	);
});

function run(text = positive, options = {}, pluginConfig = {}, factory = transform, allSources = false) {
	write(main, text);
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, "tsconfig.json"),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	const program = ts.createProgram(config.fileNames, { ...config.options, ...options });
	const diagnostics = [];
	const sources = allSources
		? program.getSourceFiles().filter((source) => !source.isDeclarationFile)
		: program.getSourceFile(main);
	const result = ts.transform(sources, [
		factory(
			program,
			{ rojo: path.join(game, "default.project.json"), ...pluginConfig },
			{ ts, addDiagnostic: (d) => diagnostics.push(d) },
		),
	]);
	const output = ts
		.createPrinter()
		.printFile(result.transformed.find((source) => path.normalize(source.fileName) === path.normalize(main)));
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
	assert.deepEqual(records.Door.provides, [id("Openable")]);
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
	assert.deepEqual(records.Door.attributes.open.check, { optional: "boolean" });
	assert.deepEqual(records.Door.attributes.maybeOpen.check, { optional: "boolean" });
	assert.match(output, /__crucible.attribute<string>\(this, "fleetId"\)/);
	assert.match(output, /__crucible.attribute<number>\(this, "maxSpeed", 30\)/);
	assert.match(output, /__crucible.attribute<string \| undefined>\(this, "label", undefined\)/);
	assert.match(output, /__crucible.resolve<Store>\("fixture-game\/server\/main:Store"\)/);
	assert.match(output, /__crucible.resolveAll<Child>\("fixture-game\/server\/main:Child"\)/);
	assert.match(output, /"crucible-path:ServerScriptService\/TS\/\*\*\/\*\{Service,Behaviour\}"/);
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
		).diagnostics.map((d) => d.messageText),
		[],
	);
	const malformed = run(header + "boot({load: [undefined]});");
	assert.ok(malformed.diagnostics.some((d) => d.messageText.startsWith("CRU006")));
});

test("CRU001 rejects every noninjectable constructor shape and accepts abstract behaviours in All", () => {
	const header = 'import { Subsystem, CrucibleBehaviour, type All } from "@trioforge/crucible";\n';
	for (const parameter of [
		"value: number",
		"value: typeof Dependency",
		"value: typeof AbstractDependency",
		"value: Instance",
		"value: Model | BasePart",
		"value?: Dependency",
		"...value: Dependency[]",
		"value: Generic<string>",
		"value: Concrete",
		"value: All<number>",
		"value: All<All<Dependency>>",
	]) {
		const text =
			header +
			'declare class Dependency {}\nabstract class AbstractDependency {}\ninterface Generic<T> { value: T; }\nclass Concrete extends CrucibleBehaviour<"Concrete"> {}\nexport class Bad extends Subsystem { constructor(' +
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
	const header = 'import { Subsystem, CrucibleBehaviour, attribute } from "@trioforge/crucible";\n';
	for (const text of [
		"export class Bad<T> extends Subsystem {}",
		"export function scope() { class Bad extends Subsystem {} }",
		"export const X = class extends Subsystem {};",
		"export function scope() { return class Y extends Subsystem {}; }",
		"export class Bad extends Subsystem { protected constructor() { super(); } }",
		"export class Bad extends Subsystem { private constructor() { super(); } }",
		"abstract class Base extends Subsystem { protected constructor() { super(); } }\nexport class Bad extends Base {}",
	])
		assert.ok(
			run(header + text).diagnostics.some((d) => d.messageText.startsWith("CRU003")),
			text,
		);
	const expression = run(
		header + 'const Bad = class extends CrucibleBehaviour<"Bad"> { field = attribute(1); };',
	).diagnostics;
	assert.equal(expression.filter((d) => d.messageText.startsWith("CRU003")).length, 1);
	assert.equal(expression.filter((d) => d.messageText.startsWith("CRU015")).length, 0);
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
		'const options = {load: ["src/server/**/*.ts"]}; boot({...options});',
		'const options = {load: ["src/server/**/*.ts"]}; boot({load: [], ...options});',
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
	assert.match(output, /"crucible-path:ServerScriptService\/TS\/main"/);
	const beforeLoad = run(
		header + 'const options = {fixedRate: 30}; boot({...options, load: ["src/server/**/*.ts"]});',
	);
	assert.deepEqual(
		beforeLoad.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(beforeLoad.output, /crucible-path:/);
	for (const wrapper of ['({load: ["src/server/**/*.ts"]})', '{load: ["src/server/**/*.ts"]} as const']) {
		const wrapped = run(header + `boot(${wrapper});`);
		assert.deepEqual(
			wrapped.diagnostics.map((d) => d.messageText),
			[],
		);
		assert.match(wrapped.output, /crucible-path:/);
	}
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

test("CRU010 rejects global primitive aliases on the attribute field line", () => {
	const { diagnostics } = run(
		[
			'import { attribute, CrucibleBehaviour } from "@trioforge/crucible";',
			"declare global {",
			" type GlobalSpeed = number;",
			"}",
			"",
			'export class Van extends CrucibleBehaviour<"Van"> {',
			" readonly speed = attribute<GlobalSpeed>();",
			"}",
		].join("\n"),
	);
	const found = diagnostics.filter((d) => d.messageText.startsWith("CRU010"));
	assert.equal(found.length, 1);
	assert.equal(found[0].file.getLineAndCharacterOfPosition(found[0].start).line + 1, 7);
});

test("ambient classes emit no registration while concrete classes do", () => {
	const { output, diagnostics } = run(
		`import { Subsystem } from "@trioforge/crucible";
export declare class Ghost extends Subsystem {}
declare namespace N { export class Ghost extends Subsystem {} }
declare global { class GlobalGhost extends Subsystem {} }
export class Real extends Subsystem {}`,
	);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	assert.doesNotMatch(output, /defineClass\(Ghost/);
	assert.doesNotMatch(output, /defineClass\(GlobalGhost/);
	assert.match(output, /defineClass\(Real/);
});

test("attribute rewrite preserves a written alias type argument", () => {
	const { output, diagnostics } = run(
		`import { attribute, CrucibleBehaviour } from "@trioforge/crucible";
type Speed = number;
export class Vehicle extends CrucibleBehaviour<"Vehicle"> {
 readonly speed = attribute<Speed>();
}`,
	);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(output, /__crucible.attribute<Speed>\(this, "speed"\)/);
});

test("mixed attribute unions use a union check", () => {
	const { output, diagnostics } = run(
		'import { attribute, CrucibleBehaviour } from "@trioforge/crucible"; export class Mixed extends CrucibleBehaviour<"Mixed"> { readonly value = attribute<string | number>(); }',
	);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(output, /__crucible.t.union\(__crucible.t.string, __crucible.t.number\)/);
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
	for (const [code, text] of [
		["CRU008", 'class Model { fake = 1; }\nclass Bad extends CrucibleBehaviour<"Bad", Model> {}'],
		[
			"CRU007",
			'class Vector3 { fake = 1; }\nclass Bad extends CrucibleBehaviour<"Bad"> { field = attribute<never>(new Vector3() as never); }',
		],
	]) {
		const { diagnostics } = run('import { CrucibleBehaviour, attribute } from "@trioforge/crucible";\n' + text);
		assert.ok(diagnostics.some((d) => d.messageText.startsWith(code)));
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

test("review regressions cover alias macros, abstract-only imports, empty files and client boot", () => {
	const header = 'import { CrucibleBehaviour, attribute, inject, type All } from "@trioforge/crucible";\n';
	const abstract = run(
		header + 'export abstract class Base extends CrucibleBehaviour<"Base"> { field = attribute<boolean>(); }',
	);
	assert.deepEqual(
		abstract.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(abstract.output, /import \* as __crucible/);
	assert.doesNotMatch(abstract.output, /defineClass/);
	assert.doesNotMatch(run("export const untouched = 1;").output, /__crucible/);
	const aliased = run(header + 'const a = attribute; class B extends CrucibleBehaviour<"B"> { x = a(1); }');
	assert.deepEqual(
		aliased.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(aliased.output, /__crucible.attribute<number>\(this, "x", 1\)/);
	assert.match(aliased.output, /"x":/);
	assert.ok(
		run(header + "const a = attribute; const bad = a(1);").diagnostics.some((d) =>
			d.messageText.startsWith("CRU015"),
		),
	);
	const client = run('import { boot } from "@trioforge/crucible/client"; boot({load: ["src/server/**/*.ts"]});');
	assert.deepEqual(
		client.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(client.output, /crucible-path:/);
});

test("type id failures report an infrastructure diagnostic for classes, injects and provides", () => {
	const manifest = path.join(game, "package.json");
	const original = fs.readFileSync(manifest, "utf8");
	try {
		write(manifest, { private: true });
		const found = run(
			'import { Subsystem, inject } from "@trioforge/crucible";\ninterface Dep {}\nclass S extends Subsystem implements Dep {}\nconst one = inject<S>();',
		).diagnostics;
		assert.ok(found.some((d) => d.file.getLineAndCharacterOfPosition(d.start).line === 2));
		assert.ok(found.some((d) => d.file.getLineAndCharacterOfPosition(d.start).line === 3));
		assert.ok(
			found.every((d) => d.messageText.startsWith("Crucible:") && d.category === ts.DiagnosticCategory.Error),
		);
		const dependency = run(
			'import { Subsystem } from "@trioforge/crucible"; interface Dep {} class S extends Subsystem { constructor(dep: Dep) { super(); } }',
		).diagnostics;
		assert.ok(dependency.some((d) => d.messageText.startsWith("Crucible:")));
		assert.ok(dependency.every((d) => !d.messageText.startsWith("CRU001")));
		write(manifest, "{");
		assert.ok(run("export const value = 1;").diagnostics.some((d) => d.messageText.startsWith("Crucible:")));
	} finally {
		write(manifest, original);
	}
	const sub = path.join(game, "src/server/sub");
	fs.mkdirSync(sub, { recursive: true });
	write(path.join(sub, "package.json"), { type: "module" });
	write(path.join(sub, "dep.ts"), "export interface Dep {}");
	try {
		const found = run(
			'import { Subsystem } from "@trioforge/crucible";\nimport type { Dep } from "./sub/dep";\nclass S extends Subsystem implements Dep {}',
		).diagnostics;
		assert.ok(found.some((d) => d.messageText.startsWith("Crucible:")));
	} finally {
		fs.rmSync(sub, { recursive: true, force: true });
	}
});

test("rootDirs globs rewrite and resolver failures report CRU006 without internal stacks", () => {
	const text = 'import { boot } from "@trioforge/crucible/server"; boot({load: ["src/server/**/*.ts"]});';
	const multipleRoots = run(text, { rootDir: undefined, rootDirs: [path.join(game, "src")] });
	assert.deepEqual(
		multipleRoots.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(multipleRoots.output, /crucible-path:ServerScriptService\/TS/);
	const missing = run(text, {}, { rojo: path.join(game, "missing.project.json") });
	assert.ok(missing.diagnostics.some((d) => d.messageText.startsWith("CRU006")));
	assert.ok(missing.diagnostics.every((d) => !/stack|CRU003|Require stack/i.test(d.messageText)));
	const configFile = path.join(game, "tsconfig.json");
	const original = fs.readFileSync(configFile, "utf8");
	const projectFile = path.join(game, "default.project.json");
	const originalProject = fs.readFileSync(projectFile, "utf8");
	try {
		const config = JSON.parse(original);
		delete config.compilerOptions.rootDir;
		config.compilerOptions.rootDirs = ["src"];
		write(configFile, config);
		write(main, text);
		fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
		const result = spawnSync(
			process.execPath,
			[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", configFile],
			{ cwd: game, encoding: "utf8" },
		);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		assert.match(fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8"), /crucible-path:/);
		config.compilerOptions.rootDirs = ["src/shared", "src/server"];
		write(configFile, config);
		fs.mkdirSync(path.join(game, "src/shared"), { recursive: true });
		const shared = path.join(game, "src/shared/foo.ts");
		const server = path.join(game, "src/server/foo.ts");
		write(shared, "export interface Foo { shared: true; }");
		write(server, "export interface Foo { server: true; }");
		const project = JSON.parse(originalProject);
		project.tree.ReplicatedStorage.Shared = { $path: "out/shared" };
		write(projectFile, project);
		const twoRootsText =
			'import { Subsystem } from "@trioforge/crucible"; import { boot } from "@trioforge/crucible/server"; export class S extends Subsystem {} boot({load: ["src/server/**/*.ts"]});';
		const twoRoots = run(twoRootsText);
		assert.deepEqual(
			twoRoots.diagnostics.map((d) => d.messageText),
			[],
		);
		assert.match(twoRoots.output, /"id": "fixture-game\/server\/main:S"/);
		assert.equal(typeId(shared, "Foo", twoRoots.program, ts), "fixture-game/shared/foo:Foo");
		assert.equal(typeId(server, "Foo", twoRoots.program, ts), "fixture-game/server/foo:Foo");
		fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
		const compiled = spawnSync(
			process.execPath,
			[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", configFile],
			{ cwd: game, encoding: "utf8" },
		);
		assert.equal(compiled.status, 0, compiled.stdout + compiled.stderr);
		assert.match(fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8"), /fixture-game\/server\/main:S/);
		assert.match(
			fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8"),
			/crucible-path:ServerScriptService\/TS/,
		);
	} finally {
		write(configFile, original);
		write(projectFile, originalProject);
		fs.rmSync(path.join(game, "src/shared"), { recursive: true, force: true });
		fs.rmSync(path.join(game, "src/server/foo.ts"), { force: true });
	}
});

test("global compiler resolver is taken from the running compiler module", () => {
	require(path.join(repo, "node_modules/roblox-ts/out/Project/functions/compileFiles.js"));
	const filename = path.resolve(__dirname, "../index.cjs");
	const sourceRequire = require("node:module").createRequire(filename);
	const isolatedRequire = (name) => sourceRequire(name);
	isolatedRequire.cache = require.cache;
	isolatedRequire.resolve = (name, options) => {
		if (name === "roblox-ts/package.json") throw new Error("No project-local rbxtsc install");
		return sourceRequire.resolve(name, options);
	};
	const sandboxModule = { exports: {} };
	require("node:vm").runInNewContext(fs.readFileSync(filename, "utf8"), {
		module: sandboxModule,
		require: isolatedRequire,
		__dirname: path.dirname(filename),
	});
	const { diagnostics, output } = run(
		'import { boot } from "@trioforge/crucible/server"; boot({load: ["src/server/**/*.ts"]});',
		{},
		{},
		sandboxModule.exports,
	);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(output, /crucible-path:/);
});

test("diagnostics deduplicate repeated and inherited faults across transformed files", () => {
	const found = run(
		'import { Subsystem, CrucibleBehaviour, attribute } from "@trioforge/crucible";\ndeclare global { interface Global {} }\nabstract class Base extends Subsystem { constructor(value: Global) { super(); } }\nclass A extends Base {}\nclass B extends Base {}\nclass Bad extends CrucibleBehaviour<"Bad"> { field = attribute<any>(); }',
	).diagnostics;
	assert.equal(found.filter((d) => d.messageText.startsWith("CRU010")).length, 1);
	assert.equal(found.filter((d) => d.messageText.startsWith("CRU007")).length, 1);
	const files = ["base.ts", "a.ts", "b.ts"].map((file) => path.join(game, "src/server", file));
	write(
		files[0],
		'import { Subsystem } from "@trioforge/crucible"; export abstract class Base extends Subsystem { constructor(n: number) { super(); } }',
	);
	write(files[1], 'import { Base } from "./base"; export class A extends Base {}');
	write(files[2], 'import { Base } from "./base"; export class B extends Base {}');
	try {
		const inherited = run("export const unrelated = 1;", {}, {}, transform, true).diagnostics;
		assert.equal(inherited.filter((d) => d.messageText.startsWith("CRU001")).length, 1);
		assert.equal(path.basename(inherited[0].file.fileName), "base.ts");
	} finally {
		for (const file of files) fs.rmSync(file);
	}
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
	write(
		path.join(game, "src/server/collection.ts"),
		'import type { All } from "@trioforge/crucible"; interface Hidden { value: number; } export type Collection = All<Hidden>;',
	);
	write(
		main,
		'import { inject } from "@trioforge/crucible"; import type { Collection } from "./collection"; export const result: Collection = inject<Collection>();',
	);
	fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
	const hidden = spawnSync(process.execPath, [cli, "-p", path.join(game, "tsconfig.json")], {
		cwd: game,
		encoding: "utf8",
	});
	assert.equal(hidden.status, 0, hidden.stdout + hidden.stderr);
	assert.match(
		fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8"),
		/resolveAll\("fixture-game\/server\/collection:Hidden"\)/,
	);
});

test("T2 exact-line diagnostics and reviewer bypasses", () => {
	const prefix = 'import { Subsystem, CrucibleBehaviour, inject, type All } from "@trioforge/crucible";\n';
	const cases = [
		["CRU004", 'import "@trioforge/crucible/emit";'],
		["CRU004", 'import "@trioforge/crucible/internal/foo";'],
		["CRU005", "export const bad = inject<number>();"],
		["CRU005", "export function make<T>() { return inject<T>(); }"],
		["CRU005", "export class Concrete extends Subsystem {}\nexport const bad = inject<typeof Concrete>();"],
		["CRU005", "export class Concrete extends Subsystem {}\nexport const bad = inject<All<typeof Concrete>>();"],
		["CRU005", "export const bad = inject<{ a: number }>();"],
		["CRU005", "enum Target { A, B }\nexport const bad = inject<Target>();"],
		[
			"CRU012",
			"interface MyP extends Promise<void> {}\nexport class Bad extends Subsystem { protected override start(): MyP | undefined { return undefined; } }",
		],
		[
			"CRU014",
			'interface H<T> { value: T; }\nexport abstract class Openable<Tag extends string> extends CrucibleBehaviour<Tag, Model> {}\nexport abstract class B<Tag extends string> extends Openable<Tag> implements H<number> { value = 1; }\nexport class Door extends B<"Door"> {}',
		],
		["CRU005", "export class Bad extends Subsystem { field = inject<Subsystem>(); }"],
		["CRU005", "export const bad = inject();"],
		["CRU009", "export class Bad extends Subsystem { onInit() {} }"],
		["CRU009", "export class Bad extends Subsystem { Update() {} }"],
		["CRU009", "export class Bad extends Subsystem { updte() {} }"],
		["CRU011", "export class Bad extends Subsystem { protected override preRender() {} }"],
		["CRU011", "export class Bad extends Subsystem { static override renderPriority = 1; }"],
		["CRU012", "export class Bad extends Subsystem { protected override async start() {} }"],
		[
			"CRU012",
			"export class Bad extends Subsystem { protected override start(): Promise<void> | undefined { return undefined; } }",
		],
		[
			"CRU012",
			"type Result = Promise<void> | undefined;\nexport class Bad extends Subsystem { protected override start(): Result { return undefined; } }",
		],
		[
			"CRU013",
			"export class Concrete extends Subsystem {}\nlet variable = Concrete;\nexport class Bad extends variable {}",
		],
		[
			"CRU013",
			"export class Concrete extends Subsystem {}\nconst alias = Concrete;\nexport class Bad extends alias {}",
		],
		[
			"CRU013",
			"export class Concrete extends Subsystem {}\ntype Constructor = typeof Concrete;\nconst alias: Constructor = Concrete;\nexport class Bad extends alias {}",
		],
	];
	for (const [code, text] of cases) {
		const { diagnostics } = run(prefix + text);
		const expectedLine = (prefix + text).trimEnd().split("\n").length;
		assert.ok(
			diagnostics.some(
				(d) =>
					d.code === 90000 + Number(code.slice(3)) &&
					d.category === ts.DiagnosticCategory.Error &&
					d.file.getLineAndCharacterOfPosition(d.start).line + 1 === expectedLine,
			),
			code + ": " + text + JSON.stringify(diagnostics.map((d) => d.messageText)),
		);
		assert.ok(!diagnostics.some((d) => d.code === 90000), "no infrastructure failure");
	}
	for (const [code, text] of [
		["CRU004", 'import type { Missing } from "@trioforge/crucible/missing";'],
		["CRU005", "export const bad = inject<Missing>();"],
		["CRU009", "export class Bad extends Subsystem { onInit(value: Missing) {} }"],
		["CRU011", "export class Bad extends Subsystem { protected override preRender(value: Missing) {} }"],
		["CRU012", "export class Bad extends Subsystem { protected override async start(value: Missing) {} }"],
		[
			"CRU013",
			"export class Concrete extends Subsystem {}\nexport class Bad extends Concrete implements Missing {}",
		],
		["CRU014", "interface Generic<T> { value: T; }\nexport const bad = inject<Generic<Missing>>();"],
	]) {
		const result = run(prefix + text);
		assert.ok(
			result.diagnostics.some((d) => d.code === 90000 + Number(code.slice(3))),
			code + " malformed input",
		);
		assert.ok(!result.diagnostics.some((d) => d.code === 90000), code + " never throws");
	}
	const definitions =
		"export interface Handler<T = string> { handle(value: T): void; }\ntype Alias = Handler<string>;\ninterface Mid extends Handler<number> {}\ninterface Transitive extends Mid {}\n";
	for (const target of ["Handler<string>", "Handler", "Alias", "Transitive"]) {
		for (const text of [
			"export class Bad extends Subsystem implements " + target + " { handle(value: any) {} }",
			"export let bad: All<" + target + ">;",
			"export const bad = inject<" + target + ">();",
			"export const bad = inject<All<" + target + ">>();",
		]) {
			const { diagnostics } = run(prefix + definitions + text);
			assert.ok(
				diagnostics.some(
					(d) =>
						d.code === 90014 &&
						d.category === ts.DiagnosticCategory.Error &&
						d.file.getLineAndCharacterOfPosition(d.start).line + 1 === 6,
				),
				target + text + JSON.stringify(diagnostics.map((d) => d.messageText)),
			);
		}
	}
	const control = run(
		prefix +
			'interface Plain { value: string; }\nexport const one = inject<Plain>();\nexport let all: All<Plain>;\nexport class Clean extends Subsystem { protected override update() {} helper() {} upXXte() {} }\nexport class CleanBehaviour extends CrucibleBehaviour<"Clean", Model> { Remove() {} isEnable() {} }',
	);
	assert.deepEqual(
		control.diagnostics.map((d) => d.messageText),
		[],
	);
});

test("T2 K1 tag forwarding written defaults and non-tag parameters", () => {
	const prefix =
		'import { CrucibleBehaviour, inject, type All } from "@trioforge/crucible";\n' +
		"export abstract class Openable<Tag extends string = string> extends CrucibleBehaviour<Tag, Model> {}\n" +
		"export abstract class Forward<Tag extends string = string> extends Openable<Tag> {}\n" +
		"export abstract class ForwardAgain<Tag extends string = string> extends Forward<Tag> {}\n";
	for (const target of ["Openable<string>", "Openable", "ForwardAgain<string>", "ForwardAgain"]) {
		const { diagnostics } = run(
			prefix + "export let all: All<" + target + ">;\nexport const one = inject<" + target + ">();",
		);
		assert.deepEqual(
			diagnostics.map((d) => d.messageText),
			[],
			target,
		);
	}
	assert.deepEqual(
		run(prefix + 'export class Door extends ForwardAgain<"Door"> {}').diagnostics.map((d) => d.messageText),
		[],
	);
	for (const target of ['Openable<"Door">', 'ForwardAgain<"Door">'])
		for (const text of ["export let all: All<" + target + ">;", "export const one = inject<" + target + ">();"]) {
			const { diagnostics } = run(prefix + text);
			assert.ok(
				diagnostics.some(
					(d) => d.code === 90014 && d.file.getLineAndCharacterOfPosition(d.start).line + 1 === 5,
				),
				text,
			);
		}
	const mixed =
		prefix +
		"export abstract class Mixed<Tag extends string, Other = number> extends ForwardAgain<Tag> { value!: Other; }\n";
	for (const text of [
		'export class Door extends Mixed<"Door"> {}',
		"export let all: All<Mixed<string>>;",
		"export const one = inject<Mixed<string>>();",
	])
		assert.ok(
			run(mixed + text).diagnostics.some((d) => d.code === 90014),
			text,
		);
});

test("T2 folder sides ignore directories above the common source root", () => {
	const file = path.join(game, "src/unmarked.ts");
	try {
		write(
			file,
			'import { Subsystem } from "@trioforge/crucible";\nexport class Unmarked extends Subsystem { protected override preRender() {} protected override onShutdown() {} }',
		);
		for (const options of [
			{},
			{ rootDir: undefined },
			{ rootDir: undefined, rootDirs: [path.join(game, "src")] },
		]) {
			const { diagnostics } = run("", options, {}, transform, true);
			const actual = diagnostics.filter((d) => path.normalize(d.file.fileName) === path.normalize(file));
			assert.deepEqual(actual.map((d) => d.code).sort(), [90011, 90016]);
			for (const diagnostic of actual) {
				assert.equal(diagnostic.category, ts.DiagnosticCategory.Error);
				assert.equal(diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start).line, 1);
			}
		}
	} finally {
		fs.rmSync(file, { force: true });
	}
});

test("T2 folder sides preserve side-named source roots", () => {
	const config = ts.getParsedCommandLineOfConfigFile(
		path.join(game, "tsconfig.json"),
		{},
		{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
	);
	for (const folder of ["server", "client", "shared"]) {
		const dir = path.join(game, "src", folder);
		fs.mkdirSync(dir, { recursive: true });
		const file = path.join(dir, "root-side.ts");
		try {
			for (const options of [
				{ rootDir: dir },
				{ rootDir: undefined, rootDirs: [dir] },
				{ rootDir: undefined },
				{ rootDir: undefined, configFilePath: undefined },
			]) {
				for (const hooks of ["allowed", "both"]) {
					const members =
						hooks === "both"
							? "protected override preRender() {} protected override onShutdown() {}"
							: folder === "server"
								? "protected override onShutdown() {}"
								: folder === "client"
									? "protected override preRender() {}"
									: "protected override update() {}";
					write(
						file,
						'import { Subsystem } from "@trioforge/crucible";\nexport class RootSide extends Subsystem { ' +
							members +
							" }",
					);
					const program = ts.createProgram([file], { ...config.options, ...options });
					const diagnostics = [];
					const result = ts.transform(program.getSourceFile(file), [
						transform(
							program,
							{ rojo: path.join(game, "default.project.json") },
							{ ts, addDiagnostic: (d) => diagnostics.push(d) },
						),
					]);
					result.dispose();
					const expected =
						hooks === "allowed"
							? []
							: folder === "server"
								? [90011]
								: folder === "client"
									? [90016]
									: [90011, 90016];
					assert.deepEqual(
						diagnostics.map((d) => d.code).sort(),
						expected,
						JSON.stringify({ folder, options, hooks }),
					);
					for (const diagnostic of diagnostics) {
						assert.equal(diagnostic.category, ts.DiagnosticCategory.Error);
						assert.equal(path.normalize(diagnostic.file.fileName), path.normalize(file));
						assert.equal(diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start).line, 1);
					}
				}
			}
		} finally {
			fs.rmSync(file, { force: true });
		}
	}
});

test("T2 folder sides include inherited members and nearest segment", () => {
	const prefix = 'import { Subsystem } from "@trioforge/crucible";\n';
	const files = [];
	try {
		for (const folder of ["client", "server", "shared", "server/client", "client/shared"]) {
			const dir = path.join(game, "src", folder);
			fs.mkdirSync(dir, { recursive: true });
			const file = path.join(dir, "side.ts");
			files.push(file);
			write(
				file,
				prefix +
					"export abstract class Base extends Subsystem { protected override preRender() {} static override renderPriority = 1; protected override onShutdown() {} }",
			);
		}
		const base = path.join(game, "src/client/base.ts");
		files.push(base);
		write(base, prefix + "export abstract class Foreign extends Subsystem { protected override preRender() {} }");

		const shutdownBase = path.join(game, "src/server/shutdown.ts");
		files.push(shutdownBase);
		write(
			shutdownBase,
			prefix + "export abstract class Shutdown extends Subsystem { protected override onShutdown() {} }",
		);
		const inherited = path.join(game, "src/client/inherited.ts");
		files.push(inherited);
		write(inherited, 'import { Shutdown } from "../server/shutdown";\nexport class Derived extends Shutdown {}');
		for (const [folder, member] of [
			["client", "protected override preRender() {}"],
			["server", "protected override onShutdown() {}"],
			["shared", "protected override update() {}"],
		]) {
			const file = path.join(game, "src", folder, "clean.ts");
			files.push(file);
			write(file, prefix + "export class Clean extends Subsystem { " + member + " }");
		}
		const malformed = path.join(game, "src/client/malformed.ts");
		files.push(malformed);
		write(
			malformed,
			prefix + "export class Malformed extends Subsystem { protected override onShutdown(value: Missing) {} }",
		);
		const result = run(
			'import { Foreign } from "../client/base";\nexport class Derived extends Foreign {}',
			{},
			{},
			transform,
			true,
		);

		for (const file of files.filter((file) => file.endsWith("clean.ts")))
			assert.deepEqual(
				result.diagnostics.filter((d) => path.normalize(d.file.fileName) === path.normalize(file)),
				[],
			);
		for (const file of [inherited, malformed])
			assert.ok(
				result.diagnostics.some(
					(d) =>
						path.normalize(d.file.fileName) === path.normalize(file) &&
						d.code === 90016 &&
						d.category === ts.DiagnosticCategory.Error &&
						d.file.getLineAndCharacterOfPosition(d.start).line === 1,
				),
			);
		assert.ok(!result.diagnostics.some((d) => d.code === 90000));
		for (const file of files.filter((file) => file.endsWith("side.ts"))) {
			const diagnostics = result.diagnostics.filter((d) => d.file.fileName === file.replaceAll("\\", "/"));
			const folder = path.basename(path.dirname(file));
			assert.equal(
				diagnostics.some((d) => d.code === 90011),
				folder !== "client",
				file,
			);
			assert.equal(
				diagnostics.some((d) => d.code === 90016),
				folder !== "server",
				file,
			);
		}
		assert.ok(
			result.diagnostics.some(
				(d) =>
					d.code === 90011 &&
					path.normalize(d.file.fileName) === path.normalize(main) &&
					d.file.getLineAndCharacterOfPosition(d.start).line === 1,
			),
		);
	} finally {
		for (const file of files) fs.rmSync(file, { force: true });
	}
});

test("T2 real rbxtsc reports all eight rules without a stack", () => {
	write(
		main,
		[
			'import { Subsystem, inject } from "@trioforge/crucible";',
			'import "@trioforge/crucible/emit";',
			"interface Promise<T> { value: T; }",
			"interface Generic<T = string> { value: T; }",
			"export class Concrete extends Subsystem { protected override preRender() {} onInit() {} field = inject<Subsystem>(); protected override start(): Promise<void> | undefined { return undefined; } }",
			"const Alias = Concrete;",
			'export class Derived extends Alias implements Generic { value = "value"; }',
		].join("\n"),
	);
	const client = path.join(game, "src/client/negative.ts");
	fs.mkdirSync(path.dirname(client), { recursive: true });
	write(
		client,
		'import { Subsystem } from "@trioforge/crucible"; export class Bad extends Subsystem { protected override onShutdown() {} }',
	);
	try {
		fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
		const result = spawnSync(
			process.execPath,
			[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", path.join(game, "tsconfig.json")],
			{ cwd: game, encoding: "utf8" },
		);
		assert.equal(result.status, 1, result.stdout + result.stderr);
		for (const code of ["CRU004", "CRU005", "CRU009", "CRU011", "CRU012", "CRU013", "CRU014", "CRU016"])
			assert.match(result.stdout + result.stderr, new RegExp(code));
		assert.doesNotMatch(result.stdout + result.stderr, /at transformer|at inspect|at visit|Node\.js v/);
	} finally {
		fs.rmSync(client, { force: true });
	}
});

test("T2 K1 constructor collections share generic validation", () => {
	const result = run(
		'import { Subsystem, CrucibleBehaviour, type All } from "@trioforge/crucible";\nexport abstract class Openable<Tag extends string = string> extends CrucibleBehaviour<Tag> {}\nexport class Clean extends Subsystem { constructor(readonly items: All<Openable>) { super(); } }',
	);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
});

test("rbxtsc Van attribute reads retain types and checks use dot calls", () => {
	const alias = path.join(game, "src/server/attribute-types.ts");
	write(alias, "export type Tint = Color3;");
	write(
		main,
		`import { attribute, CrucibleBehaviour } from "@trioforge/crucible";
import type { Tint } from "./attribute-types";
export class Van extends CrucibleBehaviour<"Van", Model> {
 readonly fleetId = attribute<string>();
 readonly maxSpeed = attribute(30);
 readonly label = attribute<string | undefined>(undefined);
 readonly tint: Tint = attribute(new Color3());
 readonly accent = attribute<Tint>(new Color3());
 readonly offset = attribute(new Vector3());
 readonly mode = attribute<"walk" | "drive">("walk");
 protected override fixedUpdate(step: number) {
  const root = this.instance.PrimaryPart;
  if (root === undefined) return;
  const velocity = root.AssemblyLinearVelocity;
  if (velocity.Magnitude > this.maxSpeed) root.AssemblyLinearVelocity = velocity.Unit.mul(this.maxSpeed);
  print(this.fleetId + "-", this.fleetId.size(), this.fleetId.sub(1, 2), this.label ?? "none", this.tint.R, this.accent.R, this.offset.Magnitude, step);
 }
}`,
	);
	try {
		fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
		const result = spawnSync(
			process.execPath,
			[
				path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"),
				"-p",
				path.join(game, "tsconfig.json"),
				"--writeTransformedFiles",
			],
			{ cwd: game, encoding: "utf8" },
		);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		const luau = fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8");
		assert.match(luau, /__crucible.t.optional\(/);
		assert.match(luau, /__crucible.t.literal\(/);
		assert.match(luau, /__crucible.t.union\(/);
		assert.doesNotMatch(luau, /__crucible.t:(?:optional|literal|union)\(/);
	} finally {
		fs.rmSync(alias);
	}
});

test("rbxtsc inferred attribute aliases resolve outside the emitting file scope", () => {
	const modes = path.join(game, "src/server/modes.ts");
	write(modes, 'export type Mixed = "a" | 3; export const DEFAULT_MIXED: Mixed = "a";');
	write(
		main,
		`import { attribute, CrucibleBehaviour } from "@trioforge/crucible";
import { DEFAULT_MIXED } from "./modes";
export class Vehicle extends CrucibleBehaviour<"Vehicle"> {
 readonly mixed = attribute(DEFAULT_MIXED);
}`,
	);
	try {
		fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
		const result = spawnSync(
			process.execPath,
			[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", path.join(game, "tsconfig.json")],
			{ cwd: game, encoding: "utf8" },
		);
		assert.equal(result.status, 0, result.stdout + result.stderr);
	} finally {
		fs.rmSync(modes);
	}
});

test("rbxtsc abstract attributes compile across base and concrete modules", () => {
	const base = path.join(game, "src/server/base.ts");
	write(
		base,
		`import { attribute, CrucibleBehaviour } from "@trioforge/crucible";
export abstract class Openable<Tag extends string> extends CrucibleBehaviour<Tag, Model> {
 readonly closedYaw = attribute(0);
 readonly key = attribute<string>();
}`,
	);
	write(main, 'import { Openable } from "./base"; export class Door extends Openable<"Door"> {}');
	try {
		fs.rmSync(path.join(game, "out"), { recursive: true, force: true });
		const result = spawnSync(
			process.execPath,
			[path.join(repo, "node_modules/roblox-ts/out/CLI/cli.js"), "-p", path.join(game, "tsconfig.json")],
			{ cwd: game, encoding: "utf8" },
		);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		const baseLuau = fs.readFileSync(path.join(game, "out/server/base.luau"), "utf8");
		const doorLuau = fs.readFileSync(path.join(game, "out/server/main.luau"), "utf8");
		assert.match(baseLuau, /local __crucible =/);
		assert.match(baseLuau, /__crucible.attribute\(self, "closedYaw", 0\)/);
		assert.match(baseLuau, /__crucible.attribute\(self, "key"\)/);
		assert.doesNotMatch(baseLuau, /defineClass\(/);
		assert.match(doorLuau, /defineClass\(Door/);
		assert.match(doorLuau, /name = "ClosedYaw"/);
		assert.match(doorLuau, /name = "Key"/);
	} finally {
		fs.rmSync(base);
	}
});

const sideHarness = require("./side-harness.cjs");
for (const c of require("./fixtures/side-cases.cjs")) {
	test(`T2 package sides f4 ${c.name}`, () => {
		const r = sideHarness.run(c);
		assert.equal(r.thrown, undefined);
		assert.deepEqual(
			r.diags.map((d) => [d.code, d.line, d.file, d.category]).sort(),
			c.expect.map(([code, line, file]) => [code, line, file, 1]).sort(),
		);
	});
}

test("T2 package sides layouts A B C D and f5 configurations", () => {
	const H = 'import { Subsystem } from "@trioforge/crucible";\n';
	const files = {
		"src/server/combat/main.ts": H + "export class S extends Subsystem { protected override onShutdown() {} }",
		"src/server/combat/r.ts": H + "export class R extends Subsystem { protected override preRender() {} }",
	};
	for (const [name, configPath, options, override] of [
		["A", "configs/tsconfig.json", { rootDir: "src" }, {}],
		["B", "src/server/tsconfig.json", { rootDir: "." }, {}],
		["C", "tsconfig.json", { rootDir: null }, { configFilePath: undefined }],
		["D rootDir", "tsconfig.json", { rootDir: "src/server/combat" }, {}],
		["D rootDirs", "tsconfig.json", { rootDir: null, rootDirs: ["src/server/combat"] }, {}],
		["f5 src", "tsconfig.json", { rootDir: "src" }, {}],
		["f5 server", "tsconfig.json", { rootDir: "src/server" }, {}],
		["f5 rootDirs", "tsconfig.json", { rootDir: null, rootDirs: ["src/server"] }, {}],
	]) {
		const dir = sideHarness.setup({ ancestor: "plain", files, compilerOptions: options });
		const original = path.join(dir, "tsconfig.json");
		if (name === "A") {
			fs.renameSync(original, path.join(dir, "tsconfig.base.json"));
			fs.mkdirSync(path.join(dir, "configs"));
			fs.writeFileSync(
				path.join(dir, configPath),
				JSON.stringify({ extends: "../tsconfig.base.json", include: ["../src"] }),
			);
		} else if (name === "B") {
			const config = JSON.parse(fs.readFileSync(original, "utf8"));
			config.include = ["."];
			config.compilerOptions.typeRoots = ["../../node_modules/@rbxts", "../../node_modules/@trioforge"];
			fs.writeFileSync(path.join(dir, configPath), JSON.stringify(config));
			fs.unlinkSync(original);
		}
		const r = sideHarness.transformAt(dir, configPath, override);
		assert.deepEqual(r.diags, ["CRU011@src/server/combat/r.ts:2"], name);
	}
});

test("T2 package sides exclude package folder and its parent", () => {
	for (const folder of ["client", "server", "shared"]) {
		const dir = sideHarness.setup({
			ancestor: folder,
			files: {
				"src/unmarked.ts":
					'import { Subsystem } from "@trioforge/crucible";\nexport class U extends Subsystem { protected override preRender() {} protected override onShutdown() {} }',
			},
		});
		for (const root of [dir, path.join(dir, "src")]) {
			fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ name: "nested-game" }));
			assert.deepEqual(sideHarness.transformAt(dir, "tsconfig.json").diags, [
				"CRU011@src/unmarked.ts:2",
				"CRU016@src/unmarked.ts:2",
			]);
		}
	}
});

test("T2 package sides exclude a side-named package root", () => {
	for (const folder of ["client", "server", "shared"]) {
		const file = `src/${folder}/main.ts`;
		const dir = sideHarness.setup({
			ancestor: "plain",
			files: {
				[file]: 'import { Subsystem } from "@trioforge/crucible";\nexport abstract class U extends Subsystem { protected override preRender() {} protected override onShutdown() {} }',
			},
		});
		fs.writeFileSync(
			path.join(dir, "src", folder, "package.json"),
			JSON.stringify(folder === "server" ? {} : { name: "nested" }),
		);
		assert.deepEqual(sideHarness.transformAt(dir, "tsconfig.json").diags, [`CRU011@${file}:2`, `CRU016@${file}:2`]);
	}
});

test("T2 package sides without a package search the whole path", () => {
	const { packageOf } = require("../type-id.cjs");
	const tempRoot =
		process.platform === "win32" ? path.join(process.env.SystemRoot, "Temp") : require("node:os").tmpdir();
	const dir = fs.mkdtempSync(path.join(tempRoot, "crucible-side-"));
	try {
		const game = sideHarness.setup({ ancestor: "plain", files: { "src/empty.ts": "export {};" } });
		fs.symlinkSync(path.join(game, "node_modules"), path.join(dir, "node_modules"), "junction");
		const file = path.join(dir, "server/combat/main.ts");
		assert.equal(packageOf(file), undefined);
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(
			file,
			'import { Subsystem } from "@trioforge/crucible";\nexport abstract class S extends Subsystem { protected override onShutdown() {} protected override preRender() {} }',
		);
		const config = ts.getParsedCommandLineOfConfigFile(
			path.join(game, "tsconfig.json"),
			{},
			{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
		);
		const program = ts.createProgram([file], {
			...config.options,
			rootDir: path.dirname(file),
			configFilePath: undefined,
		});
		const diagnostics = [];
		const result = ts.transform(program.getSourceFile(file), [
			transform(program, {}, { ts, addDiagnostic: (d) => diagnostics.push(d) }),
		]);
		result.dispose();
		assert.deepEqual(
			diagnostics.map((d) => [d.code, d.file.fileName, d.file.getLineAndCharacterOfPosition(d.start).line]),
			[[90011, file.replaceAll("\\", "/"), 1]],
		);
	} finally {
		fs.rmSync(dir, { recursive: true, force: true });
	}
});
