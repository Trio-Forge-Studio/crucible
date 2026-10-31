const assert = require("node:assert/strict");
const { test } = require("node:test");
const { project, inspect, compile, descriptorExpressions, fs, path, ts } = require("./network-harness.cjs");
const imports =
	'import { toServer, toClient, askServer, receive, ClientReady, Subsystem, CrucibleBehaviour, u8, u16, u32, f32, f64, ToServer, ToClient, AskServer, ReceiveTarget } from "@trioforge/crucible";\n';
const declaration = (type, kind = "toServer", name = "X.Test") => `export const Message = ${kind}<${type}>("${name}");`;

test("network descriptors recheck with precise builder types and preserve request response order", () => {
	const game = project({
		"src/shared/messages.ts":
			imports +
			`
export const A = toServer<{ zebra: u8; alpha: string; flag: boolean }>("X.Object");
export const B = askServer<string, u16>("X.Ask", { timeout: 37 });
export const C = askServer<undefined, unknown>("X.Default");
export const D = toServer<Map<"z" | "a", number>>("X.Map");
export const E = toServer<[string, u32]>("X.Tuple");
export const F = toClient<{ maybe: BasePart | undefined; player: Player }>("X.Stream");
export const G = toServer<{ Kind: "Leave" } | { Kind: "Join"; UserId: number }>("X.Variant");
export const H = toServer<"z" | "a" | 7 | 2 | false>("X.Literals");
`,
	});
	const result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const d = descriptorExpressions(result.output.get("src/shared/messages.ts"));
	assert.equal(
		d.get("X.Object")[0],
		'__crucible.n.object([["alpha", __crucible.n.string], ["flag", __crucible.n.bool], ["zebra", __crucible.n.u8]])',
	);
	assert.deepEqual(d.get("X.Ask"), ["__crucible.n.string", "__crucible.n.u16", "37"]);
	assert.deepEqual(d.get("X.Default"), ["__crucible.n.nil", "__crucible.n.dynamic", "undefined"]);
	assert.equal(
		d.get("X.Map")[0],
		'__crucible.n.map(__crucible.n.union([__crucible.n.literal("a"), __crucible.n.literal("z")]), __crucible.n.f64)',
	);
	assert.equal(d.get("X.Tuple")[0], "__crucible.n.tuple([__crucible.n.string, __crucible.n.u32])");
	assert.equal(
		d.get("X.Literals")[0],
		'__crucible.n.union([__crucible.n.literal(false), __crucible.n.literal(2), __crucible.n.literal(7), __crucible.n.literal("a"), __crucible.n.literal("z")])',
	);
	assert.match(d.get("X.Stream")[0], /union\(\[__crucible.n.nil, __crucible.n.instance\(\["BasePart"\]\)\]\)/);
	assert.equal(
		d.get("X.Variant")[0],
		'__crucible.n.union([__crucible.n.variant("Kind", "Join", __crucible.n.object([["UserId", __crucible.n.f64]])), __crucible.n.variant("Kind", "Leave", __crucible.n.object([]))])',
	);
	compile(game);
	const luau = fs.readFileSync(path.join(game, "out/shared/messages.luau"), "utf8");
	assert.match(luau, /askServer\("X.Ask", __crucible.n.string, __crucible.n.u16, 37\)/);
});

