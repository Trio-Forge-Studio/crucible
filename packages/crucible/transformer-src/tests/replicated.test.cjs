const assert = require("node:assert/strict");
const { test } = require("node:test");
const h = require("./state-data-harness.cjs");
const {
	project,
	inspect,
	compile,
	buildDiagnostics,
	calls,
	evaluate,
	describe,
	countNodes,
	verdicts,
	normalPath,
	reported,
} = h;
const { fs, path, F, S, Nil, L, U, O, M } = h;
const fd = fs.readFileSync(path.join(h.fixture, "fd.ts"), "utf8");
const shapes = (name) => fs.readFileSync(path.join(h.fixture, name), "utf8");
const reactive = verdicts("reactive-lower.out");
const imports = 'import { replicated, replicatedPerPlayer, toServer, u8, Replicated } from "@trioforge/crucible";\n';
const declare = (type, name = "X.State", macro = "replicated", initial = "undefined as never") =>
	`export const Value = ${macro}<${type}>("${name}", ${initial});`;

function expectRejections(result, entries) {
	for (const [name, expected] of entries) {
		const mine = reported(result, name);
		assert.ok(mine.length, `${name} is rejected`);
		for (const { code, at } of mine)
			assert.ok(
				expected.some((e) => e.code === code && normalPath(at).startsWith(normalPath(e.at))),
				`${name} ${code} at ${at} matches the probe's ${JSON.stringify(expected)}`,
			);
	}
}

