import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const root = path.resolve(import.meta.dirname, "..");
const node = (args) => spawnSync(process.execPath, args, { cwd: root, encoding: "utf8" });
const lune = () => spawnSync("lune", ["run", "test", "compiled sample"], { cwd: root, encoding: "utf8" });
const rebuild = () => {
	const sample = path.join(root, "examples/sample-game");
	fs.rmSync(path.join(sample, "out"), { recursive: true, force: true });
	const compiled = node([path.join(sample, "build.mjs")]);
	if (compiled.status !== 0) throw Error(compiled.stdout + compiled.stderr);
	const place = spawnSync(
		"rojo",
		["build", "examples/sample-game/default.project.json", "-o", "build/sample-game.rbxl"],
		{ cwd: root, encoding: "utf8" },
	);
	if (place.status !== 0) throw Error(place.stdout + place.stderr);
};
const mutants = [
	[
		"probe check paired command skipped",
		"package.json",
		"selene --config probes/engine/selene.toml probes && ",
		"",
		() => node(["--test", "test/studio/verification.test.mjs"]),
	],
	[
		"startup launch cap guard changed",
		"test/studio/startup.mjs",
		"attempt <= 3",
		"attempt <= 2",
		() => node(["--test", "test/studio/verification.test.mjs"]),
	],
	[
		"fingerprint guard reset deleted",
		"examples/sample-game/build.mjs",
		'fs.rmSync(path.join(sample, "out"), { recursive: true, force: true });',
		"{}",
		() => node(["--test", "test/sample-build.test.mjs"]),
	],
	[
		"fingerprint paired hash update short circuited",
		"examples/sample-game/build.mjs",
		"hash.update(fs.readFileSync(path.join(transformer, name)));",
		"{}",
		() => node(["--test", "test/sample-build.test.mjs"]),
	],
	[
		"fixture real declaration copy deleted",
		"packages/crucible/transformer-src/tests/transformer.test.cjs",
		'["out", "server", "client", "emit", "package.json"]',
		'["server", "client", "emit", "package.json"]',
		() =>
			node([
				"--test",
				"--test-name-pattern=fixture uses every real",
				"packages/crucible/transformer-src/tests/transformer.test.cjs",
			]),
	],
	[
		"owned PID place guard deleted",
		"test/studio/session.mjs",
		"&& args.includes(place.toLowerCase())",
		"",
		() => node(["--test", "test/studio/verification.test.mjs"]),
	],
	[
		"owned PID same typed selector arguments swapped",
		"test/studio/session.mjs",
		"ownsProcess(commandLine, place)",
		"ownsProcess(place, commandLine)",
		() => node(["--test", "test/studio/verification.test.mjs"]),
	],
	[
		"startup retry paired wait deleted",
		"test/studio/startup.mjs",
		"await wait(60000);",
		"{}",
		() => node(["--test", "test/studio/verification.test.mjs"]),
	],
	[
		"fingerprint same typed receipt arguments swapped",
		"examples/sample-game/build.mjs",
		"fs.writeFileSync(receipt, fingerprint);",
		"fs.writeFileSync(fingerprint, receipt);",
		() => node(["--test", "test/sample-build.test.mjs"]),
	],
	[
		"fixture paired manifest copy skipped",
		"packages/crucible/transformer-src/tests/transformer.test.cjs",
		'["out", "server", "client", "emit", "package.json"]',
		'["out", "server", "client", "emit"]',
		() =>
			node([
				"--test",
				"--test-name-pattern=fixture uses every real",
				"packages/crucible/transformer-src/tests/transformer.test.cjs",
			]),
	],
	[
		"fixture same typed side destinations swapped",
		"packages/crucible/transformer-src/tests/transformer.test.cjs",
		"path.join(installed, entry), { recursive: true }",
		'path.join(installed, entry === "server" ? "client" : entry === "client" ? "server" : entry), { recursive: true }',
		() =>
			node([
				"--test",
				"--test-name-pattern=fixture uses every real",
				"packages/crucible/transformer-src/tests/transformer.test.cjs",
			]),
	],
	[
		"owned session ambiguity check short circuited",
		"test/studio/session.mjs",
		"if (matches.length > 1)",
		"if (matches.length < 0)",
		() => node(["--test", "test/studio/verification.test.mjs"]),
	],
	[
		"sample required attribute guard removed",
		"examples/sample-game/src/server/systems/member.ts",
		"attribute<string>()",
		'attribute("unguarded")',
		lune,
	],
	[
		"sample boot and flag paired operation short circuited",
		"examples/sample-game/src/server/main.server.ts",
		'script.SetAttribute("CrucibleServer", true);',
		"",
		lune,
	],
	[
		"sample same typed load paths swapped",
		"examples/sample-game/src/server/main.server.ts",
		"src/server/systems/**",
		"src/client/systems/**",
		lune,
	],
];
rebuild();
const baseline = lune();
if (baseline.status !== 0) throw Error(baseline.stdout + baseline.stderr);
console.log("BASELINE sample build and compiled lifecycle specs 0");
for (const [name, file, before, after, verify] of mutants) {
	const target = path.join(root, file);
	const original = fs.readFileSync(target, "utf8");
	if (!original.includes(before)) throw Error(`Missing target ${name}`);
	try {
		fs.writeFileSync(target, original.replace(before, after));
		if (file.endsWith(".ts")) rebuild();
		else if (file.endsWith(".json")) JSON.parse(fs.readFileSync(target, "utf8"));
		else {
			const syntax = node(["--check", target]);
			if (syntax.status !== 0) throw Error(syntax.stderr);
		}
		const result = verify();
		if (result.status !== 1) throw Error(`${name} survived or failed to run\n${result.stdout}\n${result.stderr}`);
		console.log(`KILLED ${name}; rebuild or syntax 0; named spec 1`);
	} finally {
		fs.writeFileSync(target, original);
	}
}
rebuild();
const restored = lune();
if (restored.status !== 0) throw Error(restored.stdout + restored.stderr);
console.log("RESTORED sample build and compiled lifecycle specs 0");
