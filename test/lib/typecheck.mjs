import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = path.resolve(import.meta.dirname, "../..");
export function typecheck(files, options = {}) {
	const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-types-"));
	try {
		fs.symlinkSync(
			path.join(repo, "examples/sample-game/node_modules"),
			path.join(scratch, "node_modules"),
			"junction",
		);
		fs.mkdirSync(path.join(scratch, "src"));
		for (const side of ["shared", "server", "client"])
			fs.symlinkSync(
				path.join(repo, "examples/sample-game/src", side),
				path.join(scratch, "src", side),
				"junction",
			);
		const names = [];
		for (const [name, code] of files) {
			const file = path.join(scratch, "src", path.relative(path.join(repo, "examples/sample-game/src"), name));
			fs.mkdirSync(path.dirname(file), { recursive: true });
			fs.writeFileSync(file, code);
			names.push(file);
		}
		fs.writeFileSync(
			path.join(scratch, "tsconfig.json"),
			JSON.stringify({
				extends: path.join(repo, "examples/sample-game/tsconfig.json"),
				compilerOptions: { noEmit: true, rootDir: scratch, ...options },
				files: names,
				include: [],
			}),
		);
		const result = spawnSync(
			process.execPath,
			[
				path.join(path.dirname(require.resolve("tsrs/package.json")), "bin/tsrs"),
				"-p",
				path.join(scratch, "tsconfig.json"),
				"--pretty",
				"false",
			],
			{ encoding: "utf8" },
		);
		if (result.error) throw result.error;
		return { status: result.status, output: result.stdout + result.stderr };
	} finally {
		fs.rmSync(scratch, { recursive: true, force: true });
	}
}
