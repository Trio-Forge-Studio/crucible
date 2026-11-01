const assert = require("node:assert/strict");
const { test } = require("node:test");
const h = require("./state-data-harness.cjs");
const { project, inspect, compile, calls, evaluate, describe, countNodes, verdicts, normalPath, reported } = h;
const { fs, path, showPaths } = h;
const fd = fs.readFileSync(path.join(h.fixture, "fd.ts"), "utf8");
const cases = (name) => fs.readFileSync(path.join(h.fixture, "cases", name), "utf8");
const probe = verdicts("data-lower.out");
const probeText = fs.readFileSync(path.join(h.fixture, "data-lower.out"), "utf8").replaceAll("\r", "");
const macros = [
	"replicated",
	"replicatedPerPlayer",
	"profileStore",
	"dataStore",
	"profileMessage",
	"config",
	"playerConfig",
];
const local = (name) =>
	name
		.replace(/^N\.(\d+)/, "N.N$1")
		.replace("N.N26.ViewNameLong.Abcdefghijklmnopqrstuv", "N.N26.ViewNameLong.Abcdefghijklmnopqrstu");
const section = (title) => {
	const lines = probeText.split("\n");
	const start = lines.findIndex((l) => l.startsWith(title));
	const out = [];
	for (let i = start + 1; i < lines.length && lines[i].startsWith("  "); i++) out.push(lines[i].trim());
	return out;
};
// rule 131's encoding, from the probe: "[]" for an element, "*" for a map value, a marker-like key escaped.
const segment = (key) => (["[]", "*"].includes(key) || key.startsWith("\\") ? `\\${key}` : key);
function fieldPaths(node, prefix = [], out = new Map()) {
	const add = (p) => out.set(JSON.stringify(p), p);
	if (node.kind === "object")
		for (const [k, v] of node.fields) {
			add([...prefix, segment(k)]);
			fieldPaths(v, [...prefix, segment(k)], out);
		}
	if (node.kind === "array") fieldPaths(node.element, [...prefix, "[]"], out);
	if (node.kind === "map") fieldPaths(node.value, [...prefix, "*"], out);
	if (node.kind === "union") for (const b of node.branches) fieldPaths(b, prefix, out);
	if (node.kind === "variant") {
		add([...prefix, segment(node.key)]);
		fieldPaths(node.body, prefix, out);
	}
	return out;
}
const sortPaths = (paths) =>
	paths.sort((a, b) => {
		for (let i = 0; i < Math.min(a.length, b.length); i++) {
			const order = Buffer.compare(Buffer.from(a[i]), Buffer.from(b[i]));
			if (order) return order;
		}
		return a.length - b.length;
	});
const pathsOf = (node) => sortPaths([...fieldPaths(node).values()]);

test("P1 data and handle cases lower or reject as the data probe's lower.out records", () => {
	let game = project({
		"src/shared/fd.ts": fd,
		"src/shared/data.ts": cases("data-positive-shared.ts"),
		"src/server/data.ts": cases("data-positive-server.ts"),
		"src/shared/handles.ts": cases("handles-positive.ts"),
	});
	let result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const found = new Map();
	for (const file of ["src/shared/data.ts", "src/server/data.ts", "src/shared/handles.ts"])
		for (const [name, call] of calls(result.output.get(file), macros)) found.set(name, call);
	const lowers = [...probe.values()].filter((e) => e.verdict === "lowers");
	assert.equal(lowers.length, 25);
	for (const entry of lowers) {
		const call = found.get(entry.name);
		assert.ok(call, `${entry.name} is rewritten`);
		const descriptors = call.args.filter((a) => a.startsWith("__crucible.n.")).map(evaluate);
		if (entry.shape)
			assert.equal(
				`${countNodes(descriptors[0])} nodes: ${describe(descriptors[0])}`,
				`${entry.shape.nodes} nodes: ${entry.shape.text}`,
				entry.name,
			);
		if (entry.handles) {
			const [persist, replica] = descriptors;
			const marks = evaluate(call.args[4]);
			assert.deepEqual(
				{
					persist: countNodes(persist),
					serverOnly: showPaths(marks).slice(1, -1),
					replica: countNodes(replica),
					views: 1 + replica.fields.length,
				},
				entry.handles,
				entry.name,
			);
			if (entry.inline)
				assert.equal(describe(persist, new Set(marks.map((m) => JSON.stringify(m)))), entry.inline, entry.name);
		}
	}
	const view = (name) => evaluate(found.get(name).args[3]);
	assert.equal(found.get("Probe.FdClientType").args[2], found.get("FD.Players").args[3]);
	assert.equal(found.get("Probe.HiddenClientType").args[2], found.get("Game.Hidden").args[3]);
	assert.match(probeText, /^ok {2}FD\.Players replica .* equals ClientProfile<typeof PlayerData>/m);
	const emitted = result.output
		.get("src/shared/handles.ts")
		.split("\n")
		.find((l) => l.startsWith("export const PlayerData = "));
	const expected = probeText.split("\n").find((l) => l.startsWith("export const PlayerData = "));
	assert.equal(emitted.replaceAll(/\s+/g, ""), expected.replaceAll(/\s+/g, ""));
	const names = [
		"Crucible.Live.FD.Players",
		...view("FD.Players").fields.map(([k]) => `Crucible.Data.FD.Players.${k}`),
	];
	assert.deepEqual(
		names.map((n) => `${n} (${Buffer.byteLength(n)})`),
		section("FD.Players view declarations (18):"),
	);
	assert.deepEqual(
		pathsOf(view("FD.Players")).map((p) => `{ ${p.map((s) => JSON.stringify(s)).join(", ")} }`),
		section("FD.Players replicated paths"),
	);
	const added = (name) =>
		pathsOf(view(name))
			.map((p) => JSON.stringify(p))
			.filter((p) => !pathsOf(view("FD.Players")).some((q) => JSON.stringify(q) === p));
	assert.deepEqual(added("FD.Players.Plus"), ['["mood"]']);
	assert.deepEqual(added("FD.Players.Secret"), []);
	assert.equal(view("Game.AllHidden").fields.length, 0);
	assert.deepEqual(pathsOf(view("Game.Hidden")), [["coins"], ["mixed"], ["mixed", "shown"]]);
	assert.ok(pathsOf(view("Game.Collide.Plus")).some((p) => JSON.stringify(p) === '["n","a.b"]'));
	assert.ok(!pathsOf(view("Game.Collide")).some((p) => JSON.stringify(p) === '["n","a.b"]'));
	assert.equal(
		`markers: a map value is "*", a literal "*" key is "\\*": ${pathsOf(view("Game.Markers"))
			.map((p) => `{ ${p.map((s) => JSON.stringify(s)).join(", ")} }`)
			.join(" ")}`,
		probeText
			.split("\n")
			.find((l) => l.startsWith("ok  markers:"))
			.slice(4),
	);
	compile(game);

	game = project({
		"src/shared/fd.ts": fd,
		"src/shared/data.ts": cases("data-negative-shared.ts"),
		"src/server/data.ts": cases("data-negative-server.ts"),
		"src/shared/handles.ts": cases("handles-negative.ts"),
	});
	result = inspect(game);
	const rejected = [...probe.values()].filter((e) => e.verdict === "rejected");
	assert.equal(rejected.length, 26);
	for (const entry of rejected) {
		const mine = reported(result, local(entry.name));
		assert.ok(mine.length, `${entry.name} is rejected`);
		for (const { code, at } of mine)
			assert.ok(
				entry.diagnostics.some((e) => e.code === code && normalPath(at).startsWith(normalPath(e.at))),
				`${entry.name} ${code} at ${at} matches the probe's ${JSON.stringify(entry.diagnostics)}`,
			);
		for (const e of entry.diagnostics.filter((d) => d.code !== "CRU019" || entry.diagnostics.length === 1))
			assert.ok(
				mine.some((m) => m.code === e.code && normalPath(m.at) === normalPath(e.at)),
				`${entry.name} reports ${e.code} at ${e.at}: ${JSON.stringify(mine)}`,
			);
	}
	const log = compile(game, 1);
	for (const code of ["CRU018", "CRU019", "CRU026"]) assert.match(log, new RegExp(code));
});

