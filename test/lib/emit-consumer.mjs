import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-emit-"));
try {
	fs.mkdirSync(path.join(scratch, "src"));
	fs.mkdirSync(path.join(scratch, "node_modules/@trioforge"), { recursive: true });
	fs.symlinkSync(
		path.join(repo, "packages/crucible"),
		path.join(scratch, "node_modules/@trioforge/crucible"),
		"junction",
	);
	fs.mkdirSync(path.join(scratch, "node_modules/@rbxts"));
	fs.symlinkSync(
		path.join(repo, "packages/compiler-types"),
		path.join(scratch, "node_modules/@trioforge/compiler-types"),
		"junction",
	);
	for (const name of ["types"]) {
		fs.symlinkSync(
			fs.realpathSync(path.join(repo, "packages/crucible/node_modules/@rbxts", name)),
			path.join(scratch, "node_modules/@rbxts", name),
			"junction",
		);
	}
	fs.writeFileSync(path.join(scratch, "package.json"), '{"name":"emit-consumer","private":true}');
	fs.writeFileSync(
		path.join(scratch, "tsconfig.json"),
		JSON.stringify({
			extends: path.join(repo, "tsconfig.json"),
			compilerOptions: {
				rootDir: "src",
				outDir: "out",

				types: ["@trioforge/compiler-types", "@rbxts/types"],
			},
			include: ["src"],
		}),
	);
	fs.writeFileSync(
		path.join(scratch, "src/index.ts"),
		`import { Subsystem, type All } from "@trioforge/crucible";
import * as emit from "@trioforge/crucible/emit";
export class Provider extends Subsystem {}
emit.defineClass(Provider, { kind: "subsystem", id: "consumer:Provider", deps: [], provides: [] });
export const provider = emit.resolve<Provider>("consumer:Provider");
export const providers: All<Provider> = emit.resolveAll<Provider>("consumer:Provider");
export const value = emit.attribute<number>({}, "value", 0);
export const check = emit.t.union(emit.t.literal(3), emit.t.optional(emit.t.number));
`,
	);
	const result = spawnSync(process.execPath, [path.join(repo, "test/lib/compile.mjs"), scratch, "--check"], {
		cwd: repo,
		encoding: "utf8",
	});
	assert.equal(result.status, 0, result.stdout + result.stderr);
	console.log("the emit export declarations type-check under TypeScript 7");
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
