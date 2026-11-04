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
	const config = ts.getParsedCommandLineOfConfigFile(
		`${sample}/tsconfig.json`,
		{},
		{
			...ts.sys,
			onUnRecoverableConfigFileDiagnostic: (diagnostic) =>
				assert.fail(ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")),
		},
	);
	const options = { ...config.options, noEmit: true, incremental: false, tsBuildInfoFile: undefined, plugins: [] };
	const host = ts.createCompilerHost(options);
	const { fileExists, readFile, getSourceFile } = host;
	host.fileExists = (file) => files.has(file) || fileExists(file);
	host.readFile = (file) => files.get(file) ?? readFile(file);
	host.getSourceFile = (file, language, ...rest) =>
		files.has(file) ? ts.createSourceFile(file, files.get(file), language) : getSourceFile(file, language, ...rest);
	const program = ts.createProgram([...files.keys()], options, host);
	const diagnostics = [...files.keys()].flatMap((file) =>
		ts.getPreEmitDiagnostics(program, program.getSourceFile(file)),
	);
	assert.equal(
		diagnostics.length,
		0,
		ts.formatDiagnostics(diagnostics, { ...host, getNewLine: () => "\n", getCanonicalFileName: (file) => file }),
	);
});

const read = (file) => fs.readFileSync(file, "utf8").replaceAll("\r\n", "\n");
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
