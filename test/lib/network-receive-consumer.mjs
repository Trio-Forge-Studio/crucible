import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const ansi = new RegExp(String.fromCharCode(27) + String.raw`\[[0-9;]*m`, "g");
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-network-receive-"));

const messages = `import { askServer, toClient, toServer } from "@trioforge/crucible";
export const Query = askServer<undefined, unknown>("X.Query");
export const Typed = askServer<undefined, string | undefined>("X.Typed");
export const Event = toServer<string>("X.Event");
export const Ask = askServer<string, number>("X.Ask");
export const Codes = toClient<"Ok" | "Invalid">("X.Codes");
`;
const receiver = (field) => `import { receive, Subsystem, ClientReady, type ToServer } from "@trioforge/crucible";
import { Query, Typed, Event, Ask, Codes } from "../shared/messages";
export class Q extends Subsystem {
	readonly r = ${field};
}
`;
const accepted = {
	"src/server/query.ts": "receive(Query, () => {})",
	"src/server/typed.ts": 'receive(Typed, (_p) => "x")',
	"src/server/event.ts": "receive(Event, (_p: Player, s: string) => print(s))",
	"src/server/ready.ts": "receive(ClientReady, (p) => () => print(p.Name))",
	"src/server/ask.ts": "receive(Ask, (_p, s) => s.size())",
	"src/client/codes.ts": "receive(Codes, (result) => print(result))",
};
// Expected diagnostics are the TypeScript errors the root receive's declared types give each callback (K2, K9).
const rejected = {
	"src/server/typed.ts": [
		"receive(Typed, () => {})",
		2345,
		"Argument of type '() => void' is not assignable to parameter of type '(player: Player) => string | undefined'.",
	],
	"src/server/event.ts": [
		"receive(Event, (p: Player, s: number) => {})",
		2345,
		"Argument of type '(p: Player, s: number) => void' is not assignable to parameter of type '(player: Player, payload: string) => void'.",
	],
	"src/server/ready.ts": [
		"receive(ClientReady, (p: string) => {})",
		2345,
		"Argument of type '(p: string) => void' is not assignable to parameter of type '(player: Player) => void | (() => void)'.",
	],
	"src/server/ask.ts": ["receive(Ask, (p, s) => s)", 2322, "Type 'string' is not assignable to type 'number'."],
	"src/server/explicit.ts": [
		"receive<ToServer<number>>(Event, (p, n) => {})",
		2345,
		"Argument of type 'ToServer<string>' is not assignable to parameter of type 'ToServer<number>'.",
	],
	"src/client/codes.ts": [
		"receive(Codes, (result: number) => {})",
		2345,
		"Argument of type '(result: number) => void' is not assignable to parameter of type '(payload: \"Ok\" | \"Invalid\") => void'.",
	],
};

function build(name, fields) {
	const game = path.join(scratch, name);
	const write = (file, text) => {
		fs.mkdirSync(path.dirname(path.join(game, file)), { recursive: true });
		fs.writeFileSync(path.join(game, file), typeof text === "string" ? text : JSON.stringify(text));
	};
	fs.mkdirSync(path.join(game, "node_modules/@trioforge"), { recursive: true });
	fs.symlinkSync(
		path.join(repo, "packages/crucible"),
		path.join(game, "node_modules/@trioforge/crucible"),
		"junction",
	);
	fs.mkdirSync(path.join(game, "node_modules/@rbxts"));
	for (const types of ["compiler-types", "types"])
		fs.symlinkSync(
			fs.realpathSync(path.join(repo, "packages/crucible/node_modules/@rbxts", types)),
			path.join(game, "node_modules/@rbxts", types),
			"junction",
		);
	write("package.json", { name: `network-receive-${name}`, private: true });
	write("default.project.json", {
		name: "receive",
		tree: {
			$className: "DataModel",
			ReplicatedStorage: {
				$className: "ReplicatedStorage",
				rbxts_include: {
					$path: "include",
					node_modules: {
						$className: "Folder",
						"@rbxts": { $path: "node_modules/@rbxts" },
						"@trioforge": { $path: "node_modules/@trioforge" },
					},
				},
				TS: { $path: "out/shared" },
			},
			ServerScriptService: { $className: "ServerScriptService", TS: { $path: "out/server" } },
			StarterPlayer: {
				$className: "StarterPlayer",
				StarterPlayerScripts: { $className: "StarterPlayerScripts", TS: { $path: "out/client" } },
			},
		},
	});
	write("tsconfig.json", {
		extends: path.join(repo, "tsconfig.rbxts.json"),
		compilerOptions: {
			rootDir: "src",
			outDir: "out",
			declaration: false,
			typeRoots: ["node_modules/@rbxts", "node_modules/@trioforge"],
			types: ["compiler-types", "types"],
			plugins: [{ transform: "@trioforge/crucible/transformer" }],
		},
		include: ["src"],
	});
	write("src/shared/messages.ts", messages);
	for (const [file, field] of Object.entries(fields)) write(file, receiver(field));
	const result = spawnSync(process.execPath, [require.resolve("roblox-ts/out/CLI/cli.js"), "-p", game], {
		cwd: game,
		encoding: "utf8",
	});
	return {
		game,
		status: result.status,
		output: (result.stdout + result.stderr).replace(ansi, "").replaceAll("\r\n", "\n"),
	};
}

try {
	const good = build("accepted", accepted);
	assert.equal(good.status, 0, good.output);
	const emitted = fs.readFileSync(path.join(good.game, "out/server/event.luau"), "utf8");
	assert.match(emitted, /\.receive\(self, "r", /, `the transformer rewrote receive to the emit form\n${emitted}`);
	const bad = build("rejected", Object.fromEntries(Object.entries(rejected).map(([file, [field]]) => [file, field])));
	assert.equal(bad.status, 1, `the rbxtsc build rejects every wrong callback\n${bad.output}`);
	assert.equal(bad.output.match(/ error TS/g)?.length, Object.keys(rejected).length, bad.output);
	for (const [file, [, code, message]] of Object.entries(rejected)) {
		const at = bad.output.indexOf(`${file}:`);
		assert.ok(at >= 0, `${file} fails the build\n${bad.output}`);
		const reported = bad.output.slice(at).replace(/^\S+ - /, "");
		assert.ok(reported.startsWith(`error TS${code}: ${message}`), `${file}\n${bad.output}`);
	}
	console.log(`the transformed build accepts typed callbacks and rejects ${Object.keys(rejected).length} wrong ones`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
