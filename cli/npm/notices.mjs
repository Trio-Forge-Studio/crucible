import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const directory = mkdtempSync(join(tmpdir(), "crucible-licenses-"));
const args = ["about", "generate", "--locked", "--fail", "--manifest-path", "crates/crucible/Cargo.toml", "--config"];
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
