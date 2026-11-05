import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const repo = process.cwd();
const ansi = new RegExp(String.fromCharCode(27) + String.raw`\[[0-9;]*m`, "g");

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-profile-"));

const accepted = {
	"src/shared/data.ts": `import { profileStore, serverOnly, type Profile } from "@trioforge/crucible";
export const PlayerData = profileStore("FD.Players", {
	template: {
		currency: 0,
		upgrades: {} as Record<string, number>,
		settings: { vibration: true },
		notes: serverOnly(""),
	},
});
export type PlayerData = Profile<typeof PlayerData>;
`,
	"src/server/wallet.ts": `import { Subsystem } from "@trioforge/crucible";
import { PlayerData } from "../shared/data";
export class WalletService extends Subsystem {
	private readonly stamp = PlayerData.onLoad((_player, data) => ({ ...data, currency: data.currency + 1 }));
	pay(player: Player, amount: number) {
		PlayerData(player).currency.update((c) => c + amount);
		PlayerData(player).upgrades.speed.set(1);
		return PlayerData(player).notes();
	}
}
`,
	"src/client/hud.ts": `import { PlayerData } from "../shared/data";
export const label = () => \`\${PlayerData().currency()} \${PlayerData.loaded()}\`;
`,
};
// K9: each line's TypeScript error from the declared types fails the real transformed build.
const rejected = {
	"src/shared/explicit.ts": [
		`import { profileStore } from "@trioforge/crucible";
export const Coins = profileStore<{ readonly coins: number }>("FD.Coins", { template: { coins: "0" } });
`,
		2322,
		"Type 'string' is not assignable to type 'number'.",
	],
	"src/server/write.ts": [
		`import { PlayerData } from "../shared/data";
export const write = (player: Player) => PlayerData(player).currency.set("lots");
`,
		2345,
		"Argument of type 'string' is not assignable to parameter of type 'number'.",
	],
	"src/client/set.ts": [
		`import { PlayerData } from "../shared/data";
export const write = () => PlayerData().currency.set(1);
`,
		2339,
		"Property 'set' does not exist on type",
	],
};

function build(name, files) {
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
	fs.symlinkSync(
		path.join(repo, "packages/compiler-types"),
		path.join(game, "node_modules/@trioforge/compiler-types"),
		"junction",
	);
	for (const types of ["types"])
		fs.symlinkSync(
			fs.realpathSync(path.join(repo, "packages/crucible/node_modules/@rbxts", types)),
			path.join(game, "node_modules/@rbxts", types),
			"junction",
		);
	write("package.json", { name: `profile-${name}`, private: true });
	write("default.project.json", {
		name: "profile",
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
		extends: path.join(repo, "tsconfig.json"),
		compilerOptions: {
			rootDir: "src",
			outDir: "out",
			declaration: false,

			types: ["@trioforge/compiler-types", "@rbxts/types"],
		},
		include: ["src"],
	});
	for (const [file, text] of Object.entries(files)) write(file, text);
	const result = spawnSync(process.execPath, [path.join(repo, "test/lib/compile.mjs"), game], {
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
	const emitted = fs.readFileSync(path.join(good.game, "out/shared/data.luau"), "utf8");
	assert.match(emitted, /\.profileStore\("FD\.Players", /, `the transformer rewrote profileStore\n${emitted}`);
	assert.match(emitted, /\{ \{ "notes" \} \}/, `the emit form lists the serverOnly path\n${emitted}`);
	const files = { ...accepted };
	for (const [file, [text]] of Object.entries(rejected)) files[file] = text;
	const bad = build("rejected", files);
	assert.equal(bad.status, 1, `the Crucible compiler build rejects every wrong line\n${bad.output}`);
	assert.equal(bad.output.match(/ error TS/g)?.length, Object.keys(rejected).length, bad.output);
	for (const [file, [, code, message]] of Object.entries(rejected)) {
		const at = bad.output.indexOf(`${file}(`);
		assert.ok(at >= 0, `${file} fails the build\n${bad.output}`);
		const reported = bad.output.slice(at).replace(/^.*?\): /, "");
		assert.ok(reported.startsWith(`error TS${code}: ${message}`), `${file}\n${bad.output}`);
	}
	console.log(
		`the transformed build accepts a profile store and rejects ${Object.keys(rejected).length} wrong lines`,
	);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
