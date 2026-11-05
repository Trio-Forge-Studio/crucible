import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-binding-"));
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
	fs.writeFileSync(path.join(scratch, "package.json"), '{"name":"binding-consumer","private":true}');
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
	const source = `import { CrucibleBehaviour, type BehaviourClass } from "@trioforge/crucible";
import { attribute } from "@trioforge/crucible";
export abstract class TriggerZone<Tag extends string> extends CrucibleBehaviour<Tag, Model> {
	readonly speed = attribute<number>(30);
	readonly seen = this.speed;
	protected override update(_dt: number): void {}
}
export class LockerZone extends TriggerZone<"Locker"> {}
export const concrete: BehaviourClass<LockerZone> = LockerZone;
export function lookup(instance: Instance): LockerZone | undefined { return LockerZone.get(instance); }
`;
	fs.writeFileSync(path.join(scratch, "src/index.ts"), source);
	fs.writeFileSync(
		path.join(scratch, "default.project.json"),
		JSON.stringify({
			name: "binding",
			tree: {
				$className: "DataModel",
				ReplicatedStorage: {
					$className: "ReplicatedStorage",
					rbxts_include: {
						$path: "include",
						node_modules: {
							$className: "Folder",
							"@trioforge": {
								$className: "Folder",
								crucible: { $path: path.join(repo, "packages/crucible/out") },
							},
						},
					},
					TS: { $path: "out" },
				},
			},
		}),
	);

	const cli = [path.join(repo, "test/lib/compile.mjs"), scratch];
	const result = spawnSync(process.execPath, cli, { cwd: repo, encoding: "utf8" });
	assert.equal(result.status, 0, result.stdout + result.stderr);
	fs.mkdirSync(path.join(repo, "build"), { recursive: true });
	fs.copyFileSync(path.join(scratch, "out/init.luau"), path.join(repo, "build/binding-usage.luau"));
	for (const name of ["get", "refresh", "accepts", "renderPriority", "tag", "instance"]) {
		fs.writeFileSync(
			path.join(scratch, "src/index.ts"),
			source + `\nexport class Bad extends CrucibleBehaviour<"Bad"> { ${name}(): void {} }\n`,
		);
		const bad = spawnSync(process.execPath, cli, { cwd: repo, encoding: "utf8" });
		assert.notEqual(bad.status, 0, `${name} must remain reserved`);
	}
	console.log("binding declarations, compiled inheritance, and all six reserved names verified");
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