const shared =
	'import { profileStore, serverOnly, replicatedPaths, toServer, replicated } from "@trioforge/crucible";\n';
const server = 'import { dataStore, profileMessage, config, playerConfig } from "@trioforge/crucible/server";\n';
const kindNegatives = [
	["profileStore local const", "src/shared", "CRU017", 'const S = profileStore("X.Store", { template: { a: 0 } });'],
	["profileStore let", "src/shared", "CRU017", 'export let S = profileStore("X.Store", { template: { a: 0 } });'],
	[
		"profileStore two types",
		"src/shared",
		"CRU017",
		'export const S = profileStore<{ a: number }, 1>("X.Store", { template: { a: 0 } });',
	],
	[
		"profileStore extra argument",
		"src/shared",
		"CRU017",
		'export const S = profileStore("X.Store", { template: { a: 0 } }, 1 as never);',
	],
	[
		"profileStore name too long",
		"src/shared",
		"CRU018: Store names are 1 to 50 bytes",
		`export const S = profileStore("X.${"a".repeat(49)}", { template: { a: serverOnly(0) } });`,
	],
	[
		"profileStore reserved name",
		"src/shared",
		"CRU018",
		'export const S = profileStore("Crucible.Store", { template: { a: 0 } });',
	],
	[
		"profileStore scalar root",
		"src/shared",
		"CRU019: X.Store value: a store's value is a record",
		'export const S = profileStore<{ a: number } | number>("X.Store", { template: 1 as never });',
	],
	["profileStore macro value", "src/shared", "CRU023", "export const f = profileStore;"],
	[
		"profileStore namespace value",
		"src/shared",
		"CRU023",
		'import * as C from "@trioforge/crucible"; export const f = C["profileStore"];',
	],
	[
		"profileStore variant reserved",
		"src/shared",
		"CRU026: X.Store value.phase.update",
		'export const S = profileStore<{ phase: { kind: "a"; update: number } | { kind: "b" } }>("X.Store", { template: { phase: { kind: "b" } } });',
	],
	[
		"profileStore map value reserved",
		"src/shared",
		"CRU026: X.Store value.m.value.insert",
		'export const S = profileStore("X.Store", { template: { m: {} as Record<string, { insert: number }> } });',
	],
	[
		"profileStore serverOnly in a union",
		"src/shared",
		"CRU019: X.Store value.u",
		'import type { ServerOnly } from "@trioforge/crucible"; export const S = profileStore<{ u: ServerOnly<number> | string }>("X.Store", { template: { u: "" } });',
	],
	["dataStore missing type", "src/server", "CRU017", 'export const D = dataStore("X.Data", { defaults: { a: 0 } });'],
	[
		"dataStore extra argument",
		"src/server",
		"CRU017",
		'export const D = dataStore<{ a: number }>("X.Data", { defaults: { a: 0 } }, 1 as never);',
	],
	[
		"dataStore name too long",
		"src/server",
		"CRU018: Store names are 1 to 50 bytes",
		`export const D = dataStore<{ a: number }>("X.${"a".repeat(49)}", { defaults: { a: 0 } });`,
	],
	[
		"dataStore nested",
		"src/server",
		"CRU017",
		'export function make() { return dataStore<{ a: number }>("X.Data", { defaults: { a: 0 } }); }',
	],
	["dataStore macro value", "src/server", "CRU023", "export const f = dataStore;"],
	["profileMessage missing type", "src/server", "CRU017", 'export const P = profileMessage("X.Message");'],
	[
		"profileMessage extra argument",
		"src/server",
		"CRU017",
		'export const P = profileMessage<string>("X.Message", 1 as never);',
	],
	[
		"profileMessage name too long",
		"src/server",
		"CRU018: Profile message names are 1 to 64 bytes",
		`export const P = profileMessage<string>("X.${"a".repeat(63)}");`,
	],
	[
		"profileMessage Vector3",
		"src/server",
		"CRU019: X.Message value.at: Vector3 cannot be saved",
		'export const P = profileMessage<{ at: Vector3 }>("X.Message");',
	],
	["profileMessage macro value", "src/server", "CRU023", "export const f = profileMessage;"],
	["config missing type", "src/server", "CRU017", 'export const C = config("X.Config", 1 as never);'],
	[
		"config name too long",
		"src/server",
		"CRU018: Config names are 1 to 64 bytes",
		`export const C = config<number>("X.${"a".repeat(63)}", 0);`,
	],
	[
		"config serverOnly",
		"src/server",
		"CRU019: X.Config value.a: serverOnly marks a profileStore template field",
		'import type { ServerOnly } from "@trioforge/crucible"; export const C = config<{ a: ServerOnly<number> }>("X.Config", undefined as never);',
	],
	[
		"config Color3",
		"src/server",
		"CRU019: X.Config value: Color3 cannot be saved",
		'export const C = config<Color3>("X.Config", undefined as never);',
	],
	["playerConfig let", "src/server", "CRU017", 'export let C = playerConfig<number>("X.Config", 0);'],
	[
		"playerConfig Instance",
		"src/server",
		"CRU019: X.Config value: Instances cannot be saved",
		'export const C = playerConfig<Part>("X.Config", undefined as never);',
	],
	["playerConfig macro value", "src/server", "CRU023", "export const f = playerConfig;"],
	["config macro value", "src/server", "CRU023", "export const f = config;"],
];

