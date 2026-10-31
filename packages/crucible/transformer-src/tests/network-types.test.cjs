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

test("network token types reject both variance directions receiver calls and ClientReady sends", () => {
	const game = project({
		"src/client/tokens.ts": `
import {ToServer,Receiver,ClientReady} from "@trioforge/crucible";
import {Network} from "@trioforge/crucible/client";
declare const narrow: ToServer<{x:number}>;
declare const wide: ToServer<{x:number|string}>;
declare const receiver: Receiver;
export const covariant: typeof wide = narrow;
export const contravariant: typeof narrow = wide;
receiver();
Network.send(ClientReady);
`,
	});
	const { program, diagnostics } = inspect(game);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	const source = program.getSourceFile(path.join(game, "src/client/tokens.ts"));
	const codes = program
		.getSemanticDiagnostics(source)
		.map((d) => d.code)
		.sort();
	assert.deepEqual(codes, [2322, 2322, 2349, 2554]);
	compile(game, 1);
});

test("K2 rule 25 unknown responses accept an empty callback and typed responses report TS2345 only before the receive rewrite", () => {
	const messages =
		'import { askServer } from "@trioforge/crucible"; export const Query = askServer<undefined, unknown>("X.Query"); export const Typed = askServer<undefined, string | undefined>("X.Typed");';
	const receiver = (token) =>
		`import { receive, Subsystem } from "@trioforge/crucible"; import { ${token} } from "../shared/messages"; export class Q extends Subsystem { readonly r = receive(${token}, () => {}); }`;
	const check = (token) => {
		const game = project({ "src/shared/messages.ts": messages, "src/server/receiver.ts": receiver(token) });
		const { program, diagnostics } = inspect(game);
		assert.deepEqual(
			diagnostics.map((d) => d.messageText),
			[],
		);
		const source = program.getSourceFile(path.join(game, "src/server/receiver.ts"));
		return {
			game,
			semantic: program
				.getSemanticDiagnostics(source)
				.map((d) => [d.code, ts.flattenDiagnosticMessageText(d.messageText, " ")]),
		};
	};
	const unknown = check("Query");
	assert.deepEqual(unknown.semantic, []);
	compile(unknown.game);
	const typed = check("Typed");
	assert.deepEqual(typed.semantic, [
		[
			2345,
			"Argument of type '() => void' is not assignable to parameter of type '(player: Player) => string | undefined'.   Type 'void' is not assignable to type 'string | undefined'.",
		],
	]);
	// rbxtsc rechecks the transformed text, where the emit receive takes an untyped callback.
	compile(typed.game, 0);
});
