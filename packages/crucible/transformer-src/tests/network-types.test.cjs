const assert = require("node:assert/strict");
const { test } = require("node:test");
const { project, inspect, compile, fs, path, ts } = require("./network-harness.cjs");

test("K1 K16 R5 pinned rbxtsc type fixtures retain literal inference invariance and target callback errors", () => {
	for (const [name, expected] of [
		["pos", 0],
		["neg", 20],
		["v1lit", 6],
	]) {
		const original = fs.readFileSync(path.join(__dirname, "fixtures/network", name + ".ts.txt"), "utf8");
		const game = project(
			{ "src/shared/probe.ts": original.replace('"./net"', '"@trioforge/crucible"') },
			{ plugins: [] },
		);
		const net = fs.readFileSync(path.join(__dirname, "fixtures/network/net.d.ts"), "utf8");
		const sf = ts.createSourceFile("net.d.ts", net, ts.ScriptTarget.Latest, true);
		const extra = sf.statements
			.filter((s) =>
				["ServerNetwork", "ClientNetwork", "V1Server", "V1Client", "receiveV1"].includes(s.name?.text),
			)
			.map((s) => s.getText(sf))
			.join("\n");
		fs.appendFileSync(path.join(game, "node_modules/@trioforge/crucible/out/network.d.ts"), "\n" + extra);
		const { program } = inspect(game);
		const source = program.getSourceFile(path.join(game, "src/shared/probe.ts"));
		const diagnostics = program.getSemanticDiagnostics(source);
		assert.equal(
			diagnostics.length,
			expected,
			diagnostics.map((d) => ts.flattenDiagnosticMessageText(d.messageText, "\n")).join("\n"),
		);
		if (name === "neg") {
			for (const line of [12, 13, 14, 15, 16, 17, 18, 20, 23, 24, 25, 26, 27, 28])
				assert.ok(
					diagnostics.some((d) => source.getLineAndCharacterOfPosition(d.start).line + 1 === line),
					`negative probe line ${line}`,
				);
			assert.match(
				ts.flattenDiagnosticMessageText(
					diagnostics.find((d) => source.getLineAndCharacterOfPosition(d.start).line + 1 === 23).messageText,
					"\n",
				),
				/userId.*name/,
			);
		}
		compile(game, expected ? 1 : 0);
	}
});