test("data declarations take the declaration, three name indexes, escape and lowering checks per macro", () => {
	const files = {};
	for (let i = 0; i < kindNegatives.length; i++) {
		const [, dir, , text] = kindNegatives[i];
		files[`${dir}/n${i}.ts`] = (dir === "src/shared" ? shared : server) + text;
	}
	files["src/server/profile.ts"] = shared + 'export const S = profileStore("X.Side", { template: { a: 0 } });';
	files["src/client/profile.ts"] = shared + 'export const S = profileStore("X.ClientSide", { template: { a: 0 } });';
	files["src/shared/store.ts"] = shared + 'export const S = profileStore("X.Twice", { template: { a: 0 } });';
	files["src/server/store.ts"] =
		server + 'export const D = dataStore<{ a: number }>("X.Twice", { defaults: { a: 0 } });';
	files["src/server/messages.ts"] =
		server + 'export const A = profileMessage<string>("X.Msg"); export const B = profileMessage<number>("X.Msg");';
	files["src/server/configs.ts"] =
		server + 'export const A = config<number>("X.Cfg", 0); export const B = playerConfig<number>("X.Cfg", 0);';
	files["src/shared/namespaces.ts"] =
		shared +
		'export const S = profileStore("Game.Same", { template: { a: 0 } }); export const R = replicated<number>("Game.Same", 0); export const T = toServer<number>("Game.Same");';
	files["src/server/namespaces.ts"] =
		server +
		'export const P = profileMessage<number>("Game.Same"); export const C = config<number>("Game.Same", 0);';
	files["src/shared/limits.ts"] =
		shared +
		`export const S = profileStore("X.${"a".repeat(48)}", { template: { a: serverOnly(0) } }); export const V = profileStore("Game.Views", { template: { ${"f".repeat(64 - "Crucible.Data.Game.Views.".length)}: 0 } });`;
	files["src/server/limits.ts"] =
		server +
		`export const P = profileMessage<string>("X.${"a".repeat(62)}"); export const C = config<number>("Y.${"a".repeat(62)}", 0); export const D = dataStore<{ a: number }>("Z.${"a".repeat(48)}", { defaults: { a: 0 } });`;
	files["src/shared/view.ts"] =
		shared +
		`export const V = profileStore("Game.Viewz", { template: { ${"f".repeat(65 - "Crucible.Data.Game.Viewz.".length)}: 0 } });`;
	files["src/shared/wrong.ts"] =
		server + 'export const D = dataStore<{ a: number }>("X.SharedData", { defaults: { a: 0 } });';
	const game = project(files);
	const result = inspect(game);
	const at = (file) => h.diagnosticsIn(result, game, file);
	for (let i = 0; i < kindNegatives.length; i++) {
		const [name, dir, code] = kindNegatives[i];
		assert.ok(
			at(`${dir}/n${i}.ts`).some((m) => m.startsWith(code)),
			`${name} reports ${code}: ${at(`${dir}/n${i}.ts`).join("\n")}`,
		);
	}
	for (const side of ["server", "client"])
		assert.deepEqual(at(`src/${side}/profile.ts`), [
			"CRU017: Declare profile stores as an exported top-level const in a shared module.",
		]);
	assert.deepEqual(at("src/shared/store.ts"), ["CRU018: X.Twice is already declared at src/server/store.ts:2."]);
	assert.deepEqual(at("src/server/store.ts"), ["CRU018: X.Twice is already declared at src/shared/store.ts:2."]);
	assert.deepEqual(at("src/server/messages.ts"), [
		"CRU018: X.Msg is already declared at src/server/messages.ts:2.",
		"CRU018: X.Msg is already declared at src/server/messages.ts:2.",
	]);
	assert.deepEqual(at("src/server/configs.ts"), [
		"CRU018: X.Cfg is already declared at src/server/configs.ts:2.",
		"CRU018: X.Cfg is already declared at src/server/configs.ts:2.",
	]);
	assert.deepEqual(at("src/shared/namespaces.ts"), []);
	assert.deepEqual(at("src/server/namespaces.ts"), []);
	assert.deepEqual(at("src/shared/limits.ts"), []);
	assert.deepEqual(at("src/server/limits.ts"), []);
	assert.deepEqual(at("src/shared/view.ts"), [
		`CRU018: Game.Viewz value.${"f".repeat(40)}: its view name Crucible.Data.Game.Viewz.${"f".repeat(40)} is 65 bytes, over 64; this store allows field names of at most 39 bytes; rename the field, shorten the store name, or mark it serverOnly`,
	]);
	assert.ok(
		at("src/shared/wrong.ts").includes(
			"CRU017: Declare data stores as an exported top-level const in a server module.",
		),
	);
	assert.ok(at("src/shared/wrong.ts").some((m) => m.startsWith("CRU024")));
	const log = compile(game, 1);
	for (const code of ["CRU017", "CRU018", "CRU019", "CRU023", "CRU024", "CRU026"])
		assert.match(log, new RegExp(code));
});

