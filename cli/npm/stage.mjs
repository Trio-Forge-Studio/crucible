import { chmodSync, copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const cli = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export const PLATFORMS = [
	{ os: "win32", cpu: "x64" },
	{ os: "linux", cpu: "x64" },
];

const repository = { type: "git", url: "git+https://github.com/Trio-Forge-Studio/crucible.git", directory: "cli" };
const shared = { license: "MIT", repository };

export function cliVersion() {
	const manifest = readFileSync(join(cli, "Cargo.toml"), "utf8");
	const version = /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/.exec(manifest)?.[1];
	if (!version) {
		throw new Error("cli/Cargo.toml has no [workspace.package] version");
	}
	return version;
}

export function platformPackageName({ os, cpu }) {
	return `@trioforge/crucible-cli-${os}-${cpu}`;
}

function writeJson(path, value) {
	writeFileSync(path, `${JSON.stringify(value, null, "\t")}\n`);
}

export function stage(out, binaries, notices) {
	const version = cliVersion();
	rmSync(out, { recursive: true, force: true });
	const staged = [];

	const shim = join(out, "crucible");
	mkdirSync(join(shim, "bin"), { recursive: true });
	copyFileSync(join(cli, "npm", "crucible.cjs"), join(shim, "bin", "crucible.cjs"));
	writeJson(join(shim, "package.json"), {
		name: "@trioforge/crucible-cli",
		version,
		description: "The command line for Crucible games.",
		...shared,
		bin: { crucible: "bin/crucible.cjs" },
		files: ["bin"],
		engines: { node: ">=18" },
		optionalDependencies: Object.fromEntries(PLATFORMS.map((platform) => [platformPackageName(platform), version])),
	});
	copyFileSync(join(cli, "..", "LICENSE"), join(shim, "LICENSE"));
	staged.push(shim);

	for (const [key, binary] of Object.entries(binaries)) {
		const platform = PLATFORMS.find(({ os, cpu }) => `${os}-${cpu}` === key);
		if (!platform) {
			throw new Error(
				`unsupported platform ${key}. Supported: ${PLATFORMS.map(({ os, cpu }) => `${os}-${cpu}`).join(", ")}`,
			);
		}
		const executable = platform.os === "win32" ? "crucible.exe" : "crucible";
		const directory = join(out, `crucible-cli-${key}`);
		mkdirSync(directory, { recursive: true });
		copyFileSync(binary, join(directory, executable));
		chmodSync(join(directory, executable), 0o755);
		writeJson(join(directory, "package.json"), {
			name: platformPackageName(platform),
			version,
			description: `The crucible binary for ${key}.`,
			...shared,
			os: [platform.os],
			cpu: [platform.cpu],
			files: [executable, "THIRD_PARTY_NOTICES"],
			preferUnplugged: true,
		});
		copyFileSync(join(cli, "..", "LICENSE"), join(directory, "LICENSE"));
		if (notices) {
			copyFileSync(notices, join(directory, "THIRD_PARTY_NOTICES"));
		}
		staged.push(directory);
	}
	return staged;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
	const [out, ...pairs] = process.argv.slice(2);
	if (!out || pairs.length === 0 || pairs.some((pair) => !pair.includes("="))) {
		process.stderr.write("Usage: node npm/stage.mjs <out> <os>-<cpu>=<binary> [...]\n");
		process.exit(2);
	}
	const binaries = Object.fromEntries(pairs.map((pair) => pair.split(/=(.*)/s, 2)));
	for (const directory of stage(resolve(out), binaries)) {
		process.stdout.write(`${directory}\n`);
	}
}
