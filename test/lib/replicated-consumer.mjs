import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const repo = process.cwd();
const ansi = new RegExp(String.fromCharCode(27) + String.raw`\[[0-9;]*m`, "g");

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-replicated-"));

const accepted = {
	"src/shared/state.ts": `import { replicated, replicatedPerPlayer } from "@trioforge/crucible";
export const Score = replicated<number>("Job.Score", 0);
export const Owner = replicatedPerPlayer<{ coins: number } | undefined>("Data.Owner", undefined);
export const Round = replicated<string | undefined>("Job.Round", undefined);
`,
	"src/server/job.ts": `import { Subsystem } from "@trioforge/crucible";
import { publish } from "@trioforge/crucible/server";
import { Owner, Round, Score } from "../shared/state";
export class JobService extends Subsystem {
	private readonly score = publish(Score);
	private readonly owner = publish(Owner, () => ({ coins: 1 }));
	private readonly round = publish(Round);
	bump() {
		this.score((value) => value + 1);
		this.round("first");
		this.round(() => undefined);
	}
}
`,
	"src/client/hud.ts": `import { syncStatus } from "@trioforge/crucible";
import { Score } from "../shared/state";
export const label = () => \`\${Score()} \${syncStatus(Score)}\`;
`,
};
// Expected diagnostics are the TypeScript errors the declared types give each line (K9, rule 44).
const rejected = {
	"src/shared/bad.ts": [
		`import { replicated } from "@trioforge/crucible";
export const Bad = replicated<number>("Job.Bad", "zero");
`,
		2345,
		"Argument of type 'string' is not assignable to parameter of type 'number'.",
	],
	"src/client/write.ts": [
		`import { Score } from "../shared/state";
Score.set(1);
`,
		2339,
		"Property 'set' does not exist on type 'Replicated<number>'.",
	],
	"src/server/clear.ts": [
		`import { Subsystem } from "@trioforge/crucible";
import { publish } from "@trioforge/crucible/server";
import { Round } from "../shared/state";
export class RoundService extends Subsystem {
	private readonly round = publish(Round);
	clear() {
		this.round(undefined);
	}
}
`,
		2345,
		"Argument of type 'undefined' is not assignable to parameter of type 'string | ((current: string | undefined) => string | undefined)'.",
	],
	"src/client/publish.ts": [
		`import { publish } from "@trioforge/crucible/client";
publish;
`,
		2305,
		`Module '"@trioforge/crucible/client"' has no exported member 'publish'.`,
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
	write("package.json", { name: `replicated-${name}`, private: true });
	write("default.project.json", {
		name: "replicated",
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
	const emitted = fs.readFileSync(path.join(good.game, "out/shared/state.luau"), "utf8");
	assert.match(
		emitted,
		/\.replicated\("Job\.Score", 0, /,
		`the compiler rewrote replicated to the emit form\n${emitted}`,
	);
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
		`the transformed build accepts replicated declarations and rejects ${Object.keys(rejected).length} wrong lines`,
	);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
