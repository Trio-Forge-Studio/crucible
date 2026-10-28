import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";

const sample = import.meta.dirname;
const transformer = path.resolve(sample, "../../packages/crucible/transformer-src");
const hash = createHash("sha256");
for (const name of fs
	.readdirSync(transformer)
	.filter((name) => /\.(cjs|mjs)$/.test(name))
	.sort()) {
	hash.update(name);
	hash.update(fs.readFileSync(path.join(transformer, name)));
}
const fingerprint = hash.digest("hex");
const receipt = path.join(sample, "out/transformer.sha256");
if (!fs.existsSync(receipt) || fs.readFileSync(receipt, "utf8") !== fingerprint)
	fs.rmSync(path.join(sample, "out"), { recursive: true, force: true });
const result = spawnSync(
	process.execPath,
	[path.resolve(sample, "../../node_modules/roblox-ts/out/CLI/cli.js"), "--type", "game"],
	{
		cwd: sample,
		stdio: "inherit",
	},
);
if (result.status !== 0) process.exit(result.status ?? 1);
fs.writeFileSync(receipt, fingerprint);
