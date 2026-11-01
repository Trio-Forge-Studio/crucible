const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { setup } = require("./side-harness.cjs");

{
	const game = setup({
		files: {
			"src/shared/maps.ts":
				'import { n } from "@trioforge/crucible/emit";\nexport const a = n.map(n.string, n.string);\nexport const b = n.map(n.u8, n.string);\nexport const c = n.map(n.union([n.literal("a"), n.literal("b")]), n.string);\n',
		},
		compilerOptions: { plugins: [] },
	});
	const emit = path.join(game, "node_modules/@trioforge/crucible/out/emit");
	fs.copyFileSync(path.join(__dirname, "fixtures/network/schema.d.ts"), path.join(emit, "schema.d.ts"));
	fs.appendFileSync(path.join(emit, "index.d.ts"), '\nexport { n } from "./schema";\n');
	const cli = path.resolve(__dirname, "../../../../node_modules/roblox-ts/out/CLI/cli.js");
	const result = spawnSync(process.execPath, [cli, "-p", game], { cwd: game, encoding: "utf8" });
	const output = result.stdout + result.stderr;
	assert.equal(result.status, 0, output);
	console.log("K1 string, u8 and literal-union map descriptors recheck with rbxtsc 3.0.0.");
}
