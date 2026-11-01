const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const { project, compile, fs, path, repo } = require("./network-harness.cjs");
const game = project(
	{
		"src/client/empty.ts": "export {};",
		"src/shared/nested.ts": "export interface Nested { alpha: string }\nexport const version = 1;\n",
		"src/shared/payload.ts":
			'import { Nested, version } from "./nested"; export interface Payload { nested: Nested } export const v = version;\n',
		"src/shared/barrel.ts": 'export { Payload } from "./payload";\n',
		"src/shared/messages.ts":
			'import {toServer} from "@trioforge/crucible"; import type {Payload} from "./barrel"; export const Message=toServer<Payload>("X.Message");\n',
		"src/server/consumer.ts": 'import {Message} from "../shared/messages"; export const token = Message;\n',
		"src/util/barrel.ts": "export const value = 1;\n",
		"src/server/barrel.ts": 'import {value} from "../util/barrel"; export const v = value;\n',
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
let consumer = read("server/consumer");
probe(1, "direct field edit", () => {
	const before = read("shared/messages");
	write("shared/messages.ts", source("shared/messages.ts").replace("<Payload>", "<{ direct: string }>"));
	compile(game);
	assert.notEqual(read("shared/messages"), before);
	assert.match(read("shared/messages"), /n.string/);
	write("shared/messages.ts", source("shared/messages.ts").replace("<{ direct: string }>", "<Payload>"));
	compile(game);
});
probe(2, "nested value and type import edits", () => {
	let before = read("shared/messages");
	write("shared/nested.ts", "export interface Nested { alpha: number } export const version=2;");
	compile(game);
	assert.notEqual(read("shared/messages"), before);
	write("shared/payload.ts", 'import type {Nested} from "./nested"; export interface Payload { nested: Nested }');
	compile(game);
	before = read("shared/messages");
	write("shared/nested.ts", "export interface Nested { alpha: boolean } export const version=3;");
	compile(game);
	assert.notEqual(read("shared/messages"), before);
});
probe(3, "barrel re-export edit", () => {
	const before = read("shared/messages");
	write("shared/alternate.ts", "export interface Payload { alternate: string }");
	write("shared/barrel.ts", 'export {Payload} from "./alternate";');
	compile(game);
	assert.notEqual(read("shared/messages"), before);
});
probe(4, "add independent declaration", () => {
	const before = read("shared/messages");
	write(
		"shared/added.ts",
		'import {toServer} from "@trioforge/crucible"; export const Added=toServer<number>("X.Added");',
	);
	compile(game);
	assert.equal(read("shared/messages"), before);
	assert.match(read("shared/added"), /X.Added/);
});
probe(5, "delete declaration and callers leaves no stale output", () => {
	fs.unlinkSync(path.join(game, "src/shared/added.ts"));
	compile(game);
	assert.ok(!fs.existsSync(path.join(game, "out/shared/added.luau")), "deleted declaration has no emitted module");
});
probe(6, "move declaration preserves descriptor", () => {
	const before = read("shared/messages");
	fs.renameSync(path.join(game, "src/shared/messages.ts"), path.join(game, "src/shared/moved.ts"));
	write("server/consumer.ts", source("server/consumer.ts").replace("/messages", "/moved"));
	compile(game);
	assert.equal(read("shared/moved"), before);
});
probe(7, "canonical object and union reorder", () => {
	write(
		"shared/moved.ts",
		'import {toServer} from "@trioforge/crucible"; export const Message=toServer<{z: "b"|"a"; a: number}>("X.Message");',
	);
	compile(game);
	const before = read("shared/moved");
	write("shared/moved.ts", source("shared/moved.ts").replace('{z: "b"|"a"; a: number}', '{a: number; z: "a"|"b"}'));
	compile(game);
	assert.equal(read("shared/moved"), before);
});
probe(8, "width optional Instance and required client rejection", () => {
	write(
		"shared/moved.ts",
		'import {toClient,u8,u16} from "@trioforge/crucible"; export const Message=toClient<{width:u8; part?:BasePart}>("X.Message");',
	);
	compile(game);
	const before = read("shared/moved");
	write("shared/moved.ts", source("shared/moved.ts").replace("width:u8", "width:u16"));
	compile(game);
	assert.notEqual(read("shared/moved"), before);
	write("shared/moved.ts", source("shared/moved.ts").replace("part?:", "part:"));
	assert.match(compile(game, 1), /CRU020/);
	write("shared/moved.ts", source("shared/moved.ts").replace("part:", "part?:"));
	compile(game);
});
probe(9, "duplicate introduced in a new file", () => {
	write(
		"shared/duplicate.ts",
		'import {toClient} from "@trioforge/crucible"; export const Duplicate=toClient<string>("X.Message");',
	);
	assert.match(compile(game, 1), /CRU018/);
	fs.unlinkSync(path.join(game, "src/shared/duplicate.ts"));
	compile(game);
});
probe(10, "unimported new augmentation fails on the new file", () => {
	write(
		"shared/moved.ts",
		'import {toServer} from "@trioforge/crucible"; import {Payload} from "./payload"; export const Message=toServer<Payload>("X.Message");',
	);
	compile(game);
	const before = read("shared/moved");
	write(
		"augmentation.ts",
		'import "./shared/payload"; declare module "./shared/payload" { interface Payload { extra: string } }',
	);
	const log = compile(game, 1);
	assert.match(log, /augmentation.ts[\s\S]*CRU010/);
	assert.equal(read("shared/moved"), before);
	fs.unlinkSync(path.join(game, "src/augmentation.ts"));
	compile(game);
});
probe(11, "declare global payload", () => {
	write("global.ts", "export {}; declare global { interface GlobalPayload { x: string } }");
	write(
		"shared/global-message.ts",
		'import {toServer} from "@trioforge/crucible"; export const Global=toServer<GlobalPayload>("X.Global");',
	);
	assert.match(compile(game, 1), /CRU010/);
	fs.unlinkSync(path.join(game, "src/shared/global-message.ts"));
	fs.unlinkSync(path.join(game, "src/global.ts"));
	compile(game);
});
probe(12, "installed transformer change triggers full driver rebuild", () => {
	const binary = path.join(repo, "cli/target/debug/crucible.exe");
	assert.ok(fs.existsSync(binary), "pnpm check built the real crucible CLI");
	const driver = () => {
		const r = spawnSync(binary, ["build"], { cwd: game, encoding: "utf8", timeout: 120000 });
		assert.equal(r.status, 0, r.stdout + r.stderr);
		return r.stdout + r.stderr;
	};
	driver();
	const sentinel = path.join(game, "out/full-rebuild-sentinel");
	fs.writeFileSync(sentinel, "stale");
	const src = path.join(game, "node_modules/@trioforge/crucible/transformer-src");
	fs.mkdirSync(src, { recursive: true });
	fs.cpSync(path.join(__dirname, ".."), src, { recursive: true });
	const module = path.join(src, "network.cjs");
	fs.appendFileSync(module, "\nmodule.exports.freshnessProbe = true;\n");
	const built = spawnSync(process.execPath, [path.join(src, "build.mjs")], { encoding: "utf8" });
	assert.equal(built.status, 0, built.stderr);
	assert.match(driver(), /transformer changed, compiling from clean/);
	assert.ok(!fs.existsSync(sentinel));
});
probe(13, "unchanged diagnostic survives unrelated edit", () => {
	const file = "shared/bad.ts";
	write(file, 'import {toServer} from "@trioforge/crucible"; const Bad=toServer<string>("X.Bad");');
	assert.match(compile(game, 1), /CRU017/);
	write("server/unrelated.ts", "export const x = 2;");
	assert.match(compile(game, 1), /CRU017/);
	fs.unlinkSync(path.join(game, "src", file));
	compile(game);
});
probe(14, "consumer text stays identical after payload edit", () => {
	consumer = read("server/consumer");
	write("shared/nested.ts", "export interface Nested { alpha: string; added: number }");
	compile(game);
	assert.equal(read("server/consumer"), consumer);
});
probe(15, "barrel side change then wrong-side reference", () => {
	write("util/barrel.ts", 'export const value=1; export {Network} from "@trioforge/crucible/client";');
	assert.match(compile(game, 1), /CRU024/);
	write(
		"server/barrel.ts",
		'import {value,Network} from "../util/barrel"; export const v=value;export const N=Network;',
	);
	assert.match(compile(game, 1), /src\/server\/barrel.ts[\s\S]*CRU024/);
});
console.log(JSON.stringify(results, null, 2));
if (results.some((r) => !r.pass)) process.exitCode = 1;
