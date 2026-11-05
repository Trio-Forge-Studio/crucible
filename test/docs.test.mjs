import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import ts from "typescript";

const repo = path.resolve(import.meta.dirname, "..");
const sample = path.join(repo, "examples/sample-game").replaceAll("\\", "/");

function codeBlocks() {
	const files = new Map();
	for (const name of fs.readdirSync(path.join(repo, "docs"))) {
		if (!name.endsWith(".md") || name === "sample-game.md") continue;
		const markdown = fs.readFileSync(path.join(repo, "docs", name), "utf8");
		const blocks = [...markdown.matchAll(/^```(ts|typescript|tsx)\n([\s\S]*?)^```$/gm)];
		blocks.forEach((block, index) =>
			files.set(
				`${sample}/src/docs/${path.basename(name, ".md")}-block-${index + 1}.${block[1] === "tsx" ? "tsx" : "ts"}`,
				block[2],
			),
		);
	}
	return files;
}

test("TypeScript blocks in docs type-check against the runtime package", () => {
	const files = codeBlocks();
	assert.ok(
		[...files.keys()].some((file) => file.includes("/logging-block-")),
		"the logging guide has ts blocks",
	);
	assert.ok(
		[...files.keys()].some((file) => file.includes("/vide-block-") && file.endsWith(".tsx")),
		"the vide guide has tsx blocks",
	);
	assert.ok(
		[...files.keys()].some((file) => file.includes("/ui-block-") && file.endsWith(".tsx")),
		"the ui guide has tsx blocks",
	);
	for (const guide of ["random", "spline"])
		assert.ok(
			[...files.keys()].some((file) => file.includes(`/${guide}-block-`)),
			`the ${guide} guide has ts blocks`,
		);
	const config = ts.getParsedCommandLineOfConfigFile(
		`${sample}/tsconfig.json`,
		{},
		{
			...ts.sys,
			onUnRecoverableConfigFileDiagnostic: (diagnostic) =>
				assert.fail(ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")),
		},
	);
	const base = { ...config.options, noEmit: true, incremental: false, tsBuildInfoFile: undefined, plugins: [] };
	// The ui guide's blocks use Crucible UI's JSX, and every other tsx block is a Vide component.
	const ui = new Map([...files].filter(([file]) => file.includes("/ui-block-")));
	const rest = new Map([...files].filter(([file]) => !ui.has(file)));
	for (const [blocks, jsx] of [
		[rest, { jsx: ts.JsxEmit.React, jsxFactory: "Vide.jsx", jsxFragmentFactory: "Vide.Fragment" }],
		[ui, { jsx: ts.JsxEmit.ReactJSX, jsxImportSource: "@trioforge/crucible/ui" }],
	]) {
		const options = { ...base, ...jsx };
		const host = ts.createCompilerHost(options);
		const { fileExists, readFile, getSourceFile } = host;
		host.fileExists = (file) => blocks.has(file) || fileExists(file);
		host.readFile = (file) => blocks.get(file) ?? readFile(file);
		host.getSourceFile = (file, language, ...rest) =>
			blocks.has(file)
				? ts.createSourceFile(file, blocks.get(file), language)
				: getSourceFile(file, language, ...rest);
		const program = ts.createProgram([...blocks.keys()], options, host);
		const diagnostics = [...blocks.keys()].flatMap((file) =>
			ts.getPreEmitDiagnostics(program, program.getSourceFile(file)),
		);
		assert.equal(
			diagnostics.length,
			0,
			ts.formatDiagnostics(diagnostics, {
				...host,
				getNewLine: () => "\n",
				getCanonicalFileName: (file) => file,
			}),
		);
	}
});

const read = (file) => fs.readFileSync(file, "utf8").replaceAll("\r\n", "\n");

const cru = (number) => `CRU${String(number).padStart(3, "0")}`;
const sorted = (codes) => [...new Set(codes)].sort();
// Crucible UI builds only with the Crucible compiler, so the rbxtsc transformer never reports its class codes.
const compilerOnly = ["CRU028", "CRU029", "CRU030", "CRU031"];

test("the diagnostics guide lists every CRU code the transformer and the compiler report, in order", () => {
	const transformerDir = path.join(repo, "packages/crucible/transformer-src");
	const transformer = sorted(
		fs
			.readdirSync(transformerDir)
			.filter((name) => name.endsWith(".cjs"))
			.flatMap((name) => [...read(path.join(transformerDir, name)).matchAll(/"(CRU\d{3})"/g)].map((m) => m[1])),
	);
	const compilerDir = path.join(repo, "cli/crates/compiler/src");
	const compiler = sorted(
		fs
			.readdirSync(compilerDir, { recursive: true })
			.filter((name) => name.endsWith(".rs"))
			.flatMap((name) => {
				const code = read(path.join(compilerDir, name));
				return [
					...code.matchAll(/Code::Rule\((\d+)\)/g),
					...code.matchAll(/\bfail_with\((?:"(?:[^"\\]|\\.)*"|[^;"])*?,\s*(\d+),?\s*\)/g),
				].map((m) => cru(m[1]));
			}),
	);
	const headings = [...read(path.join(repo, "docs/diagnostics.md")).matchAll(/^## (CRU\d{3})$/gm)].map((m) => m[1]);
	assert.ok(transformer.length > 0 && compiler.length > 0, "found CRU codes in both rule sources");
	assert.deepEqual(
		compiler,
		sorted([...transformer, ...compilerOnly]),
		"the compiler reports the transformer's CRU codes plus the Crucible UI class codes",
	);
	assert.deepEqual(headings, sorted(headings), "docs/diagnostics.md lists its CRU headings out of order or twice");
	assert.deepEqual(
		headings,
		compiler,
		"docs/diagnostics.md headings differ from the CRU codes the transformer and the compiler report",
	);
});
const declared = (code, call) =>
	[...code.matchAll(new RegExp(String.raw`\b${call}(?:<[^(]*>)?\("([^"]+)"`, "g"))].map((match) => match[1]);

