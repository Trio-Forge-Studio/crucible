import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-network-consumer-"));
try {
	fs.mkdirSync(path.join(scratch, "src"));
	fs.mkdirSync(path.join(scratch, "node_modules/@trioforge"), { recursive: true });
	fs.symlinkSync(
		path.join(repo, "packages/crucible"),
		path.join(scratch, "node_modules/@trioforge/crucible"),
		"junction",
	);
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules/@rbxts"),
		path.join(scratch, "node_modules/@rbxts"),
		"junction",
	);
	fs.writeFileSync(path.join(scratch, "package.json"), '{"name":"network-consumer","private":true}');
	fs.writeFileSync(
		path.join(scratch, "tsconfig.json"),
		JSON.stringify({
			extends: path.join(repo, "tsconfig.rbxts.json"),
			compilerOptions: {
				rootDir: "src",
				outDir: "out",
				typeRoots: ["node_modules/@rbxts", "node_modules/@trioforge"],
				types: ["compiler-types", "types"],
			},
			include: ["src"],
		}),
	);
	fs.writeFileSync(
		path.join(scratch, "src/index.ts"),
		`import { n } from "@trioforge/crucible/emit";
export const fields = n.object([
 ["a", n.array(n.tuple([n.bool, n.f64, n.f32, n.u8, n.u16, n.u32, n.string]))],
 ["b", n.map(n.union([n.literal("a"), n.literal("b")]), n.vector3)],
 ["c", n.union([n.nil, n.instance(["BasePart", "Model"])])],
 ["d", n.union([n.variant("Kind", "Leave", n.object([]))])],
 ["e", n.color3], ["f", n.cframe], ["g", n.dynamic],
]);
`,
	);
	const result = spawnSync(
		process.execPath,
		[require.resolve("roblox-ts/out/CLI/cli.js"), "--type", "package", "-p", scratch],
		{ cwd: repo, encoding: "utf8" },
	);
	assert.equal(result.status, 0, result.stdout + result.stderr);
	const output = fs.readFileSync(path.join(scratch, "out/init.luau"), "utf8");
	for (const method of ["object", "array", "tuple", "map", "union", "literal", "instance", "variant"])
		assert.ok(output.includes(`n.${method}(`), `${method} uses dot calls`);
	console.log("all Lowering descriptor shapes compile through the physical emit entry with dot calls");
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