test("CRU027 checks profile store calls by the callee's type in every file side, barrels included", () => {
	const store = shared + 'export const PlayerData = profileStore("FD.Players", { template: { coins: 0 } });\n';
	const use = (from) => `import { PlayerData } from "${from}";\ndeclare const player: Player;\n`;
	const server = "is server-only; the client reads PlayerData().";
	const client = "reads the local player's profile; on the server call";
	const files = {
		"src/shared/template.ts": store,
		"src/shared/barrel.ts": 'export { PlayerData } from "./template";',
		"src/server/good.ts":
			use("../shared/template") +
			"PlayerData(player).coins.set(1); PlayerData.loaded(player); PlayerData.handoff(player); PlayerData.players();",
		"src/client/good.ts": use("../shared/template") + "PlayerData().coins(); PlayerData.loaded();",
		"src/shared/good.ts":
			use("./template") +
			'import { replicatedPaths } from "@trioforge/crucible"; export const paths = replicatedPaths(PlayerData); export const store = PlayerData;',
	};
	const triggers = [
		["client", "../shared/template", "PlayerData(player);", `CRU027: PlayerData(player) ${server}`],
		["client", "../shared/template", "PlayerData.handoff(player);", `CRU027: PlayerData.handoff ${server}`],
		["client", "../shared/template", "PlayerData.loaded(player);", `CRU027: PlayerData.loaded ${server}`],
		["client", "../shared/template", 'PlayerData["save"](player);', `CRU027: PlayerData.save ${server}`],
		["server", "../shared/template", "PlayerData();", `CRU027: PlayerData() ${client} PlayerData(player).`],
		[
			"server",
			"../shared/template",
			"PlayerData.loaded();",
			`CRU027: PlayerData.loaded() ${client} PlayerData.loaded(player).`,
		],
		["shared", "./template", "PlayerData(player);", `CRU027: PlayerData(player) ${server}`],
		["shared", "./template", "PlayerData.players();", `CRU027: PlayerData.players ${server}`],
		["shared", "./template", "PlayerData();", `CRU027: PlayerData() ${client} PlayerData(player).`],
		["client", "../shared/barrel", "PlayerData(player);", `CRU027: PlayerData(player) ${server}`],
		["client", "../shared/barrel", "PlayerData.inspect(1);", `CRU027: PlayerData.inspect ${server}`],
		["server", "../shared/barrel", "PlayerData();", `CRU027: PlayerData() ${client} PlayerData(player).`],
		["shared", "./barrel", "PlayerData(player);", `CRU027: PlayerData(player) ${server}`],
		["shared", "./barrel", "PlayerData();", `CRU027: PlayerData() ${client} PlayerData(player).`],
		["neutral", "../shared/template", "PlayerData();", `CRU027: PlayerData() ${client} PlayerData(player).`],
		[
			"client",
			"../shared/template",
			"const alias = PlayerData; (alias)(player);",
			"CRU027: alias(player) is server-only; the client reads alias().",
		],
	];
	triggers.forEach(([side, from, code], i) => (files[`src/${side}/t${i}.ts`] = use(from) + code));
	const game = project(files);
	const result = inspect(game);
	for (const file of ["src/server/good.ts", "src/client/good.ts", "src/shared/good.ts", "src/shared/barrel.ts"])
		assert.deepEqual(h.diagnosticsIn(result, game, file), [], file);
	triggers.forEach(([side, , code, message], i) =>
		assert.deepEqual(h.diagnosticsIn(result, game, `src/${side}/t${i}.ts`), [message], code),
	);
	assert.match(compile(game, 1), /CRU027/);
});

test("CRU025 rejects a ProcessReceipt assignment in a program that holds a grant claim", () => {
	const store = shared + 'export const PlayerData = profileStore("FD.Players", { template: { coins: 0 } });\n';
	const claim =
		'import { Subsystem } from "@trioforge/crucible"; import { PlayerData } from "../shared/template";\nexport class Shop extends Subsystem { readonly skins = PlayerData.grant(1, (player, data) => data); }';
	const receipt = (code) => `declare const handler: (receipt: ReceiptInfo) => Enum.ProductPurchaseDecision;\n${code}`;
	const triggers = [
		'game.GetService("MarketplaceService").ProcessReceipt = handler;',
		'const market = game.GetService("MarketplaceService"); market.ProcessReceipt = handler;',
		'const market = game.GetService("MarketplaceService"); market["ProcessReceipt"] = handler;',
		'const market = game.GetService("MarketplaceService"); (market).ProcessReceipt = handler;',
		'const market = game.GetService("MarketplaceService"); (market.ProcessReceipt) = handler;',
		"export function install(market: MarketplaceService) { market.ProcessReceipt = handler; }",
	];
	const files = { "src/shared/template.ts": store, "src/server/shop.ts": claim };
	triggers.forEach((code, i) => (files[`src/server/r${i}.ts`] = receipt(code)));
	files["src/server/other.ts"] = receipt(
		"const other = { ProcessReceipt: handler }; other.ProcessReceipt = handler;",
	);
	let game = project(files);
	const result = inspect(game);
	triggers.forEach((code, i) =>
		assert.deepEqual(
			h.diagnosticsIn(result, game, `src/server/r${i}.ts`),
			[
				"CRU025: Crucible's data module owns ProcessReceipt; register grants with a profile store's grant(productId, ...).",
			],
			code,
		),
	);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/server/other.ts"), []);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/server/shop.ts"), []);
	assert.match(compile(game, 1), /CRU025/);
	game = project({
		"src/shared/template.ts": store,
		"src/server/shop.ts":
			'import { PlayerData } from "../shared/template"; declare const player: Player; PlayerData(player).coins.set(1);',
		"src/server/r.ts": receipt(triggers[0]),
	});
	assert.deepEqual(
		inspect(game).diagnostics.map((d) => d.messageText),
		[],
	);
	compile(game);
});

