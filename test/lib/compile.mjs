import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = path.resolve(import.meta.dirname, "../..");
const project = path.resolve(process.argv[2]);
const args = [path.join(project, "tsconfig.json"), path.join(project, "out")];
const rojo = path.join(project, "default.project.json");
if (fs.existsSync(rojo)) args.push(rojo, path.join(project, "include"));
if (process.argv.includes("--check")) {
	const checked = spawnSync(
		process.execPath,
		[
			path.join(path.dirname(require.resolve("tsrs/package.json")), "bin/tsrs"),
			"-p",
			path.join(project, "tsconfig.json"),
			"--pretty",
			"false",
		],
		{ stdio: "inherit" },
	);
	if (checked.error) throw checked.error;
	process.exit(checked.status ?? 1);
}
const result = spawnSync(
	"cargo",
	[
		"run",
		"--quiet",
		"--locked",
		"--manifest-path",
		path.join(repo, "cli/Cargo.toml"),
		"-p",
		"crucible-compiler",
		"--example",
		"compile",
		"--",
		...args,
	],
	{ stdio: "inherit" },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
