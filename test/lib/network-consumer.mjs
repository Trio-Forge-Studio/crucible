import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const ansi = new RegExp(String.fromCharCode(27) + String.raw`\[[0-9;]*m`, "g");
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-network-consumer-"));
try {
	const project = (name, file) => {
		const root = path.join(scratch, name);
		fs.mkdirSync(path.join(root, "src"), { recursive: true });
		fs.mkdirSync(path.join(root, "node_modules/@trioforge"), { recursive: true });
		fs.symlinkSync(
			path.join(repo, "packages/crucible"),
			path.join(root, "node_modules/@trioforge/crucible"),
			"junction",
		);
		fs.mkdirSync(path.join(root, "node_modules/@rbxts"));
		for (const types of ["compiler-types", "types"])
			fs.symlinkSync(
				fs.realpathSync(path.join(repo, "packages/crucible/node_modules/@rbxts", types)),
				path.join(root, "node_modules/@rbxts", types),
				"junction",
			);
		fs.writeFileSync(path.join(root, "package.json"), `{"name":"network-${name}","private":true}`);
		fs.writeFileSync(
			path.join(root, "tsconfig.json"),
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
		fs.copyFileSync(path.join(repo, "test/lib", file), path.join(root, "src/index.ts"));
		const result = spawnSync(
			process.execPath,
			[require.resolve("roblox-ts/out/CLI/cli.js"), "--type", "package", "-p", root],
			{ cwd: repo, encoding: "utf8" },
		);
		return { root, status: result.status, output: (result.stdout + result.stderr).replace(ansi, "") };
	};

	const accepted = project("accepted", "network-descriptors.ts");
	assert.equal(accepted.status, 0, accepted.output);
	const luau = fs.readFileSync(path.join(accepted.root, "out/init.luau"), "utf8");
	for (const method of ["object", "array", "tuple", "map", "union", "literal", "instance", "variant"])
		assert.ok(luau.includes(`n.${method}(`), `${method} uses dot calls`);

	const rejected = project("rejected", "network-descriptors-rejected.ts");
	assert.notEqual(rejected.status, 0, "rejected shapes must not type-check");
	const errors = [...rejected.output.matchAll(/index\.ts:(\d+):\d+ - error (TS\d+)/g)].map(([, line, code]) => ({
		line: Number(line),
		code,
	}));
	assert.ok(
		errors.every(({ code }) => code === "TS2345"),
		`only argument type errors\n${rejected.output}`,
	);
	const lines = fs.readFileSync(path.join(repo, "test/lib/network-descriptors-rejected.ts"), "utf8").split("\n");
	const starts = lines.flatMap((text, i) => (text.startsWith("export const ") ? [i + 1] : []));
	starts.forEach((start, i) => {
		const end = starts[i + 1] ?? lines.length;
		assert.ok(
			errors.some(({ line }) => line >= start && line < end),
			`${lines[start - 1]} must fail to type-check\n${rejected.output}`,
		);
	});
	console.log(
		`every Lowering descriptor shape type-checks with rbxtsc 3.0.0; ${starts.length} rejected shapes fail to type-check`,
	);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