test("configs rewrite to their emit forms and configs.md case 12 rejects a client or shared read with CRU024", () => {
	const declarations =
		server +
		'export interface GlobalEvent { readonly name: string; readonly multiplier: number }\nexport const NO_EVENT: GlobalEvent = { name: "", multiplier: 1 };\nexport const GlobalEventConfig = config<GlobalEvent>("GlobalEvent", NO_EVENT);\nexport const Discount = playerConfig<number>("Shop.WelcomeDiscount", 0);\n';
	const read = (from) => `import { GlobalEventConfig, Discount } from "${from}";\ndeclare const player: Player;\n`;
	const game = project({
		"src/server/configs.ts": declarations,
		"src/server/use.ts":
			read("./configs") + "export const event = GlobalEventConfig(); export const d = Discount(player);",
		"src/util/barrel.ts": 'export { GlobalEventConfig, Discount } from "../server/configs";',
		"src/client/global.ts": read("../server/configs") + "export const event = GlobalEventConfig().name;",
		"src/client/player.ts": read("../server/configs") + "export const d = Discount(player);",
		"src/shared/global.ts": read("../server/configs") + "export const event = GlobalEventConfig();",
		"src/shared/player.ts": read("../server/configs") + "export const d = Discount(player);",
		"src/client/barrel.ts": read("../util/barrel") + "export const event = GlobalEventConfig();",
	});
	const result = inspect(game);
	const found = calls(result.output.get("src/server/configs.ts"), macros);
	assert.deepEqual(found.get("GlobalEvent"), {
		types: ["GlobalEvent"],
		args: [
			'"GlobalEvent"',
			"NO_EVENT",
			h.O([
				["multiplier", h.F],
				["name", h.S],
			]),
		],
	});
	assert.deepEqual(found.get("Shop.WelcomeDiscount"), {
		types: ["number"],
		args: ['"Shop.WelcomeDiscount"', "0", h.F],
	});
	assert.deepEqual(h.diagnosticsIn(result, game, "src/server/configs.ts"), []);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/server/use.ts"), []);
	for (const file of [
		"client/global.ts",
		"client/player.ts",
		"shared/global.ts",
		"shared/player.ts",
		"client/barrel.ts",
	])
		assert.ok(
			h.diagnosticsIn(result, game, `src/${file}`).some((m) => m.startsWith("CRU024")),
			`${file}: ${h.diagnosticsIn(result, game, `src/${file}`)}`,
		);
	assert.match(compile(game, 1), /CRU024/);
});

test("K9 the build rejects a profileStore template, dataStore defaults or config fallback that breaks its type", () => {
	for (const [file, text] of [
		[
			"src/shared/store.ts",
			shared + 'export const S = profileStore<{ a: number }>("X.Store", { template: { a: "s" } });',
		],
		[
			"src/server/store.ts",
			server + 'export const D = dataStore<{ a: number }>("X.Data", { defaults: { a: "s" } });',
		],
		["src/server/config.ts", server + 'export const C = config<number>("X.Config", "s");'],
		["src/server/player.ts", server + 'export const C = playerConfig<{ a: number }>("X.Config", { a: true });'],
	]) {
		const game = project({ [file]: text });
		assert.deepEqual(
			inspect(game).diagnostics.map((d) => d.messageText),
			[],
			file,
		);
		assert.match(compile(game, 1), /TS2322|TS2345/, file);
	}
	const game = project({
		"src/shared/store.ts":
			shared +
			'export const S = profileStore<{ a: number }>("X.Store", { template: { a: 1 } });\nexport const Inferred = profileStore("X.Inferred", { template: { a: 1, secret: serverOnly("") } });',
		"src/server/use.ts":
			'import { S, Inferred } from "../shared/store"; declare const player: Player; export const a: number = S(player).a(); export const secret: string = Inferred(player).secret();',
	});
	compile(game);
});