const negatives = [
	["local const", "CRU017", 'const X = toServer<string>("X.Test");'],
	["let", "CRU017", 'export let X = toServer<string>("X.Test");'],
	["nested", "CRU017", 'export function make() { return toServer<string>("X.Test"); }'],
	["missing type", "CRU017", 'export const X = toServer("X.Test");'],
	[
		"options variable",
		"CRU017",
		'const o = { timeout: 3 }; export const X = askServer<string, number>("X.Test", o);',
	],
	["options empty", "CRU017", 'export const X = askServer<string, number>("X.Test", {});'],
	["timeout zero", "CRU017", 'export const X = askServer<string, number>("X.Test", {timeout: 0});'],
	["timeout ceiling", "CRU017", 'export const X = askServer<string, number>("X.Test", {timeout: 121});'],
	["timeout expression", "CRU017", 'export const X = askServer<string, number>("X.Test", {timeout: 1 + 2});'],
	["extra options", "CRU017", 'export const X = askServer<string, number>("X.Test", {timeout: 2, extra: 1});'],
	["computed option", "CRU017", 'export const X = askServer<string, number>("X.Test", {["timeout"]: 2});'],
	["name variable", "CRU018", 'const n = "X.Test"; export const X = toServer<string>(n);'],
	["name template", "CRU018", "export const X = toServer<string>(`X.Test`);"],
	["name concatenation", "CRU018", 'export const X = toServer<string>("X." + "Test");'],
	...["", "1.Bad", "A..B", "A-b", "Crucible", "Crucible.X", "X." + "a".repeat(63)].map((name) => [
		`invalid name ${name}`,
		"CRU018",
		`export const X = toServer<string>(${JSON.stringify(name)});`,
	]),
	...[
		"void",
		"any",
		"never",
		"symbol",
		"bigint",
		"{}",
		"{}[]",
		"Record<never, string>",
		"() => void",
		"{ f(): void }",
		"DateTime",
		"(string | undefined)[]",
		"[string?]",
		"[string, ...number[]]",
		"[string | undefined]",
		"Map<string, number | undefined>",
		"Map<f32, string>",
		"Map<f64, string>",
		"Map<number, string>",
		"{1: string}",
		'{"-1": string}',
		"{2.5: string}",
		"{a: number} | {b: string}",
		"u8 | number",
		"string[] | {n: number}",
		"string[] | number[]",
		"string & {x: number}",
	].map((type) => [`unsupported ${type}`, "CRU019", declaration(type)]),
	["empty interface", "CRU019", "interface Empty {} " + declaration("Empty")],
	["recursive interface", "CRU019", "interface Recursive { next?: Recursive } " + declaration("Recursive")],
	["numeric enum properties", "CRU019", "const enum Slot { A, B } " + declaration("Record<Slot,string>")],
	["template type", "CRU019", "type T = `a${string}`; " + declaration("T")],
	["generic payload", "CRU019", 'function f<T>() { return toServer<T>("X.Test"); }'],
	["required client instance", "CRU020", declaration("BasePart", "toClient")],
	["client array", "CRU020", declaration("BasePart[]", "toClient")],
	["client tuple", "CRU020", declaration("[BasePart]", "toClient")],
	["client map", "CRU020", declaration("Map<string, BasePart>", "toClient")],
	["response streaming", "CRU020", 'export const X = askServer<undefined, Model>("X.Test");'],
	["macro value", "CRU023", "export const f = toServer;"],
	["macro parameter", "CRU023", "function use(f: unknown) {} use(receive);"],
	["namespace escape", "CRU023", 'import * as C from "@trioforge/crucible"; export const f = C.toClient;'],
];

test("network diagnostic catalogue rejects every unsupported declaration with real rbxtsc", () => {
	const files = {};
	for (let i = 0; i < negatives.length; i++) files[`src/shared/n${i}.ts`] = imports + negatives[i][2];
	for (const side of ["server", "client", "neutral"])
		files[`src/${side}/wrong.ts`] = imports + declaration("string", "toServer", `X.${side}`);
	const game = project(files);
	const result = inspect(game);
	for (let i = 0; i < negatives.length; i++) {
		const [name, code] = negatives[i];
		assert.ok(
			result.diagnostics.some((d) => d.file.fileName.endsWith(`/n${i}.ts`) && d.messageText.includes(code)),
			`${name} reports ${code}: ${result.diagnostics.map((d) => d.messageText).join("\n")}`,
		);
	}
	const log = compile(game, 1);
	for (const side of ["server", "client", "neutral"])
		assert.ok(result.diagnostics.some((d) => d.file.fileName.endsWith(`/${side}/wrong.ts`) && d.code === 90017));
	for (const code of new Set(negatives.map((n) => n[1]))) assert.match(log, new RegExp(code));
});

