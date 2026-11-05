import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { appendFileSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const directory = mkdtempSync(join(tmpdir(), "crucible-licenses-"));
const manifest = ["--manifest-path", "crates/crucible/Cargo.toml"];
const args = ["about", "generate", "--locked", "--fail", ...manifest, "--config"];
try {
	const rejected = join(directory, "about.toml");
	writeFileSync(rejected, "accepted = []\nignore-dev-dependencies = true\n");
	const negative = spawnSync("cargo", [...args, rejected, "notices.hbs", "-o", join(directory, "rejected.txt")], {
		encoding: "utf8",
	});
	assert.notEqual(negative.status, 0, "cargo-about must reject unaccepted licenses");
	assert.match(negative.stderr, /failed to satisfy license requirements/i);
	console.log(`cargo-about negative control exited ${negative.status}: failed to satisfy license requirements`);
	console.log("Unaccepted-license negative control failed as required");
	const generated = spawnSync("cargo", [...args, "about.toml", "notices.hbs", "-o", process.argv[2]], {
		stdio: "inherit",
	});
	assert.equal(generated.status, 0, "cargo-about notices generation failed");
} finally {
	rmSync(directory, { recursive: true, force: true });
}

// cargo-about prints each crate's license, but Apache-2.0 also asks for tsrs's NOTICE, and the binary embeds
// roblox-ts's runtime library, which no crate declares.
const metadata = spawnSync("cargo", ["metadata", "--format-version", "1", "--locked", ...manifest], {
	encoding: "utf8",
	maxBuffer: 64 * 1024 * 1024,
});
assert.equal(metadata.status, 0, metadata.stderr);
const tsrs = JSON.parse(metadata.stdout).packages.find(({ name }) => name === "tsrs_core");
assert.ok(tsrs, "the crucible binary links tsrs");
const rule = "=".repeat(80);
appendFileSync(
	process.argv[2],
	[
		`${rule}\ntsrs NOTICE (${tsrs.source})\n\n${readFileSync(join(dirname(tsrs.manifest_path), "../../NOTICE"), "utf8")}`,
		`${rule}\nroblox-ts 3.0.0 runtime library (RuntimeLib.lua and Promise.lua), MIT\n\n${readFileSync("crates/compiler/roblox-ts/LICENSE", "utf8")}`,
	].join("\n"),
);
