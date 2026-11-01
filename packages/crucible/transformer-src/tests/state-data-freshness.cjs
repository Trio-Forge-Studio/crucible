const assert = require("node:assert/strict");
const { project, compile, fs, path } = require("./state-data-harness.cjs");
const game = project(
	{
		"src/shared/nested.ts": "export interface Wallet { coins: number }\nexport const version = 1;\n",
		"src/shared/profile.ts":
			'import type { Wallet } from "./nested"; export interface Profile { wallet: Wallet; xp: number }\n',
		"src/shared/barrel.ts": 'export type { Profile } from "./profile";\n',
		"src/shared/store.ts":
			'import { profileStore, serverOnly } from "@trioforge/crucible"; import type { Profile } from "./barrel";\ndeclare const template: Profile;\nexport const PlayerData = profileStore("FD.Players", { template: { ...template, secret: serverOnly(0) } });\n',
		"src/shared/state.ts":
			'import { replicated } from "@trioforge/crucible"; import type { Wallet } from "./nested";\nexport const Shown = replicated<Wallet | undefined>("Game.Shown", undefined);\n',
		"src/server/use.ts":
			'import { PlayerData } from "../shared/store"; declare const player: Player; export const coins = PlayerData(player).wallet.coins();\n',
		"src/client/use.ts": 'import { PlayerData } from "../shared/store"; export const xp = PlayerData().xp();\n',
	},
	{ incremental: true },
);
const write = (rel, text) => {
	const file = path.join(game, "src", rel);
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, text);
};
const read = (rel) => fs.readFileSync(path.join(game, "out", rel + ".luau"), "utf8");
const source = (rel) => fs.readFileSync(path.join(game, "src", rel), "utf8");
const results = [];
function probe(number, name, run) {
	try {
		run();
		results.push({ number, name, pass: true });
		console.log(`PASS ${number} ${name}`);
	} catch (error) {
		results.push({ number, name, pass: false, error: String(error) });
		console.error(`FAIL ${number} ${name}\n${error}`);
	}
}
compile(game);
assert.ok(fs.existsSync(path.join(game, "out/build.tsbuildinfo")));
probe(1, "profile store: editing a type the template reaches recompiles the declaring module", () => {
	const store = read("shared/store");
	const state = read("shared/state");
	write("shared/nested.ts", "export interface Wallet { coins: number; gems: number }\nexport const version = 2;\n");
	compile(game);
	assert.notEqual(read("shared/store"), store);
	assert.match(read("shared/store"), /"gems"/);
	assert.notEqual(read("shared/state"), state);
	assert.match(read("shared/state"), /"gems"/);
});
probe(2, "profile store: editing the template type through a barrel recompiles the declaring module", () => {
	const before = read("shared/store");
	write(
		"shared/profile.ts",
		'import type { Wallet } from "./nested"; export interface Profile { wallet: Wallet; xp: number; level: number }\n',
	);
	compile(game);
	assert.notEqual(read("shared/store"), before);
	assert.match(read("shared/store"), /"level"/);
});
probe(3, "profile store: marking a field serverOnly in the declaring module changes the replica and paths", () => {
	write("shared/store.ts", source("shared/store.ts").replace("secret: serverOnly(0)", "secret: 0"));
	compile(game);
	const before = read("shared/store");
	write("shared/store.ts", source("shared/store.ts").replace("secret: 0", "secret: serverOnly(0)"));
	compile(game);
	assert.notEqual(read("shared/store"), before);
	assert.match(read("shared/store"), /\{\s*"secret"\s*\}/);
});
probe(4, "a reserved name introduced in a reached type fails the next build", () => {
	write("shared/nested.ts", "export interface Wallet { coins: number; update: number }\nexport const version = 3;\n");
	assert.match(compile(game, 1), /CRU026/);
	write("shared/nested.ts", "export interface Wallet { coins: number; gems: number }\nexport const version = 2;\n");
	compile(game);
});
probe(5, "a client file that starts calling a server member fails the next build", () => {
	const client = source("client/use.ts");
	write("client/use.ts", client + "declare const player: Player; PlayerData.handoff(player);\n");
	assert.match(compile(game, 1), /CRU027/);
	write("client/use.ts", client);
	compile(game);
});
probe(6, "a duplicate store name introduced in a new file fails the next build", () => {
	write(
		"server/records.ts",
		'import { dataStore } from "@trioforge/crucible/server"; export const Records = dataStore<{ a: number }>("FD.Players", { defaults: { a: 0 } });\n',
	);
	assert.match(compile(game, 1), /CRU018/);
	fs.unlinkSync(path.join(game, "src/server/records.ts"));
	compile(game);
});
probe(7, "an augmentation of a reached template type fails on the new file", () => {
	write(
		"augmentation.ts",
		'import "./shared/nested"; declare module "./shared/nested" { interface Wallet { extra: string } }\n',
	);
	assert.match(compile(game, 1), /augmentation.ts[\s\S]*CRU010/);
	fs.unlinkSync(path.join(game, "src/augmentation.ts"));
	compile(game);
});
probe(8, "consumers keep identical output after a template type edit", () => {
	const server = read("server/use");
	const client = read("client/use");
	write("shared/nested.ts", "export interface Wallet { coins: number; gems: number; tokens: number }\n");
	compile(game);
	assert.equal(read("server/use"), server);
	assert.equal(read("client/use"), client);
});
console.log(JSON.stringify(results, null, 2));
if (results.some((r) => !r.pass)) process.exitCode = 1;
