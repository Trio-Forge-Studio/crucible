import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";

const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-reactive-mutants-"));

const OWNERSHIP = "packages/crucible/src/internal/reactive/ownership.ts";
const LIFECYCLE = "packages/crucible/src/internal/lifecycle.ts";
const RUNNER = "packages/crucible/src/internal/runner.ts";
const BOOT = "packages/crucible/src/internal/boot.ts";
const SERVER_SYNC = "packages/crucible/src/internal/sync/server-sync.ts";
const CHARM = "packages/crucible/src/internal/reactive/charm.ts";

// [name, kind, rules, file, [[before, after], ...]]. Kinds: guard (a reset or guard deleted), swap (two operands of
// the same type exchanged), pair (one half of a paired operation short-circuited). Charm owns values, so the targets
// are the lifetimes Crucible keeps on it.
const mutants = [
	[
		"interval and generation swapped",
		"swap",
		[21, 22],
		OWNERSHIP,
		[["enabling() === s ? interval(s) : generation(s)", "enabling() === s ? generation(s) : interval(s)"]],
	],
	[
		"listed children never end",
		"guard",
		[23],
		OWNERSHIP,
		[["		unlist(child);\n		dispose(child);", "		unlist(child);"]],
	],
	[
		"a child's end loses the next sibling",
		"swap",
		[23],
		OWNERSHIP,
		[
			[
				"		unlist(child);\n		dispose(child);\n		child = owner.first;",
				"		const after = child.next;\n		unlist(child);\n		dispose(child);\n		child = after;",
			],
		],
	],
	[
		"unlisting keeps the head",
		"swap",
		[27],
		OWNERSHIP,
		[
			[
				"	if (owner.prev === undefined) parent.first = owner.next;",
				"	if (owner.prev === undefined) parent.first = owner;",
			],
		],
	],
	[
		"a scope never arms its run",
		"guard",
		[23],
		OWNERSHIP,
		[["	if (parent.role === Role.Run) arm(parent);\n", ""]],
	],
	[
		"a disposed run keeps its body",
		"guard",
		[36],
		OWNERSHIP,
		[
			[
				"		if (owner.body === undefined) return;\n		owner.body = undefined;\n",
				"		if (owner.body === undefined) return;\n",
			],
		],
	],
	["effects outside a run are not listed", "guard", [20], OWNERSHIP, [["		adopt(owner, run);\n", ""]]],
	[
		"a run has no context",
		"pair",
		[23],
		OWNERSHIP,
		[["call(body, undefined, undefined, run.state, run);", "call(body, undefined, undefined, run.state);"]],
	],
	[
		"a run outside its lifecycle scope",
		"pair",
		[13],
		OWNERSHIP,
		[
			["	openScope();\n	const [result, value] = call(body,", "	const [result, value] = call(body,"],
			["	if (closeScope()) drain();\n	if (!failed) return;", "	if (!failed) return;"],
		],
	],
	["a failed run not rethrown", "pair", [17, 18], OWNERSHIP, [["	if (!run.creating) error(failure, 0);\n", ""]]],
	[
		"a failed first run thrown inside Charm",
		"swap",
		[17],
		OWNERSHIP,
		[["	if (!run.creating) error(failure, 0);\n	run.failure = failure;", "	error(failure, 0);"]],
	],
	[
		"a failed first run not rethrown",
		"pair",
		[17, 36],
		OWNERSHIP,
		[
			[
				'		settle("an effect\'s first run failed");\n		error(failure, 0);',
				'		settle("an effect\'s first run failed");',
			],
		],
	],
	[
		"a nested effect created as a root",
		"swap",
		[26],
		OWNERSHIP,
		[["setActiveSub(nested ? lifetime.node : undefined)", "setActiveSub(undefined)"]],
	],
	[
		"a disposed nested effect keeps its children",
		"guard",
		[26],
		OWNERSHIP,
		[["		for (const child of children) dispose(child);\n", ""]],
	],
	[
		"a disposed nested effect never halts",
		"guard",
		[26],
		OWNERSHIP,
		[["	finish(run);\n	halt(run);\n}", "	finish(run);\n}"]],
	],
	[
		"a halted nested effect keeps its parent link",
		"guard",
		[26],
		OWNERSHIP,
		[["	run.node.subs = undefined;\n	run.node.subsTail = undefined;\n", ""]],
	],
	[
		"a self-disposed entry is never stopped",
		"guard",
		[36],
		OWNERSHIP,
		[["	if (run.body === undefined) run.stop?.();\n", ""]],
	],
	[
		"a failed first run keeps its run",
		"guard",
		[36],
		OWNERSHIP,
		[["		dispose(run);\n		settle(", "		settle("]],
	],
	[
		"a first run's reached failure keeps its run",
		"guard",
		[17],
		OWNERSHIP,
		[
			[
				"	if (flushed) return;\n	dispose(run);\n	error(err, 0);",
				"	if (flushed) return;\n	error(err, 0);",
			],
		],
	],
	[
		"a first run's reached failure not rethrown",
		"pair",
		[17],
		OWNERSHIP,
		[
			[
				"	if (flushed) return;\n	dispose(run);\n	error(err, 0);",
				"	if (flushed) return;\n	dispose(run);",
			],
		],
	],
	[
		"a first run not batched",
		"pair",
		[17],
		OWNERSHIP,
		[
			[
				"	startBatch();\n	const [ok, stop] = pcall(charmEffect,",
				"	const [ok, stop] = pcall(charmEffect,",
			],
			['		settle("an effect\'s first run failed");\n', ""],
			[
				"	const [flushed, err] = pcall(endBatch);\n	if (flushed) return;\n	dispose(run);\n	error(err, 0);\n",
				"",
			],
		],
	],
	[
		"a failed first run's reached failure unreported",
		"guard",
		[17],
		OWNERSHIP,
		[['		settle("an effect\'s first run failed");', "		pcall(endBatch);"]],
	],
	[
		"boot holds nothing",
		"guard",
		[36, 76],
		OWNERSHIP,
		[["	if (pending !== undefined) return false;\n	pending = [];\n	return true;", "	return false;"]],
	],
	[
		"commit skips no unlisted entry",
		"guard",
		[36],
		OWNERSHIP,
		[
			[
				"		const lifetime = run.parent;\n		if (lifetime === undefined) continue;",
				"		const lifetime = run.parent!;",
			],
		],
	],
	[
		"a watch its source disposed still changes",
		"guard",
		[36],
		OWNERSHIP,
		[["(running()[1] as Owner).body === undefined || rawequal(value, previous)", "rawequal(value, previous)"]],
	],
	[
		"a watch baseline read without its owner",
		"pair",
		[15],
		OWNERSHIP,
		[["call(peek, source, undefined, owner.state, owner)", "call(peek, source, undefined)"]],
	],
	[
		"a failed watch baseline leaves tracking off",
		"pair",
		[15],
		OWNERSHIP,
		[
			["	const active = getActiveSub();\n	const [result, baseline]", "	const [result, baseline]"],
			[
				"	setActiveSub(active);\n	if (result !== CallResult.Ok) error(baseline, 0);",
				"	if (result !== CallResult.Ok) error(baseline, 0);",
			],
		],
	],
	[
		"a scope setup outside its lifecycle scope",
		"pair",
		[13],
		OWNERSHIP,
		[
			["	openScope();\n	const [result, value] = call(setup,", "	const [result, value] = call(setup,"],
			[
				"	if (closeScope()) drain();\n	if (result !== CallResult.Ok) error(value, 0);",
				"	if (result !== CallResult.Ok) error(value, 0);",
			],
		],
	],
	[
		"a failed setup not rethrown",
		"pair",
		[23],
		OWNERSHIP,
		[
			[
				"	if (result !== CallResult.Ok) error(value, 0);\n	return () => dispose(scope);",
				"	return () => dispose(scope);",
			],
		],
	],
	[
		"engine release failure unreported",
		"pair",
		[28],
		OWNERSHIP,
		[["	if (!ok) pcall(reportRelease,", "	if (false as boolean) pcall(reportRelease,"]],
	],
	[
		"connection released as an Instance",
		"swap",
		[28],
		OWNERSHIP,
		[
			[
				"	if (kind === Release.Disconnect) engine = disconnect;",
				"	if (kind === Release.Disconnect) engine = destroyInstance;",
			],
		],
	],
	[
		"Destroy() preferred over destroy()",
		"swap",
		[28],
		OWNERSHIP,
		[
			[
				'		if (typeIs((value as DestroyFields).destroy, "function")) return Release.DestroyMethod;\n		if (typeIs((value as DestroyFields).Destroy, "function")) return Release.DestroyUpperMethod;',
				'		if (typeIs((value as DestroyFields).Destroy, "function")) return Release.DestroyUpperMethod;\n		if (typeIs((value as DestroyFields).destroy, "function")) return Release.DestroyMethod;',
			],
		],
	],
	[
		"table method kinds swapped",
		"swap",
		[28],
		OWNERSHIP,
		[["		else if (kind === Release.DestroyMethod)", "		else if (kind === Release.DestroyUpperMethod)"]],
	],
	["nil cleanup appended", "guard", [28], OWNERSHIP, [["	if (release === undefined) return;\n", ""]]],
	[
		"failure blamed on the next release",
		"swap",
		[28],
		OWNERSHIP,
		[
			[
				"	const at = batch.at;\n	reportRelease(",
				"	const at = math.max(batch.at - 2, 0);\n	reportRelease(",
			],
		],
	],
	[
		"release after a failure skipped",
		"guard",
		[28],
		OWNERSHIP,
		[["		i = batch.at - 2;", "		i = batch.at - 4;"]],
	],
	[
		"failed release never closes its scope",
		"pair",
		[13, 28],
		OWNERSHIP,
		[["	batch.scoped = false;\n	if (closeScope()) drain();\n}", "}"]],
	],
	[
		"a hook is not batched",
		"pair",
		[13],
		LIFECYCLE,
		[
			[
				"	startBatch();\n	const [result, value] = call(callback, a, b, owner);\n	const [ok, err] = pcall(endBatch);\n	if (!ok) effects().effectFailed(owner, hook, err);",
				"	const [result, value] = call(callback, a, b, owner);",
			],
		],
	],
	[
		"a scope callback leaves its batch open",
		"pair",
		[20],
		LIFECYCLE,
		[
			[
				"		const [ok, err] = pcall(endBatch);\n		if (!ok) effects().effectFailed(owner ?? blame, hook, err);",
				"",
			],
		],
	],
	[
		"lookup ignores a thread a call started",
		"guard",
		[20],
		RUNNER,
		[
			[
				"	if (depth > 0 && (threads[depth - 1] === thread || !owners.has(thread)))",
				"	if (depth > 0 && threads[depth - 1] === thread)",
			],
		],
	],
	["boot never ends intervals", "pair", [21, 75], BOOT, [["		disabled: endInterval,\n", ""]]],
	[
		"a projection a failed flush skipped is never examined",
		"guard",
		[13],
		SERVER_SYNC,
		[["				if (!projection.dirty && skipped(projection)) this.changed = true;\n", ""]],
	],
	[
		"a quiet scan reads every peer",
		"guard",
		[70],
		SERVER_SYNC,
		[["peers < size && visits < SYNC_HEARTBEAT_SCAN", "peers < size"]],
	],
	[
		"a quiet scan's peers are not visits",
		"guard",
		[70],
		SERVER_SYNC,
		[["			this.scanned += 1;\n			visits += 1;\n", "			this.scanned += 1;\n"]],
	],
	[
		"a quiet scan's reads are not visits",
		"pair",
		[70],
		SERVER_SYNC,
		[["			visits = this.scan();", "			this.scan();"]],
	],
	[
		"a quiet scan starts at the first peer",
		"swap",
		[70],
		SERVER_SYNC,
		[
			[
				"		const size = this.order.size();\n		let visits = 0;",
				"		const size = this.order.size();\n		this.scanned = 0;\n		let visits = 0;",
			],
		],
	],
	[
		"a leaving peer moves the scan past the next peer",
		"guard",
		[70],
		SERVER_SYNC,
		[["		if (index < this.scanned) this.scanned -= 1;\n", ""]],
	],
	[
		"a projection a failed flush skipped is never pulled",
		"guard",
		[13],
		SERVER_SYNC,
		[["	if (!projection.dirty && !skipped(projection)) return;", "	if (!projection.dirty) return;"]],
	],
	[
		"a committed boot keeps its effects held",
		"guard",
		[36, 76],
		BOOT,
		[["	if (holding) releaseEffects(true);\n", ""]],
	],
	[
		"held effects released between booted and the network commit",
		"swap",
		[76],
		BOOT,
		[
			[
				'			unbooted = side.booted();\n			if (side.name === "client") sealDeclarations();',
				'			unbooted = side.booted();\n			releaseEffects(true);\n			if (side.name === "client") sealDeclarations();',
			],
		],
	],
	["a Charm that clears skipped flags loads", "guard", [13], CHARM, [["	checkSkip();\n", ""]]],
	["a Charm whose stop keeps a nested effect's reads loads", "guard", [26], CHARM, [["	checkHalt();\n", ""]]],
	["a Charm that runs every queued effect refused", "guard", [13], CHARM, [["runs === 2 || ", ""]]],
	["Charm never checked", "guard", [13, 26], CHARM, [["	if (checked) return;\n", "	if (!checked) return;\n"]]],
	[
		"a check inside a batch or a flush runs on",
		"guard",
		[13],
		CHARM,
		[["		if (runs === 1) unflushed();\n", ""]],
	],
	[
		"a batch end that runs no effect passes the check",
		"guard",
		[13],
		CHARM,
		[["	if (!threw) unflushed();\n", ""]],
	],
	["a check leaves its caller's subscriber unset", "guard", [13], CHARM, [["	setActiveSub(active);\n", ""]]],
	[
		"a refused check leaves its effects subscribed",
		"swap",
		[13],
		CHARM,
		[
			[
				"	stopParent();\n	stopChild?.();\n	if (!ok) error(err, 0);",
				"	if (!ok) error(err, 0);\n	stopParent();\n	stopChild?.();",
			],
		],
	],
	[
		"a refused check marks Charm checked",
		"guard",
		[13],
		CHARM,
		[["	if (checked) return;\n", "	if (checked) return;\n	checked = true;\n"]],
	],
];

