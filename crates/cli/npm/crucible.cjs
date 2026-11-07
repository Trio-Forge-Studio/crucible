#!/usr/bin/env node
const { spawnSync } = require("node:child_process");

const platformPackage = `@trioforge/crucible-cli-${process.platform}-${process.arch}`;
const executable = process.platform === "win32" ? "crucible.exe" : "crucible";

let binary;
try {
	binary = require.resolve(`${platformPackage}/${executable}`);
} catch {
	process.stderr.write(
		`crucible: ${platformPackage} is not installed. Crucible has no prebuilt binary for ${process.platform}-${process.arch}, or optional dependencies were skipped during install.\n`,
	);
	process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
	throw result.error;
}
if (result.signal) {
	process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
