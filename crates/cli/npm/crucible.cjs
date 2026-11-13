#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const { copyFileSync, mkdtempSync, readdirSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { dirname, join } = require("node:path");

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

// Windows cannot delete a running binary, so an upgrade run from 0.5.0 or older leaves the old one where npm
// (.crucible-cli-win32-x64-<random>) or pnpm (crucible-cli-win32-x64_pacquet-stage_<n>) moved it aside.
try {
	const scope = dirname(dirname(binary));
	for (const name of readdirSync(scope)) {
		if (/^\.crucible-cli-|^crucible-cli-.+_pacquet-stage_/.test(name)) {
			rmSync(join(scope, name), { recursive: true, force: true });
		}
	}
} catch {}

const args = process.argv.slice(2);
let command = binary;
let copy;
// The upgrade's install replaces this package's folder, which Windows refuses while the binary in it runs.
if (process.platform === "win32" && args[0] === "upgrade") {
	try {
		copy = mkdtempSync(join(tmpdir(), "crucible-upgrade-"));
		command = join(copy, executable);
		copyFileSync(binary, command);
	} catch (error) {
		process.stderr.write(
			`crucible: cannot copy ${binary} to a temporary folder to run the upgrade: ${error.message}\n`,
		);
		process.exit(1);
	}
}

const result = spawnSync(command, args, { stdio: "inherit" });
if (copy) {
	try {
		rmSync(copy, { recursive: true, force: true, maxRetries: 10 });
	} catch {}
}
if (result.error) {
	throw result.error;
}
if (result.signal) {
	process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
