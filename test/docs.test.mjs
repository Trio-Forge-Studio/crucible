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
		if (!name.endsWith(".md")) continue;
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