test("transformer limits equal the reactive and data contract tables and the runtime limits when available", () => {
	const { limits, reactiveLimits, dataLimits } = require("../declarations.cjs");
	assert.deepEqual(limits, {
		MESSAGE_NAME_MAX_BYTES: 64,
		MAX_ASK_TIMEOUT_SECONDS: 120,
		MAX_UNION_BRANCHES: 256,
		MAX_SCHEMA_NODES: 4096,
		MAX_DEPTH: 32,
	});
	assert.deepEqual(reactiveLimits, { MESSAGE_NAME_MAX_BYTES: 64, SYNC_VALUE_DEPTH: 28 });
	assert.deepEqual(dataLimits, {
		STORE_NAME_MAX_BYTES: 50,
		MESSAGE_NAME_MAX_BYTES: 64,
		CONFIG_NAME_MAX_BYTES: 64,
		PERSIST_DEPTH: 28,
		VIEW_NAME_PREFIX: "Crucible.Data.",
		LIVE_NAME_PREFIX: "Crucible.Live.",
		VIEW_NAME_MAX_BYTES: 64,
		RESERVED_HANDLE_NAMES: ["set", "update", "insert", "remove"],
	});
	assert.equal(Buffer.byteLength(dataLimits.LIVE_NAME_PREFIX) + dataLimits.STORE_NAME_MAX_BYTES, 64);
	const ts = h.ts;
	const read = (e) =>
		ts.isNumericLiteral(e)
			? Number(e.text)
			: ts.isStringLiteralLike(e)
				? e.text
				: ts.isArrayLiteralExpression(e)
					? e.elements.map(read)
					: ts.isAsExpression(e) || ts.isSatisfiesExpression(e) || ts.isParenthesizedExpression(e)
						? read(e.expression)
						: undefined;
	for (const [rel, table] of [
		["reactive/limits.ts", reactiveLimits],
		["data/limits.ts", dataLimits],
		["configs/limits.ts", { CONFIG_NAME_MAX_BYTES: dataLimits.CONFIG_NAME_MAX_BYTES }],
	]) {
		const file = path.resolve(__dirname, "../../src/internal", rel);
		if (!fs.existsSync(file)) continue;
		const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true);
		const values = new Map();
		for (const statement of sf.statements)
			if (ts.isVariableStatement(statement))
				for (const decl of statement.declarationList.declarations)
					if (decl.initializer) values.set(decl.name.text, read(decl.initializer));
		for (const [name, value] of Object.entries(table))
			if (values.has(name)) assert.deepEqual(values.get(name), value, `${rel} ${name}`);
	}
});

test("persist rejections name the annotation to write and serverOnly marks follow optional fields", () => {
	let game = project({
		"src/shared/good.ts":
			shared +
			'import type { ServerOnly } from "@trioforge/crucible";\nexport const Opt = profileStore<{ a: number; b?: ServerOnly<number> }>("X.Opt", { template: { a: 0 } });\nexport const Reserved = replicated<{ set: number; update: string }>("X.Reserved", { set: 1, update: "" });\nexport const Tagged = replicated<{ by: string } & ({ kind: "a"; x: number } | { kind: "b" })>("X.Tagged", { by: "", kind: "b" });',
		"src/server/good.ts":
			server +
			'export const Records = dataStore<{ insert: number; remove: string }>("X.Records", { defaults: { insert: 0, remove: "" } });',
	});
	let result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const found = calls(result.output.get("src/shared/good.ts"), macros);
	assert.deepEqual(found.get("X.Opt").args.slice(2), [
		h.O([
			["a", h.F],
			["b", h.U(h.Nil, h.F)],
		]),
		h.O([["a", h.F]]),
		'[["b"]]',
	]);
	assert.equal(
		found.get("X.Reserved").args[2],
		h.O([
			["set", h.F],
			["update", h.S],
		]),
	);
	assert.equal(
		found.get("X.Tagged").args[2],
		h.U(
			h.V(
				"kind",
				"a",
				h.O([
					["by", h.S],
					["x", h.F],
				]),
			),
			h.V("kind", "b", h.O([["by", h.S]])),
		),
	);
	assert.equal(
		calls(result.output.get("src/server/good.ts"), macros).get("X.Records").args[2],
		h.O([
			["insert", h.F],
			["remove", h.S],
		]),
	);
	compile(game);
	const store = (name, template) => `export const S${name} = profileStore("X.${name}", { template: ${template} });\n`;
	game = project({
		"src/shared/bad.ts":
			shared +
			'import type { ServerOnly } from "@trioforge/crucible";\n' +
			store("Empty", "{ quests: [] }") +
			store("Record", "{ upgrades: {} }") +
			store("Undefined", "{ note: undefined }") +
			store("Reserved", "{ coins: 0, update: 0 } as { coins: number; update: number }") +
			store("Element", "{ jobs: [] as ServerOnly<number>[] }") +
			store("Twice", "{ a: serverOnly({ b: serverOnly(0) }) }") +
			'export const SState = replicated<{ x: ServerOnly<number> }>("X.State", undefined as never);',
		"src/server/bad.ts":
			'import type { ServerOnly } from "@trioforge/crucible";\n' +
			server +
			'export const D = dataStore<{ key: ServerOnly<string> }>("X.Data", { defaults: undefined as never });\n' +
			'export const U = dataStore<unknown>("X.Unknown", { defaults: undefined as never });',
	});
	result = inspect(game);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/bad.ts"), [
		"CRU019: X.Empty value.quests[]: an empty array has no element type; annotate it: [] as string[]",
		"CRU019: X.Record value.upgrades: an empty object type accepts anything; annotate an empty template table: {} as Record<string, number>",
		"CRU019: X.Undefined value.note: a position that is always undefined; annotate it: undefined as string | undefined",
		"CRU026: X.Reserved value.update: `update` is a handle method name (set, update, insert, remove); rename the field",
		"CRU019: X.Element value.jobs[]: serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member",
		"CRU019: X.Twice value.a.b: this field is already inside a serverOnly field",
		"CRU019: X.State value.x: serverOnly marks a profileStore template field; it means nothing here",
	]);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/server/bad.ts"), [
		"CRU019: X.Data value.key: serverOnly marks a profileStore template field; it means nothing here",
		"CRU019: X.Unknown value: unknown cannot be stored or replicated state; give the value a declared type",
	]);
	compile(game, 1);
});

const rootMark =
	"serverOnly marks a template field, never the store's whole value; wrap the value in an object and mark the field";
const fieldMark =
	"serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member";
const symbolKey = "symbol property names cannot cross the network";
const hideImports = 'import type { ServerOnly } from "@trioforge/crucible";\n';
const declareStore = (name, type) =>
	`export const S${name} = profileStore<${type}>("X.${name}", { template: undefined as never });\n`;