test("replicated rewrites keep the type argument and append the state descriptor", () => {
	const game = project({
		"src/shared/fd.ts": fd,
		"src/shared/state.ts":
			imports +
			`import type { OwnerData, PlayerData } from "./fd";
type JobPhaseName = "Waiting" | "Countdown" | "Intro" | "Running" | "Settling" | "Resetting";
export const JobPhase = replicated<JobPhaseName | undefined>("Job.Phase", undefined);
export const OwnerProfile = replicatedPerPlayer<OwnerData | undefined>("Data.Owner", undefined);
export const Rewards = replicated<{ coins: number }>("Job.Rewards", { coins: 3 });
declare const data: PlayerData;
export const wider = { ...data, played: { savedSeconds: 0, sessionStartedAt: 0 } };
export const leak: (player: Player) => OwnerData | undefined = () => wider;
`,
	});
	const result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const found = calls(result.output.get("src/shared/state.ts"), ["replicated", "replicatedPerPlayer"]);
	assert.deepEqual(found.get("Job.Phase"), {
		types: ["JobPhaseName | undefined"],
		args: [
			'"Job.Phase"',
			"undefined",
			U(Nil, ...["Countdown", "Intro", "Resetting", "Running", "Settling", "Waiting"].map(L)),
		],
	});
	assert.deepEqual(found.get("Job.Rewards"), {
		types: ["{ coins: number; }"],
		args: ['"Job.Rewards"', "{ coins: 3 }", O([["coins", F]])],
	});
	const owner = found.get("Data.Owner");
	assert.deepEqual(owner.types, ["OwnerData | undefined"]);
	assert.equal(owner.args[1], "undefined");
	const expected = reactive.get("Data.Owner").shape;
	const node = evaluate(owner.args[2]);
	assert.equal(`${countNodes(node)} nodes: ${describe(node)}`, `${expected.nodes} nodes: ${expected.text}`);
	assert.doesNotMatch(owner.args[2], /purchaseIds|redeemedCodes|analytics/);
	compile(game);
	const luau = fs.readFileSync(path.join(game, "out/shared/state.luau"), "utf8");
	assert.match(luau, /__crucible\.replicated\("Job\.Phase", nil, __crucible\.n\.union\(/);
	assert.match(luau, /__crucible\.replicatedPerPlayer\("Data\.Owner", nil, __crucible\.n\.union\(/);
});

test("replicated state lowering follows rule 42 over the reactive probe's shapes", () => {
	let game = project({ "src/shared/shapes.ts": shapes("shapes-positive.ts") });
	let result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const found = calls(result.output.get("src/shared/shapes.ts"), ["replicated"]);
	for (let i = 1; i <= 17; i++) {
		assert.equal(reactive.get(`P.${i}`).verdict, "lowers");
		assert.ok(found.has(`P.P${i}`), `P.${i} is rewritten`);
	}
	const value = (name) => found.get(name.replace(".", ".P")).args[2];
	assert.equal(value("P.13"), M(S, F));
	assert.equal(value("P.14"), M(S, F));
	assert.equal(
		value("P.15"),
		O([
			["a", F],
			["b", U(Nil, S)],
		]),
	);
	assert.equal(
		value("P.16"),
		O([
			["Game", F],
			["Music", F],
		]),
	);
	assert.equal(value("P.17"), M(U(L("Jobs"), L("Money")), F));
	assert.equal(
		value("P.10"),
		O([
			["c", "__crucible.n.color3"],
			["cf", "__crucible.n.cframe"],
			["tag", S],
		]),
	);
	compile(game);

	game = project({
		"src/shared/shapes.ts": shapes("shapes-negative.ts"),
		"src/shared/merge-a.ts":
			imports +
			'declare global { interface Merged { readonly a: number } } declare const merged: Merged; export const MergedDecl = replicated<Merged>("N.N30", merged);',
		"src/shared/merge-b.ts": "declare global { interface Merged { readonly b: string } } export {};",
	});
	result = inspect(game);
	const negatives = [...reactive.values()].filter((e) => e.name.startsWith("N."));
	assert.equal(negatives.length, 30);
	for (const entry of negatives) assert.equal(entry.verdict, "rejected");
	expectRejections(
		result,
		negatives.map((e) => [
			e.name.replace(".", ".N"),
			e.name === "N.30" ? [{ code: "CRU010", at: "value" }] : e.diagnostics,
		]),
	);
	assert.ok(
		h
			.diagnosticsIn(result, game, "src/shared/shapes.ts")
			.includes("CRU019: N.N17 value.by: replicated state cannot hold Instances; replicate a stable id"),
	);
	assert.ok(
		h
			.diagnosticsIn(result, game, "src/shared/shapes.ts")
			.includes(
				"CRU019: N.N19 value: unknown cannot be stored or replicated state; give the value a declared type",
			),
	);
	const log = compile(game, 1);
	for (const code of ["CRU019", "CRU010"]) assert.match(log, new RegExp(code));
});

test("replicated depth stops at SYNC_VALUE_DEPTH where a payload stops at MAX_DEPTH", () => {
	const nested = (n) => "{v:".repeat(n) + "string" + "}".repeat(n);
	let game = project({
		"src/shared/depth.ts":
			imports +
			`export const Deep = replicated<${nested(27)}>("X.Deep", undefined as never);\nexport const Payload = toServer<${nested(31)}>("X.Payload");`,
	});
	assert.deepEqual(
		inspect(game).diagnostics.map((d) => d.messageText),
		[],
	);
	game = project({ "src/shared/depth.ts": imports + declare(nested(28), "X.Deep") });
	assert.match(
		inspect(game)
			.diagnostics.map((d) => d.messageText)
			.join("\n"),
		/CRU019: X\.Deep value(\.v)+: schema exceeds the depth limit of 28/,
	);
	compile(game, 1);
});

const declarationNegatives = [
	["local const", "CRU017", 'const X = replicated<string>("X.State", "");'],
	["let", "CRU017", 'export let X = replicated<string>("X.State", "");'],
	["nested", "CRU017", 'export function make() { return replicated<string>("X.State", ""); }'],
	["not exported", "CRU017", 'const X = replicatedPerPlayer<string>("X.State", ""); export { X };'],
	["missing type", "CRU017", 'export const X = replicated("X.State", undefined as never);'],
	["two types", "CRU017", 'export const X = replicated<string, number>("X.State", "");'],
	["extra argument", "CRU017", 'export const X = replicated<string>("X.State", "", 1 as never);'],
	["name variable", "CRU018", 'const n = "X.State"; export const X = replicated<string>(n, "");'],
	...["", "1.Bad", "A..B", "A-b", "Crucible", "Crucible.State", "X." + "a".repeat(63)].map((name) => [
		`invalid name ${JSON.stringify(name)}`,
		"CRU018",
		`export const X = replicated<string>(${JSON.stringify(name)}, "");`,
	]),
	["macro value", "CRU023", "export const f = replicated;"],
	["macro parameter", "CRU023", "function use(f: unknown) {} use(replicatedPerPlayer);"],
	["namespace value", "CRU023", 'import * as C from "@trioforge/crucible"; export const f = C.replicated;'],
	["bracket value", "CRU023", 'import * as C from "@trioforge/crucible"; export const f = C["replicatedPerPlayer"];'],
	["server state", "CRU019: X.State value", declare("ServerOnlyLike", "X.State")],
	[
		"state width key",
		"CRU019: X.State value.key: state maps are keyed by string",
		declare("ReadonlyMap<u8, string>"),
	],
	[
		"state number literal key",
		"CRU019: X.State value.key: state maps are keyed by string",
		declare("ReadonlyMap<1 | 2, string>"),
	],
];

test("replicated declarations take networking's declaration, name and escape checks", () => {
	const files = {};
	for (let i = 0; i < declarationNegatives.length; i++)
		files[`src/shared/n${i}.ts`] =
			imports +
			'import type { ServerOnly } from "@trioforge/crucible"; type ServerOnlyLike = { x: ServerOnly<number> };\n' +
			declarationNegatives[i][2];
	for (const side of ["server", "client", "neutral"])
		files[`src/${side}/wrong.ts`] = imports + declare("string", `X.${side}`, "replicated", '""');
	files["src/shared/a.ts"] = imports + declare("string", "X.Twice", "replicated", '""');
	files["src/shared/b.ts"] = imports + declare("number", "X.Twice", "replicatedPerPlayer", "0");
	files["src/shared/message.ts"] = imports + 'export const Twice = toServer<string>("X.Twice");';
	files["src/shared/limit.ts"] = imports + declare("string", "X." + "a".repeat(62), "replicated", '""');
	const game = project(files);
	const result = inspect(game);
	for (let i = 0; i < declarationNegatives.length; i++) {
		const [name, code] = declarationNegatives[i];
		assert.ok(
			h.diagnosticsIn(result, game, `src/shared/n${i}.ts`).some((m) => m.startsWith(code)),
			`${name} reports ${code}: ${h.diagnosticsIn(result, game, `src/shared/n${i}.ts`).join("\n")}`,
		);
	}
	for (const side of ["server", "client", "neutral"])
		assert.deepEqual(h.diagnosticsIn(result, game, `src/${side}/wrong.ts`), [
			"CRU017: Declare replicated state as an exported top-level const in a shared module.",
		]);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/a.ts"), [
		"CRU018: X.Twice is already declared at src/shared/b.ts:2.",
	]);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/b.ts"), [
		"CRU018: X.Twice is already declared at src/shared/a.ts:2.",
	]);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/message.ts"), []);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/limit.ts"), []);
	const log = compile(game, 1);
	for (const code of ["CRU017", "CRU018", "CRU019", "CRU023"]) assert.match(log, new RegExp(code));
});

