import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const ansi = new RegExp(String.fromCharCode(27) + String.raw`\[[0-9;]*m`, "g");
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-network-api-"));

function compile(name, file) {
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
	fs.writeFileSync(path.join(root, "package.json"), `{"name":"network-api-${name}","private":true}`);
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
	return { status: result.status, output: (result.stdout + result.stderr).replace(ansi, "") };
}

try {
	const accepted = compile("accepted", "network-api.ts");
	assert.equal(accepted.status, 0, accepted.output);
	const rejected = compile("rejected", "network-api-rejected.ts");
	assert.notEqual(rejected.status, 0, "rejected forms must not type-check");
	const lines = [...rejected.output.matchAll(/index\.ts:(\d+):\d+ - error TS\d+/g)].map(([, line]) => Number(line));
	const source = fs.readFileSync(path.join(repo, "test/lib/network-api-rejected.ts"), "utf8").split("\n");
	const from = source.indexOf("// REJECTED") + 1;
	const starts = source.flatMap((text, i) => (i >= from && text.startsWith("export ") ? [i + 1] : []));
	starts.forEach((start, i) => {
		const end = starts[i + 1] ?? source.length + 1;
		assert.ok(
			lines.some((line) => line >= start && line < end),
			`${source[start - 1]} must fail to type-check\n${rejected.output}`,
		);
	});
	assert.ok(
		lines.every((line) => line > from),
		`only the rejected statements fail\n${rejected.output}`,
	);
	console.log(`the accepted API forms type-check; ${starts.length} rejected forms each fail`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