function run(command, args, timeout = 300000) {
	const result = spawnSync(command, args, { cwd: scratch, encoding: "utf8", timeout, shell: false });
	if (result.error?.code === "ETIMEDOUT") return { ...result, status: "timeout", error: undefined };
	assert.equal(result.error, undefined, String(result.error));
	return result;
}

function failing(output) {
	return [...output.matchAll(/^FAIL (reactive-[\w-]+\.luau): (.*)$/gm)].map((match) => `${match[1]}: ${match[2]}`);
}

function names(rule) {
	return new RegExp(rule === "limits" ? "\\blimits\\b" : `\\brule ${rule}\\b`);
}

try {
	// reactive-charm's P4 reads the built place's rbxts_include.
	fs.mkdirSync(path.join(scratch, "build"), { recursive: true });
	fs.copyFileSync(path.join(repo, "build/sample-game.rbxl"), path.join(scratch, "build/sample-game.rbxl"));
	for (const name of ["tsconfig.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	fs.cpSync(path.join(repo, ".lune"), path.join(scratch, ".lune"), { recursive: true });
	fs.cpSync(path.join(repo, "crates/compiler/roblox-ts"), path.join(scratch, "crates/compiler/roblox-ts"), {
		recursive: true,
	});
	fs.cpSync(path.join(repo, "test"), path.join(scratch, "test"), { recursive: true });
	fs.cpSync(path.join(repo, "packages/crucible"), path.join(scratch, "packages/crucible"), {
		recursive: true,
		filter: (p) => !p.split(path.sep).includes("node_modules"),
	});
	fs.symlinkSync(path.join(repo, "node_modules"), path.join(scratch, "node_modules"), "junction");
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules"),
		path.join(scratch, "packages/crucible/node_modules"),
		"junction",
	);
	const files = new Set(mutants.map((mutant) => mutant[3]));
	const originals = new Map([...files].map((file) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const build = () => run(process.execPath, [path.join(repo, "test/lib/compile.mjs"), "packages/crucible"]);
	assert.equal(build().status, 0, "baseline build");
	const baseline = run("lune", ["run", "test", "reactive"]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	const first = Number(process.argv[2] ?? 0);
	const kinds = new Map();
	let killed = 0;
	for (const [index, [name, kind, rules, file, edits]] of mutants.entries()) {
		if (index < first) continue;
		const original = originals.get(file);
		let mutated = original;
		for (const [before, after] of edits) {
			assert.equal(mutated.split(before).length, 2, `${name}: pattern must occur once in ${file}: ${before}`);
			mutated = mutated.replace(before, after);
		}
		fs.writeFileSync(path.join(scratch, file), mutated);
		const compiled = build();
		assert.equal(compiled.status, 0, `${name} build failed\n${compiled.stdout}${compiled.stderr}`);
		const tested = run("lune", ["run", "test", ...rules.map((rule) => `rule ${rule} `)], 120000);
		const failed = failing(`${tested.stdout ?? ""}${tested.stderr ?? ""}`);
		const owned = failed.filter((spec) => rules.some((rule) => names(rule).test(spec)));
		fs.writeFileSync(path.join(scratch, file), original);
		assert.notEqual(tested.status, 0, `${name} survived\n${tested.stdout}${tested.stderr}`);
		// Unbounded recursion can abort Lune, and an unbounded flush can hang it, before it reports. Only the rule's
		// specs ran, so the abort or the hang is theirs.
		const crashed = !/\d+ passed, \d+ failed/.test(tested.stdout ?? "");
		if (crashed)
			owned.push(`${tested.status === "timeout" ? "hung" : "crashed"} in the specs for rule ${rules.join(", ")}`);
		assert.ok(
			owned.length > 0,
			`${name} was not caught by a spec for rule ${rules.join(", ")}: ${failed.join("; ")}`,
		);
		killed += 1;
		for (const rule of rules) kinds.set(rule, new Set([...(kinds.get(rule) ?? []), kind]));
		console.log(
			`KILLED ${index} ${name} | ${kind} | rules ${rules.join(",")} | ${failed.length} specs failed | ${owned[0]}`,
		);
	}
	assert.equal(build().status, 0, "restored build");
	console.log(`${killed}/${mutants.length - first} reactive source mutants killed; restored build 0`);
	for (const [rule, set] of [...kinds].sort((a, b) =>
		String(a[0]).localeCompare(String(b[0]), "en", { numeric: true }),
	))
		console.log(`rule ${rule}: ${[...set].sort().join(", ")}`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