test("network receives rewrite owner field target callback and enforce placement sides and own callback returns", () => {
	const files = {
		"src/shared/messages.ts":
			imports +
			`export const Event = toServer<string>("X.Event"); export const Out = toClient<string>("X.Out"); export const Ask = askServer<string, number>("X.Ask");`,
	};
	const pre = imports + 'import {Event, Out, Ask} from "../shared/messages";\n';
	files["src/server/good.ts"] =
		pre +
		`
declare const cond: boolean;
function factory(): (p: Player, v: string) => void { return () => {}; }
export abstract class Base extends Subsystem { readonly input = receive(Event, (p, s) => { print(p.Name, s); }); }
export class Good extends Base {
 readonly ready = receive(ClientReady, p => cond ? () => print(p.Name) : undefined);
 readonly readyBlock = receive(ClientReady, p => { if(cond) return () => print(p.Name); });
 readonly request = receive(Ask, (p, s) => s.size());
 readonly made = receive(Event, factory());
}`;
	files["src/client/good.ts"] =
		pre + "export class Good extends Subsystem { readonly out = receive(Out, s => { print(s); }); }";
	let game = project(files);
	let result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(
		result.output.get("src/server/good.ts"),
		/__crucible.receive\(this, "input", Event, \(p: Player, s: string\) =>/,
	);
	compile(game);
	const bad = [
		["CRU021", "const x = receive(Event, () => {});"],
		["CRU021", 'class X extends CrucibleBehaviour { readonly tag = "x"; readonly r = receive(Event, () => {}); }'],
		["CRU021", "class X extends Subsystem { static readonly r = receive(Event, () => {}); }"],
		["CRU021", "class X extends Subsystem { r = receive(Event, () => {}); }"],
		["CRU021", "class X extends Subsystem { method() { receive(Event, () => {}); } }"],
		[
			"CRU021",
			"declare const target: ToServer<string> | ToClient<string>; class X extends Subsystem { readonly r = receive(target, () => {}); }",
		],
		[
			"CRU021",
			"declare const target: ReceiveTarget; class X extends Subsystem { readonly r = receive(target, () => {}); }",
		],
		["CRU021", "declare const target: any; class X extends Subsystem { readonly r = receive(target, () => {}); }"],
		[
			"CRU021",
			"abstract class Base extends Subsystem { readonly r = receive(Event, () => {}); } class X extends Base { readonly r = receive(Event, () => {}); }",
		],
		["CRU022", "class X extends Subsystem { readonly r = receive(Out, () => {}); }"],
		["CRU012", "class X extends Subsystem { readonly r = receive(Event, async () => {}); }"],
		[
			"CRU012",
			"const cb: (p: Player, s: string) => void = async () => {}; class X extends Subsystem { readonly r = receive(Event, cb); }",
		],
		["CRU012", "class X extends Subsystem { readonly r = receive(Event, p => new Set<Player>().add(p)); }"],
		["CRU012", "function cb() { return 5; } class X extends Subsystem { readonly r = receive(Event, cb); }"],
		[
			"CRU012",
			"declare const cond: boolean; class X extends Subsystem { readonly r = receive(Event, p => cond ? new Set<Player>().add(p) : undefined); }",
		],
		[
			"CRU012",
			"declare const cond: boolean; const cb: (p: Player) => void = p => cond ? (async () => {})() : undefined; class X extends Subsystem { readonly r = receive(ClientReady, cb); }",
		],
		[
			"CRU012",
			"declare const cond: boolean; const cb: (p: Player) => void = p => cond ? 5 : undefined; class X extends Subsystem { readonly r = receive(ClientReady, cb); }",
		],
		[
			"CRU012",
			"const helper = { cb() { return 7; } }; class X extends Subsystem { readonly r = receive(Event, helper.cb); }",
		],
		["CRU012", "class X extends Subsystem { readonly r = receive(Ask, async () => 5); }"],
	];
	const cases = Object.fromEntries(bad.map(([, code], i) => [`src/server/b${i}.ts`, pre + code]));
	for (const [side, token] of [
		["client", "ClientReady"],
		["shared", "Event"],
		["neutral", "Event"],
	])
		cases[`src/${side}/bad.ts`] = pre + `class X extends Subsystem { readonly r = receive(${token}, () => {}); }`;
	game = project({ "src/shared/messages.ts": files["src/shared/messages.ts"], ...cases });
	result = inspect(game);
	for (let i = 0; i < bad.length; i++)
		assert.ok(
			result.diagnostics.some((d) => d.file.fileName.endsWith(`/b${i}.ts`) && d.messageText.includes(bad[i][0])),
			`receiver ${i} reports ${bad[i][0]}`,
		);
	for (const side of ["client", "shared", "neutral"])
		assert.ok(
			result.diagnostics.some(
				(d) => d.file.fileName.endsWith(`/${side}/bad.ts`) && d.messageText.includes("CRU022"),
			),
		);
	const log = compile(game, 1);
	for (const code of ["CRU012", "CRU021", "CRU022"]) assert.match(log, new RegExp(code));
});

test("network duplicate names and transitive global or merged payloads fail at the transformed file", () => {
	const game = project({
		"src/shared/a.ts": imports + declaration("string"),
		"src/shared/b.ts": imports + declaration("number"),
		"src/global.d.ts": "interface ScriptPayload { value: string }\n",
		"src/alias.d.ts": "type GlobalString = string;",
		"src/shared/alias.ts": imports + declaration("{ name: GlobalString }", "toServer", "X.Alias"),
		"src/shared/global.ts": imports + declaration("{ nested: ScriptPayload }", "toServer", "X.Global"),
		"src/shared/type.ts": "export interface Payload { count: number }",
		"src/shared/merged.ts":
			imports + 'import { Payload } from "./type"; ' + declaration("Payload", "toServer", "X.Merge"),
		"src/augment.ts":
			'import "./shared/type"; declare module "./shared/type" { interface Payload { extra: string } }',
		"src/global.ts": "export {}; declare global { interface GlobalPayload { flag: boolean } }",
		"src/shared/declared-global.ts": imports + declaration("GlobalPayload", "toServer", "X.DeclaredGlobal"),
	});
	const result = inspect(game);
	for (const file of ["a.ts", "b.ts"])
		assert.ok(result.diagnostics.some((d) => d.file.fileName.endsWith(file) && d.code === 90018));
	for (const file of ["global.ts", "merged.ts", "augment.ts", "declared-global.ts", "alias.ts"])
		assert.ok(
			result.diagnostics.some((d) => d.file.fileName.endsWith(file) && d.code === 90010),
			file,
		);
	compile(game, 1);
});

test("network side entry references follow named and star barrels namespace aliases type imports and dynamic imports", () => {
	const game = project({
		"src/util/named.ts": 'export { Network } from "@trioforge/crucible/client";',
		"src/util/star.ts": 'export * from "@trioforge/crucible/client";',
		"src/server/named.ts": 'import { Network } from "../util/named"; export const N = Network;',
		"src/server/ns.ts": 'import * as util from "../util/star"; export const N = util.Network;',
		"src/shared/type.ts":
			'import type { Network } from "@trioforge/crucible/client"; export type T = typeof Network;',
		"src/server/dynamic.ts": 'export const N = import("@trioforge/crucible/client");',
		"src/client/good.ts": 'import { Network as N } from "@trioforge/crucible/client"; export const M = N;',
		"src/server/good.ts":
			'import { Network, boot } from "@trioforge/crucible/server"; export const N = Network; export const B = boot;',
	});
	const result = inspect(game);
	for (const file of [
		"util/named.ts",
		"util/star.ts",
		"server/named.ts",
		"server/ns.ts",
		"shared/type.ts",
		"server/dynamic.ts",
	])
		assert.ok(
			result.diagnostics.some((d) => d.file.fileName.endsWith(file) && d.code === 90024),
			file,
		);
	assert.ok(!result.diagnostics.some((d) => d.file.fileName.endsWith("good.ts")));
	assert.match(compile(game, 1), /CRU024/);
});

test("network compiler limits equal the approved table and runtime limits when available", () => {
	const { limits } = require("../network.cjs");
	assert.deepEqual(limits, {
		MESSAGE_NAME_MAX_BYTES: 64,
		MAX_ASK_TIMEOUT_SECONDS: 120,
		MAX_UNION_BRANCHES: 256,
		MAX_SCHEMA_NODES: 4096,
		MAX_DEPTH: 32,
	});
	const runtime = path.resolve(__dirname, "../../src/internal/network/limits.ts");
	if (fs.existsSync(runtime)) {
		const sf = ts.createSourceFile(runtime, fs.readFileSync(runtime, "utf8"), ts.ScriptTarget.Latest, true);
		const values = new Map();
		for (const statement of sf.statements)
			if (ts.isVariableStatement(statement))
				for (const decl of statement.declarationList.declarations)
					values.set(decl.name.text, Number(decl.initializer?.text));
		for (const [name, value] of Object.entries(limits)) assert.equal(values.get(name), value, name);
	}
});

test("network schema boundaries and identity checks accept limits and reject overflow", () => {
	const branches = (n) => Array.from({ length: n }, (_, i) => `"L${i.toString().padStart(3, "0")}"`).join("|");
	const nested = (n) => "{value:".repeat(n) + "string" + "}".repeat(n);
	const broad = (n) => "{" + Array.from({ length: n }, (_, i) => `f${i}:string`).join(";") + "}";
	let game = project({
		"src/shared/valid.ts":
			imports +
			`
export const Names=toServer<string>("${"A".repeat(64)}");
export const MaxTimeout=askServer<string,number>("X.Maximum",{timeout:120});
export const Union=toServer<${branches(256)}>("X.Union");
export const Depth=toServer<${nested(31)}>("X.Depth");
export const Nodes=toServer<${broad(4095)}>("X.Nodes");
export const Shapes=toServer<{a:Map<string,u8>;b:ReadonlyMap<u16,string>;c:Map<u32,number>;d:Readonly<Record<string,number>>;e:ReadonlyArray<number>;f:readonly [string,number];g:Vector3;h:Color3;i:CFrame;j:BasePart|Model;k:false|3|"z";l:{b:number};m:{b:number}}>("X.Shapes");
export const Alias=toServer<Partial<Record<"z"|"a",number>>>("X.Alias");
`,
	});
	let result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const output = descriptorExpressions(result.output.get("src/shared/valid.ts"));
	assert.match(output.get("X.Shapes")[0], /n.instance\(\["BasePart", "Model"\]\)/);
	assert.match(
		output.get("X.Shapes")[0],
		/\["g", __crucible.n.vector3\], \["h", __crucible.n.color3\], \["i", __crucible.n.cframe\]/,
	);
	assert.match(
		output.get("X.Shapes")[0],
		/n.union\(\[__crucible.n.literal\(false\), __crucible.n.literal\(3\), __crucible.n.literal\("z"\)\]\)/,
	);
	assert.equal(
		output.get("X.Alias")[0],
		'__crucible.n.object([["a", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])], ["z", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])]])',
	);
	compile(game);
	game = project({
		"src/shared/union.ts": imports + declaration(branches(257), "toServer", "X.Union"),
		"src/shared/depth.ts": imports + declaration(nested(32), "toServer", "X.Depth"),
		"src/shared/nodes.ts": imports + declaration(broad(4096), "toServer", "X.Nodes"),
		"src/shared/fake-width.ts":
			imports +
			'declare const width: unique symbol; type Fake=number & {[width]?:"u8"};' +
			declaration("Fake", "toServer", "X.Fake"),
	});
	result = inspect(game);
	for (const file of ["union.ts", "depth.ts", "nodes.ts", "fake-width.ts"])
		assert.ok(
			result.diagnostics.some((d) => d.file.fileName.endsWith(file) && d.code === 90019),
			file,
		);
	compile(game, 1);
});

test("network callback parameter types survive imported payload aliases", () => {
	const game = project({
		"src/shared/model.ts": "export interface Payload { value: string }",
		"src/shared/messages.ts": imports + 'import {Payload} from "./model";' + declaration("Payload"),
		"src/server/service.ts":
			imports +
			'import {Message} from "../shared/messages"; export class Service extends Subsystem { readonly r=receive(Message,(p,payload)=>{print(p.Name,payload.value);}); }',
	});
	assert.deepEqual(
		inspect(game).diagnostics.map((d) => d.messageText),
		[],
	);
	compile(game);
});

test("network declaration and receiver aliases and namespaces preserve macro symbol identity", () => {
	const game = project({
		"src/shared/macros.ts": 'export {toServer as message, receive as claim} from "@trioforge/crucible";',
		"src/shared/messages.ts":
			'import {message} from "./macros"; import * as C from "@trioforge/crucible"; export const Named=message<string>("X.Named"); export const Namespace=C.toServer<number>("X.Namespace");',
		"src/server/service.ts":
			'import {Subsystem as Base} from "@trioforge/crucible"; import {claim} from "../shared/macros"; import {Named} from "../shared/messages"; export class Service extends Base { readonly r=claim<typeof Named>(Named,(p,value)=>{print(p.Name,value);}); }',
		"src/client/caller.ts":
			'import {Network as N} from "@trioforge/crucible/client"; import {Named,Namespace} from "../shared/messages"; N.send(Named,"literal"); N.send(Namespace,19);',
	});
	const result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	assert.match(
		result.output.get("src/shared/messages.ts"),
		/__crucible.toServer<string>\("X.Named", __crucible.n.string\)/,
	);
	assert.match(
		result.output.get("src/shared/messages.ts"),
		/__crucible.toServer<number>\("X.Namespace", __crucible.n.f64\)/,
	);
	assert.match(result.output.get("src/server/service.ts"), /__crucible.receive\(this, "r", Named/);
	compile(game);
});