test("rule 129 rejects a serverOnly mark on the store's whole value in every spelling", () => {
	const roots = {
		Type: declareStore("Type", "ServerOnly<{ secret: number }>"),
		Call: 'export const SCall = profileStore("X.Call", { template: serverOnly({ secret: 1 }) });\n',
		CallTyped:
			'export const SCallTyped = profileStore<ServerOnly<{ secret: number }>>("X.CallTyped", { template: serverOnly({ secret: 1 }) });\n',
		Const: 'const tmpl = serverOnly({ secret: 1 });\nexport const SConst = profileStore("X.Const", { template: tmpl });\n',
		Alias: "type Hidden = ServerOnly<{ secret: number }>;\ntype Root = Hidden;\n" + declareStore("Alias", "Root"),
		Generic: "type Mark<T> = ServerOnly<T>;\n" + declareStore("Generic", "Mark<{ secret: number }>"),
		EmptyIntersection: declareStore("EmptyIntersection", "ServerOnly<{ secret: number }> & {}"),
		Scalar: declareStore("Scalar", "ServerOnly<number>"),
		RecordInside: declareStore("RecordInside", "ServerOnly<Record<string, number>>"),
		MapInside: declareStore("MapInside", "ServerOnly<Map<string, number>>"),
		Optional: declareStore("Optional", "ServerOnly<{ secret: number }> | undefined"),
	};
	const game = project({ "src/shared/root.ts": shared + hideImports + Object.values(roots).join("") });
	const result = inspect(game);
	const diagnostics = h.diagnosticsIn(result, game, "src/shared/root.ts");
	for (const name of Object.keys(roots))
		assert.ok(
			diagnostics.includes(`CRU019: X.${name} value: ${rootMark}`),
			`${name} reports the root mark: ${diagnostics.join("\n")}`,
		);
	assert.equal(
		diagnostics.filter((d) => d.includes(rootMark)).length,
		Object.keys(roots).length,
		"one root report per declaration",
	);
	compile(game, 1);
});

test("rule 129 keeps every serverOnly value that survives a type spelling out of the replica", () => {
	const rejected = {
		RootIntersection: [declareStore("RootIntersection", "ServerOnly<{ secret: number }> & { a: number }"), "value"],
		RootIntersections: [
			declareStore("RootIntersections", "ServerOnly<{ s: number }> & ServerOnly<{ t: number }>"),
			"value",
		],
		RootExtends: [
			"interface Mine extends ServerOnly<{ s: number }> { b: number }\n" + declareStore("RootExtends", "Mine"),
			"value",
		],
		RootPartial: [declareStore("RootPartial", "Partial<ServerOnly<{ s: number }>>"), "value"],
		RootSpread: [
			'export const SRootSpread = profileStore("X.RootSpread", { template: { ...serverOnly({ b: 1 }) } });\n',
			"value",
		],
		FieldIntersection: [
			declareStore("FieldIntersection", "{ a: number; b: ServerOnly<{ x: number }> & { y: number } }"),
			"value.b",
		],
		FieldIntersections: [
			declareStore(
				"FieldIntersections",
				"{ a: number; b: ServerOnly<{ x: number }> & ServerOnly<{ y: number }> }",
			),
			"value.b",
		],
		FieldExtends: [
			"interface Hides extends ServerOnly<{ s: number }> { b: number }\n" +
				declareStore("FieldExtends", "{ a: number; m: Hides }"),
			"value.m",
		],
		FieldOmit: [declareStore("FieldOmit", "{ a: number; o: Omit<ServerOnly<{ s: number }>, never> }"), "value.o"],
		FieldSpread: [
			'export const SFieldSpread = profileStore("X.FieldSpread", { template: { a: 1, ...serverOnly({ b: 1 }) } });\n',
			"value",
		],
	};
	const game = project({
		"src/shared/survive.ts":
			shared +
			hideImports +
			Object.values(rejected)
				.map(([source]) => source)
				.join(""),
	});
	const diagnostics = h.diagnosticsIn(inspect(game), game, "src/shared/survive.ts");
	for (const [name, [, at]] of Object.entries(rejected))
		assert.ok(
			diagnostics.includes(`CRU019: X.${name} ${at}: ${symbolKey}`),
			`${name} rejects the brand's symbol key at ${at}: ${diagnostics.join("\n")}`,
		);
	compile(game, 1);
});

test("rule 129 rejects a serverOnly mark inside an array, map, tuple, union or optional object", () => {
	const positions = {
		Element: ["{ a: number; b: ServerOnly<number>[] }", "value.b[]", fieldMark],
		ElementObject: ["{ a: number; b: { s: ServerOnly<number> }[] }", "value.b[].s", fieldMark],
		MapValue: ["{ a: number; b: Map<string, ServerOnly<number>> }", "value.b.value", fieldMark],
		MapObjectValue: ["{ a: number; b: Map<string, { s: ServerOnly<number> }> }", "value.b.value.s", fieldMark],
		RecordValue: ["{ a: number; b: Record<string, ServerOnly<number>> }", "value.b.value", fieldMark],
		Tuple: ["{ a: number; b: [ServerOnly<number>, string] }", "value.b[0]", fieldMark],
		UnionLiteral: ['{ a: number; u: ServerOnly<number> | "z" }', "value.u", fieldMark],
		UnionVariant: ['{ a: number; u: { k: "p"; s: ServerOnly<number> } | { k: "q" } }', "value.u.s", fieldMark],
		OptionalParent: ["{ a: number; p?: { s: ServerOnly<number> } }", "value.p.s", fieldMark],
		UnionObjects: [
			"{ a: number; u: ServerOnly<{ x: number }> | { y: number } }",
			"value.u",
			"union members ServerOnly<{ x: number; }>, { y: number; } have no discriminant; give each a distinct literal property",
		],
		UnionMarks: [
			"{ a: number; u: ServerOnly<number> | ServerOnly<string> | undefined }",
			"value.u",
			"union members ServerOnly<number>, ServerOnly<string> have no discriminant; give each a distinct literal property",
		],
		NestedAliasElement: [
			"{ a: number; l: Hidden[] }",
			"value.l[]",
			fieldMark,
			"type Hidden = ServerOnly<{ v: number }>;\n",
		],
	};
	const game = project({
		"src/shared/positions.ts":
			shared +
			hideImports +
			Object.values(positions)
				.map((position) => position[3] ?? "")
				.join("") +
			Object.entries(positions)
				.map(([name, [type]]) => declareStore(name, type))
				.join(""),
	});
	const diagnostics = h.diagnosticsIn(inspect(game), game, "src/shared/positions.ts");
	assert.deepEqual(
		diagnostics,
		Object.entries(positions).map(([name, [, at, message]]) => `CRU019: X.${name} ${at}: ${message}`),
	);
	compile(game, 1);
});

