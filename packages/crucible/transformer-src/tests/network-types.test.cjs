const assert = require("node:assert/strict");
const { test } = require("node:test");
const { stripVTControlCharacters } = require("node:util");
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

test("K2 K9 rule 25 unknown responses accept an empty callback and the build rejects callbacks that break their target's types", () => {
	const messages =
		'import { askServer, toServer } from "@trioforge/crucible"; export const Query = askServer<undefined, unknown>("X.Query"); export const Typed = askServer<undefined, string | undefined>("X.Typed"); export const Event = toServer<string>("X.Event"); export const Ask = askServer<string, number>("X.Ask");';
	const receiver = (field) =>
		`import { receive, Subsystem, ClientReady, ToServer } from "@trioforge/crucible"; import { Query, Typed, Event, Ask } from "../shared/messages"; export class Q extends Subsystem { readonly r = ${field}; }`;
	const semantic = (program, game, file) =>
		program
			.getSemanticDiagnostics(program.getSourceFile(path.join(game, file)))
			.map((d) => [d.code, ts.flattenDiagnosticMessageText(d.messageText, "\n")]);
	const positive = project({
		"src/shared/messages.ts": messages,
		"src/server/query.ts": receiver("receive(Query, () => {})"),
	});
	const checked = inspect(positive);
	assert.deepEqual(
		checked.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.deepEqual(semantic(checked.program, positive, "src/server/query.ts"), []);
	compile(positive);
	const negatives = {
		"src/server/typed.ts": [
			"receive(Typed, () => {})",
			2345,
			"Argument of type '() => void' is not assignable to parameter of type '(player: Player) => string | undefined'.\n  Type 'void' is not assignable to type 'string | undefined'.",
		],
		"src/server/event.ts": [
			"receive(Event, (p: Player, s: number) => {})",
			2345,
			"Argument of type '(p: Player, s: number) => void' is not assignable to parameter of type '(player: Player, payload: string) => void'.\n  Types of parameters 's' and 'payload' are incompatible.\n    Type 'string' is not assignable to type 'number'.",
		],
		"src/server/ready.ts": [
			"receive(ClientReady, (p: string) => {})",
			2345,
			"Argument of type '(p: string) => void' is not assignable to parameter of type '(player: Player) => void | (() => void)'.\n  Types of parameters 'p' and 'player' are incompatible.\n    Type 'Player' is not assignable to type 'string'.",
		],
		"src/server/ask.ts": ["receive(Ask, (p, s) => s)", 2322, "Type 'string' is not assignable to type 'number'."],
		"src/server/explicit.ts": [
			"receive<ToServer<number>>(Event, (p, n) => {})",
			2345,
			"Argument of type 'ToServer<string>' is not assignable to parameter of type 'ToServer<number>'.\n  The types of '[message].payload' are incompatible between these types.\n    Type '(value: string) => string' is not assignable to type '(value: number) => number'.\n      Types of parameters 'value' and 'value' are incompatible.\n        Type 'number' is not assignable to type 'string'.",
		],
	};
	const files = { "src/shared/messages.ts": messages };
	for (const [file, [field]] of Object.entries(negatives)) files[file] = receiver(field);
	const game = project(files);
	const { program, diagnostics } = inspect(game);
	assert.deepEqual(
		diagnostics.map((d) => d.messageText),
		[],
	);
	const output = stripVTControlCharacters(compile(game, 1)).replaceAll("\r\n", "\n");
	assert.equal(output.match(/ error TS/g)?.length, 5, output);
	for (const [file, [, code, message]] of Object.entries(negatives)) {
		assert.deepEqual(semantic(program, game, file), [[code, message]]);
		const reported = output.slice(output.indexOf(`${file}:`)).replace(/^\S+ - /, "");
		assert.ok(
			output.includes(`${file}:`) && reported.startsWith(`error TS${code}: ${message}`),
			`${file}\n${output}`,
		);
	}
});
