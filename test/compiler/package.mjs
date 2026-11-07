import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";
const require = createRequire(import.meta.url);
const repo = path.resolve(import.meta.dirname, "../..");
const project = path.resolve(process.argv[2] ?? "packages/crucible");
for (const [command, args] of [
	[
		"cargo",
		[
			"run",
			"--quiet",
			"--locked",
			"--manifest-path",
			path.join(repo, "Cargo.toml"),
			"-p",
			"crucible-compiler",
			"--example",
			"compile",
			"--",
			path.join(project, "tsconfig.json"),
			path.join(project, "out"),
		],
	],
	[
		process.execPath,
		[
			path.join(path.dirname(require.resolve("tsrs/package.json")), "bin/tsrs"),
			"-p",
			path.join(project, "tsconfig.json"),
			"--noEmit",
			"false",
			"--declaration",
			"--emitDeclarationOnly",
			"--stripInternal",
			"--outDir",
			path.join(project, "out"),
		],
	],
]) {
	const result = spawnSync(command, args, { stdio: "inherit" });
	if (result.error) throw result.error;
	if (result.status !== 0) process.exit(result.status ?? 1);
}