test("rule 129 marks a field at any object depth through aliases, optionals and partials and prunes what empties", () => {
	const game = project({
		"src/shared/marks.ts":
			shared +
			hideImports +
			"type Hidden = ServerOnly<{ v: number }>;\n" +
			declareStore(
				"Marks",
				"{ a: number; c: { w: Hidden; z: string }; h: Hidden; n: Partial<{ b: ServerOnly<number> }>; o?: ServerOnly<number> }",
			) +
			declareStore("Only", "{ only: ServerOnly<number> }") +
			declareStore("Deep", "{ w: { only: ServerOnly<number> } }"),
	});
	const result = inspect(game);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/marks.ts"), []);
	const found = calls(result.output.get("src/shared/marks.ts"), macros);
	assert.deepEqual(found.get("X.Marks").args.slice(2), [
		h.O([
			["a", h.F],
			[
				"c",
				h.O([
					["w", h.O([["v", h.F]])],
					["z", h.S],
				]),
			],
			["h", h.O([["v", h.F]])],
			["n", h.O([["b", h.U(h.Nil, h.F)]])],
			["o", h.U(h.Nil, h.F)],
		]),
		h.O([
			["a", h.F],
			["c", h.O([["z", h.S]])],
		]),
		'[["c", "w"], ["h"], ["n", "b"], ["o"]]',
	]);
	assert.deepEqual(found.get("X.Only").args.slice(2), [h.O([["only", h.F]]), h.O([]), '[["only"]]']);
	assert.deepEqual(found.get("X.Deep").args.slice(2), [
		h.O([["w", h.O([["only", h.F]])]]),
		h.O([]),
		'[["w", "only"]]',
	]);
	compile(game);
});

test("K12 a profileStore root is an object with declared fields and a dataStore keeps its record root", () => {
	const message = "a profile store's value is an object with declared fields; wrap the record in a field";
	const game = project({
		"src/shared/records.ts":
			shared +
			declareStore("Record", "Record<string, number>") +
			declareStore("Map", "Map<string, number>") +
			declareStore("Index", "{ [key: string]: number }") +
			'export const SInferred = profileStore("X.Inferred", { template: {} as Record<string, number> });\n' +
			declareStore("Wrapped", "{ upgrades: Record<string, number> }"),
		"src/server/records.ts":
			server +
			'export const DRecord = dataStore<Record<string, number>>("X.DataRecord", { defaults: {} });\n' +
			'export const DMap = dataStore<Map<string, number>>("X.DataMap", { defaults: new Map() });\n',
	});
	const result = inspect(game);
	assert.deepEqual(
		h.diagnosticsIn(result, game, "src/shared/records.ts"),
		["Record", "Map", "Index", "Inferred"].map((name) => `CRU019: X.${name} value: ${message}`),
	);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/server/records.ts"), []);
	const found = calls(result.output.get("src/server/records.ts"), macros);
	assert.deepEqual(found.get("X.DataRecord").args.slice(2), [h.M(h.S, h.F)]);
	assert.deepEqual(found.get("X.DataMap").args.slice(2), [h.M(h.S, h.F)]);
	const wrapped = calls(result.output.get("src/shared/records.ts"), macros).get("X.Wrapped");
	assert.deepEqual(wrapped.args.slice(2), [
		h.O([["upgrades", h.M(h.S, h.F)]]),
		h.O([["upgrades", h.M(h.S, h.F)]]),
		"[]",
	]);
	compile(game, 1);
});

test("a profileStore field name may hold digits and underscores", () => {
	const game = project({
		"src/shared/digits.ts": shared + declareStore("Digits", "{ slot2: number; item_3b: string; Tier1: boolean }"),
	});
	const result = inspect(game);
	assert.deepEqual(h.diagnosticsIn(result, game, "src/shared/digits.ts"), []);
	const view = h.O([
		["Tier1", h.B],
		["item_3b", h.S],
		["slot2", h.F],
	]);
	assert.deepEqual(calls(result.output.get("src/shared/digits.ts"), macros).get("X.Digits").args.slice(2), [
		view,
		view,
		"[]",
	]);
});

test("persist depth stops at PERSIST_DEPTH", () => {
	const nested = (n) => "{v:".repeat(n) + "string" + "}".repeat(n);
	let game = project({
		"src/shared/deep.ts":
			shared + `export const Deep = profileStore<${nested(27)}>("X.Deep", { template: undefined as never });`,
	});
	assert.deepEqual(
		inspect(game).diagnostics.map((d) => d.messageText),
		[],
	);
	game = project({
		"src/server/deep.ts": server + `export const Deep = config<${nested(28)}>("X.Deep", undefined as never);`,
	});
	assert.match(
		inspect(game)
			.diagnostics.map((d) => d.messageText)
			.join("\n"),
		/CRU019: X\.Deep value(\.v)+: schema exceeds the depth limit of 28/,
	);
	compile(game, 1);
});