test("replicated declarations report CRU010 for global, merged and augmented types they reach", () => {
	const game = project({
		"src/global.d.ts": "interface ScriptState { value: string }\n",
		"src/shared/global.ts": imports + declare("{ nested: ScriptState }", "X.Global"),
		"src/shared/type.ts": "export interface Score { points: number }",
		"src/shared/merged.ts": imports + 'import { Score } from "./type"; ' + declare("Score", "X.Merged"),
		"src/augment.ts":
			'import "./shared/type"; declare module "./shared/type" { interface Score { extra: string } }',
		"src/shared/clean.ts": "export interface Clean { points: number }",
		"src/shared/untouched.ts": imports + 'import { Clean } from "./clean"; ' + declare("Clean", "X.Clean"),
	});
	const result = inspect(game);
	const files = [
		...new Set(
			result.diagnostics
				.filter((d) => d.code === 90010)
				.map((d) => path.relative(game, d.file.fileName).replaceAll("\\", "/")),
		),
	].sort();
	assert.deepEqual(files, ["src/augment.ts", "src/shared/global.ts", "src/shared/merged.ts"]);
	assert.deepEqual(buildDiagnostics(compile(game, 1), "CRU010"), files);
});

test("K9 the build rejects a replicated initial value that breaks the declared type", () => {
	let game = project({
		"src/shared/state.ts": imports + 'export const X = replicated<{ a: number }>("X.State", { a: "s" });',
	});
	assert.match(compile(game, 1), /TS2322|error TS/);
	game = project({
		"src/shared/state.ts": imports + 'export const X = replicatedPerPlayer<{ a: number }>("X.State", { a: "s" });',
	});
	assert.deepEqual(
		inspect(game).diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(compile(game, 1), /TS2322|TS2345/);
	game = project({
		"src/shared/state.ts": imports + 'export const X = replicatedPerPlayer<{ a: number }>("X.State", { a: 1 });',
		"src/client/read.ts": 'import { X } from "../shared/state"; export const a: number = X().a;',
	});
	compile(game);
});
