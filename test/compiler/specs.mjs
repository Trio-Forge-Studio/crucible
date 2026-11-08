import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";

const repo = process.cwd();
const run = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-compiler-specs-"));
const place = path.join(repo, "build/sample-game.rbxl");
const declarations = path.join(repo, "packages/crucible/out");
if (!fs.existsSync(place) || !fs.existsSync(path.join(declarations, "index.d.ts"))) {
	console.error("run `pnpm build` first");
	process.exit(1);
}

function step(command, args, options = {}) {
	const result = spawnSync(command, args, { cwd: repo, stdio: "inherit", ...options });
	if (result.status !== 0) process.exit(result.status ?? 1);
}

fs.rmSync(run, { recursive: true, force: true });
fs.mkdirSync(path.join(run, "packages/crucible"), { recursive: true });
fs.mkdirSync(path.join(run, "build"));

const out = path.join(run, "packages/crucible/out");
const game = path.join(run, "sample-game-out");
const compile = (...args) =>
	step("cargo", [
		"run",
		"--quiet",
		"--locked",
		"--manifest-path",
		"Cargo.toml",
		"-p",
		"crucible-transform",
		"--example",
		"compile-framework",
		"--",
		...args,
	]);
compile("packages/crucible/tsconfig.json", out);
compile(
	"examples/sample-game/tsconfig.json",
	game,
	"examples/sample-game/default.project.json",
	"examples/sample-game/include",
);

(function copyDeclarations(dir) {
	for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
		const from = path.join(dir, entry.name);
		if (entry.isDirectory()) copyDeclarations(from);
		else if (entry.name.endsWith(".d.ts")) {
			const to = path.join(out, path.relative(declarations, from));
			fs.mkdirSync(path.dirname(to), { recursive: true });
			fs.copyFileSync(from, to);
		}
	}
})(declarations);

// A Windows junction can only name a folder, so files are copied.
const link = (from, to) =>
	fs.statSync(from).isDirectory() ? fs.symlinkSync(from, to, "junction") : fs.copyFileSync(from, to);
for (const name of fs.readdirSync(repo)) {
	if (!["packages", "build", ".git"].includes(name)) link(path.join(repo, name), path.join(run, name));
}
for (const name of fs.readdirSync(path.join(repo, "packages"))) {
	if (name !== "crucible") link(path.join(repo, "packages", name), path.join(run, "packages", name));
}
for (const name of fs.readdirSync(path.join(repo, "packages/crucible"))) {
	if (name !== "out") link(path.join(repo, "packages/crucible", name), path.join(run, "packages/crucible", name));
}

step("lune", ["run", "test/compiler/place", place, out, game, path.join(run, "build/sample-game.rbxl")]);
step("lune", ["run", "test", ...process.argv.slice(2)], { cwd: run });