function sampleGuide() {
	const markdown = read(path.join(repo, "docs/sample-game.md"));
	const snippets = [...markdown.matchAll(/^`(src\/[^`]+\.ts)`\n\n```ts\n([\s\S]*?)^```$/gm)].map((match) => ({
		file: match[1],
		code: match[2],
		source: read(path.join(repo, "examples/sample-game", match[1])),
	}));
	assert.equal(snippets.length, (markdown.match(/^```ts$/gm) ?? []).length, "every ts block names its file");
	return snippets;
}

test("every sample guide snippet is its sample file, so the guide cannot drift from the code", () => {
	const snippets = sampleGuide();
	assert.deepEqual(
		snippets.map(({ file }) => file),
		[
			"src/shared/messages.ts",
			"src/shared/state.ts",
			"src/shared/profile.ts",
			"src/server/systems/event.ts",
			"src/server/systems/wallet.ts",
			"src/client/systems/hud.ts",
		],
	);
	for (const { file, code, source } of snippets) assert.equal(code, source, `${file} differs from its sample file`);
});

test("the sample guide declares each store and message once, under the name the sample code uses", () => {
	const snippets = sampleGuide();
	const guide = snippets.map(({ code }) => code).join("\n");
	assert.deepEqual(declared(guide, "profileStore"), ["Sample.Players"]);
	assert.deepEqual(declared(guide, "toServer"), ["Coins.Collect"]);
	assert.deepEqual(declared(guide, "toClient"), ["Coins.Collected"]);
	assert.deepEqual(declared(guide, "replicated"), ["Coins.Total"]);
	assert.deepEqual(declared(guide, "config"), ["GlobalEvent"]);
	assert.match(snippets.find(({ file }) => file === "src/shared/profile.ts").code, /migrations: \[fromGold\]/);
	for (const { file, code } of snippets) {
		const imports = code.split("\n").filter((line) => line.startsWith("import "));
		assert.equal(new Set(imports).size, imports.length, `${file} repeats an import line`);
		if (file !== "src/shared/profile.ts")
			assert.doesNotMatch(code, /profileStore\(/, `${file} redeclares the store`);
	}
	for (const { file, code } of snippets.filter(({ file }) => /\/(wallet|hud)\.ts$/.test(file))) {
		assert.match(code, /from "\.\.\/\.\.\/shared\//, `${file} imports the shared declarations`);
	}
});
