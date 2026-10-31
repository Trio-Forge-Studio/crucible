// Mutation campaign for the reactive core and core edits E1 and E2. Each mutant rewrites TypeScript source in a scratch
// copy, rebuilds the package with rbxtsc, and runs the Lune specs named for the mutant's rules. It is killed only when
// one of them fails. Run from the repository root: node test/lib/reactive-mutants.mjs [first index]
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-reactive-mutants-"));

const GRAPH = "packages/crucible/src/internal/reactive/graph.ts";
const OWNERSHIP = "packages/crucible/src/internal/reactive/ownership.ts";
const FLUSH = "packages/crucible/src/internal/reactive/flush.ts";
const CONTEXT = "packages/crucible/src/internal/reactive/context.ts";
const LIMITS = "packages/crucible/src/internal/reactive/limits.ts";
const API = "packages/crucible/src/reactive.ts";
const LIFECYCLE = "packages/crucible/src/internal/lifecycle.ts";
const BOOT = "packages/crucible/src/internal/boot.ts";

// [name, kind, rules, file, [[before, after], ...]]. Kinds: guard (a reset or guard deleted), swap (two operands of
// the same type exchanged), pair (one half of a paired operation short-circuited).
const mutants = [
	["update stores the callback", "pair", [1], API, [["update(this, change);", "write(this, change);"]]],
	["update callback is not pure", "guard", [1], GRAPH, [[`, "update's callback"),`, "),"]]],
	[
		"read without tracking",
		"pair",
		[2],
		API,
		[["__call: (node) => readState(node),", "__call: (node) => node.value,"]],
	],
	[
		"state read skips its edge",
		"pair",
		[2],
		GRAPH,
		[["	track(node);\n	return node.value;", "	return node.value;"]],
	],
	["equal write guard", "guard", [3], GRAPH, [["	if (rawequal(value, node.value)) return;\n", ""]]],
	[
		"computed handle rebuilds its node",
		"pair",
		[4],
		API,
		[
			[
				"const node = createComputedNode(read);\n	return () => readComputed(node) as T;",
				"return () => readComputed(createComputedNode(read)) as T;",
			],
		],
	],
	["equal result moves the version", "guard", [5], GRAPH, [["!rawequal(cache.value, result)", "true"]]],
	["far observers marked dirty", "swap", [5, 33], GRAPH, [["levels.push(Mark.Check);", "levels.push(Mark.Dirty);"]]],
	[
		"failure compared by its traceback",
		"swap",
		[6],
		GRAPH,
		[["cache.reason !== failure.reason", "cache.message !== failure.message"]],
	],
	[
		"failure drops the previous dependencies",
		"guard",
		[6],
		GRAPH,
		[["if (!frame.reads!.has(edge.source)) kept.push", "if (false as boolean) kept.push"]],
	],
	[
		"cached error read as nil",
		"guard",
		[6, 19],
		GRAPH,
		[["	if (cache.kind === CacheKind.Error) error(cache.message, 0);\n", ""]],
	],
	["purity guard", "guard", [7], GRAPH, [["if (pure !== undefined) error(", "if (false as boolean) error("]]],
	[
		"effect purity guard",
		"guard",
		[7],
		FLUSH,
		[
			[
				"export function createEffect(body: () => unknown) {\n	guardPure",
				"export function createEffect(body: () => unknown) {\n	tostring",
			],
		],
	],
	["cycle check", "guard", [8], GRAPH, [["	if (node.evaluating) cycle(node);\n", ""]]],
	[
		"evaluating never cleared",
		"guard",
		[8, 13],
		GRAPH,
		[["	node.evaluating = false;\n	let failure", "	let failure"]],
	],
	[
		"write leaves the epoch",
		"guard",
		[9],
		GRAPH,
		[["	node.version += 1;\n	epoch += 1;", "	node.version += 1;"]],
	],
	[
		"unattached clean cache trusted",
		"guard",
		[9],
		GRAPH,
		[["		if (node.mark === Mark.Clean) node.mark = Mark.Check;\n", ""]],
	],
	[
		"attach registers nothing",
		"pair",
		[10],
		GRAPH,
		[["			source.observers.add(node);\n			if (first", "			if (first"]],
	],
	["stale run not requeued", "pair", [10], FLUSH, [["	markEffect(effect, staleAfterRun(effect));\n", ""]]],
	[
		"untracked frame never popped",
		"pair",
		[11],
		GRAPH,
		[
			[
				"	push(frame);\n	const [ok, result] = pcall(read);\n	pop(frame);",
				"	push(frame);\n	const [ok, result] = pcall(read);",
			],
		],
	],
	[
		"untracked drops ownership",
		"guard",
		[11],
		GRAPH,
		[["lifetime: below?.lifetime, pure: pure ?? below?.pure", "lifetime: undefined, pure: pure ?? below?.pure"]],
	],
	[
		"frames shared by every thread",
		"guard",
		[12, 29],
		CONTEXT,
		[
			["const stacks = setmetatable(", "const anyThread = coroutine.running();\nconst stacks = setmetatable("],
			[
				"export function top(): Frame | undefined {\n	const stack = stacks.get(coroutine.running());",
				"export function top(): Frame | undefined {\n	const stack = stacks.get(anyThread);",
			],
			[
				"export function inBody(): boolean {\n	const stack = stacks.get(coroutine.running());",
				"export function inBody(): boolean {\n	const stack = stacks.get(anyThread);",
			],
			[
				"export function push(frame: Frame) {\n	const thread = coroutine.running();",
				"export function push(frame: Frame) {\n	const thread = anyThread;",
			],
			[
				"export function pop(frame: Frame) {\n	const thread = coroutine.running();",
				"export function pop(frame: Frame) {\n	const thread = anyThread;",
			],
		],
	],
	[
		"open frames never finished",
		"pair",
		[13],
		CONTEXT,
		[["const open = job.worker === undefined ? [] : discard(job.worker);", "const open: Frame[] = [];"]],
	],
	[
		"completion error swallowed",
		"guard",
		[13, 35],
		CONTEXT,
		[["	if (result !== CallResult.Ok) error(value, 0);\n", ""]],
	],
	[
		"deferred endings dropped",
		"pair",
		[13, 25],
		OWNERSHIP,
		[["		if (deferred !== undefined) for (const ending of deferred) finish(ending);\n", ""]],
	],
	[
		"running body never leaves the stack",
		"pair",
		[13, 25],
		OWNERSHIP,
		[["		if (index >= 0) bodies.remove(index);\n", ""]],
	],
	[
		"boundary outside within",
		"pair",
		[16, 32],
		CONTEXT,
		[["within(owner, undefined, Hook.Callback, outer, job, owner)", "call(outer, job, owner, owner)"]],
	],
	[
		"equal computeds do not skip a run",
		"guard",
		[14, 33],
		FLUSH,
		[["			if (mark === Mark.Check && !dependenciesMoved(effect)) continue;\n", ""]],
	],
	[
		"dependencies rewired backwards",
		"swap",
		[14],
		FLUSH,
		[["rewire(effect, effect.dependencies, reads);", "rewire(effect, reads, effect.dependencies);"]],
	],
	["watch equal guard", "guard", [15], FLUSH, [["		if (rawequal(value, previous)) return;\n", ""]]],
	["watch arguments swapped", "swap", [15], FLUSH, [["onChange(value, before)", "onChange(before, value)"]]],
	["watch previous never moves", "guard", [15], FLUSH, [["		previous = value;\n", ""]]],
	[
		"watch baseline read at the first run",
		"pair",
		[15],
		FLUSH,
		[["let previous = readUntracked(source);", "let previous = undefined as unknown as T;"]],
	],
	[
		"run owner dropped",
		"guard",
		[16],
		FLUSH,
		[
			[
				"newLifetime(Role.Run, effect.parent, effect.parent.owner, effect)",
				"newLifetime(Role.Run, effect.parent, undefined, effect)",
			],
		],
	],
	[
		"return check",
		"guard",
		[17],
		FLUSH,
		[
			[
				`	else if (value !== undefined && !typeIs(value, "function"))\n		failure =`,
				`	else if (false as boolean)\n		failure =`,
			],
		],
	],
	[
		"thrown run counted as returned",
		"guard",
		[18],
		FLUSH,
		[["	if (completion.result !== CallResult.Ok) failure = tostring(value);\n	else if", "	if"]],
	],
	[
		"failed effect keeps running",
		"pair",
		[18],
		FLUSH,
		[
			[
				"function fail(effect: EffectNode, reason: string) {\n	retire(effect);",
				"function fail(effect: EffectNode, reason: string) {\n	tostring(effect);",
			],
		],
	],
	[
		"failure never reported",
		"pair",
		[18, 79],
		FLUSH,
		[
			[
				'	report("error", `Crucible: effect at',
				'	if (false as boolean) report("error", `Crucible: effect at',
			],
		],
	],
	[
		"frame owner ignored",
		"guard",
		[20],
		OWNERSHIP,
		[["	if (frame?.lifetime !== undefined) return frame.lifetime;\n", ""]],
	],
	[
		"generation never ends",
		"pair",
		[20],
		OWNERSHIP,
		[
			[
				"			generations.delete(s);\n			endLifetime(created);",
				"			generations.delete(s);",
			],
		],
	],
	[
		"no-owner check",
		"guard",
		[20, 39],
		FLUSH,
		[
			[
				'export function createEffect(body: () => unknown) {\n	guardPure("create effects");\n	const owner = resolveOwner();\n	if (owner === undefined) error(NO_OWNER, 0);',
				'export function createEffect(body: () => unknown) {\n	guardPure("create effects");\n	const owner = resolveOwner()!;',
			],
		],
	],
	[
		"cleanup no-owner check",
		"guard",
		[20],
		OWNERSHIP,
		[
			[
				"	const owner = resolveOwner();\n	if (owner === undefined) error(NO_OWNER, 0);\n	if (owner.ended)",
				"	const owner = resolveOwner()!;\n	if (owner.ended)",
			],
		],
	],
	[
		"interval and generation swapped",
		"swap",
		[21, 22],
		OWNERSHIP,
		[["enabling() === s ? interval(s) : generation(s)", "enabling() === s ? generation(s) : interval(s)"]],
	],
	[
		"ended interval reused",
		"guard",
		[21, 75],
		OWNERSHIP,
		[["	intervals.delete(s);\n	endLifetime(lifetime);", "	endLifetime(lifetime);"]],
	],
	[
		"child scope not adopted",
		"pair",
		[23],
		OWNERSHIP,
		[["	if (parent !== undefined) adopt(parent, scope);\n", ""]],
	],
	[
		"failed setup not ended",
		"pair",
		[23, 24],
		OWNERSHIP,
		[["		failure = completion.value;\n		endLifetime(scope);", "		failure = completion.value;"]],
	],
	["failed setup not rethrown", "pair", [23], OWNERSHIP, [["	if (failed) error(failure, 0);\n", ""]]],
	[
		"ending never waits for a running body",
		"guard",
		[25],
		OWNERSHIP,
		[["		if (!under(body, lifetime)) continue;", "		if (true as boolean) continue;"]],
	],
	[
		"children ended oldest first",
		"swap",
		[25, 26],
		OWNERSHIP,
		[
			[
				"	let child = lifetime.tail;\n	while (child !== undefined) {\n		const previous: Owned | undefined = child.prev;",
				"	let child = lifetime.head;\n	while (child !== undefined) {\n		const previous: Owned | undefined = child.next;",
			],
		],
	],
	[
		"ended lifetime stays linked",
		"pair",
		[25, 27],
		OWNERSHIP,
		[["	releases.clear();\n	leave(lifetime);", "	releases.clear();"]],
	],
	[
		"returned cleanup registered first",
		"swap",
		[26],
		FLUSH,
		[["run.releases.push(value as () => void);", "run.releases.insert(0, value as () => void);"]],
	],
	[
		"previous run never ended",
		"pair",
		[26],
		FLUSH,
		[["	if (previous !== undefined) endLifetime(previous);\n", ""]],
	],
	[
		"ended generation treated as live",
		"guard",
		[27],
		OWNERSHIP,
		[["	if (hasEnded(s)) {", "	if (false as boolean) {"]],
	],
	[
		"effect under an ended owner",
		"guard",
		[27],
		FLUSH,
		[
			[
				"	if (owner === undefined) error(NO_OWNER, 0);\n	if (owner.ended) return;\n	createIn(owner, body);",
				"	if (owner === undefined) error(NO_OWNER, 0);\n	createIn(owner, body);",
			],
		],
	],
	[
		"cleanup under an ended owner deferred",
		"guard",
		[27],
		OWNERSHIP,
		[
			[
				"	if (owner.ended) runRelease(owner, release);\n	else owner.releases.push(release);",
				"	owner.releases.push(release);",
			],
		],
	],
	[
		"children warning repeats",
		"guard",
		[27, 79],
		OWNERSHIP,
		[["parent.count >= OWNER_CHILDREN_WARNING && !parent.warned", "parent.count >= OWNER_CHILDREN_WARNING"]],
	],
	[
		"releases run oldest first",
		"swap",
		[28],
		OWNERSHIP,
		[
			[
				"	for (let i = releases.size() - 1; i >= 0; i--) runRelease(lifetime, releases[i]);",
				"	for (let i = 0; i < releases.size(); i++) runRelease(lifetime, releases[i]);",
			],
		],
	],
	[
		"cleanup failure unreported",
		"pair",
		[28],
		OWNERSHIP,
		[
			[
				"		if (completion.result !== CallResult.Ok)\n			runtimeDiagnostic(",
				"		if (false as boolean)\n			runtimeDiagnostic(",
			],
		],
	],
	[
		"write runs effects synchronously",
		"pair",
		[30, 31, 37],
		FLUSH,
		[["const [ok] = pcall(port.defer, flush);", "const [ok] = pcall(flush);"]],
	],
	["write marks nothing", "pair", [30], GRAPH, [["	epoch += 1;\n	markObservers(node);", "	epoch += 1;"]]],
	[
		"second request",
		"guard",
		[31, 77],
		FLUSH,
		[
			[
				"if (requested || flushing || held || port === undefined) return;",
				"if (flushing || held || port === undefined) return;",
			],
		],
	],
	[
		"request without a port",
		"guard",
		[31],
		FLUSH,
		[
			[
				"if (requested || flushing || held || port === undefined) return;",
				"if (requested || flushing || held) return;",
			],
			["const [ok] = pcall(port.defer, flush);", "const [ok] = pcall(port!.defer, flush);"],
		],
	],
	["batch unsorted", "pair", [33], FLUSH, [["		batch.sort(createdBefore);\n", ""]]],
	["creation order reversed", "swap", [33], GRAPH, [["	return a.id < b.id;", "	return b.id < a.id;"]]],
	[
		"computed dependency not pulled",
		"guard",
		[33],
		GRAPH,
		[
			[
				"		if (source.kind === NodeKind.Computed) validate(source);\n		if (source.version !== edge.seen) return true;",
				"		if (source.version !== edge.seen) return true;",
			],
		],
	],
	[
		"per-effect bound",
		"guard",
		[34],
		FLUSH,
		[["			if (effect.runs === MAX_RUNS_PER_EFFECT) {", "			if (false as boolean) {"]],
	],
	["run counts never reset", "guard", [34], FLUSH, [["	for (const effect of ran) effect.runs = 0;\n", ""]]],
	["bound report never re-armed", "guard", [34], FLUSH, [["	}\n	boundReported = false;\n}", "	}\n}"]]],
	["bound reported every time", "guard", [34, 79], FLUSH, [["	if (boundReported) return;\n", ""]]],
	["total bound drops the effect it stopped at", "pair", [34], FLUSH, [["				cursor -= 1;\n", ""]]],
	[
		"round bound ignored",
		"guard",
		[34],
		FLUSH,
		[["		if (rounds === MAX_FLUSH_ROUNDS) {", "		if (false as boolean) {"]],
	],
	[
		"recovery loses the remainder",
		"pair",
		[35],
		FLUSH,
		[["	const blamed = culprit;\n	carry();", "	const blamed = culprit;\n	batch = [];"]],
	],
	["culprit not latched when stop fails", "guard", [35], FLUSH, [["			blamed.stopped = true;\n", ""]]],
	[
		"flushing never reset",
		"guard",
		[35],
		FLUSH,
		[["	culprit = undefined;\n	flushing = false;", "	culprit = undefined;"]],
	],
	["recovery skipped", "pair", [35], FLUSH, [["	if (!ok) recover(err);\n", ""]]],
	[
		"culprit never stopped",
		"pair",
		[35],
		FLUSH,
		[["const [ok, stopErr] = pcall(retire, blamed);", "const [ok, stopErr] = pcall(tostring, blamed);"]],
	],
	[
		"barrier ignored",
		"guard",
		[36, 76],
		FLUSH,
		[
			[
				"if (requested || flushing || held || port === undefined) return;",
				"if (requested || flushing || port === undefined) return;",
			],
		],
	],
	[
		"release requests nothing",
		"pair",
		[36],
		FLUSH,
		[["	held = false;\n	if (queue.size() > 0) request();", "	held = false;"]],
	],
	[
		"render commit priority",
		"guard",
		[38],
		LIMITS,
		[["RENDER_COMMIT_PRIORITY = 2_001", "RENDER_COMMIT_PRIORITY = 2_000"]],
	],
	["per-effect limit", "guard", [34], LIMITS, [["MAX_RUNS_PER_EFFECT = 64", "MAX_RUNS_PER_EFFECT = 65"]]],
	[
		"enabling mark never cleared",
		"guard",
		[21, 75],
		LIFECYCLE,
		[
			[
				"			run(s, Hook.OnEnable);\n			enablingNow = previous;",
				"			run(s, Hook.OnEnable);",
			],
		],
	],
	[
		"enabling mark saved after set",
		"swap",
		[21, 75],
		LIFECYCLE,
		[
			[
				"			const previous = enablingNow;\n			enablingNow = s;",
				"			enablingNow = s;\n			const previous = enablingNow;",
			],
		],
	],
	[
		"Disable step leaves the interval",
		"pair",
		[21, 75],
		LIFECYCLE,
		[["			run(s, Hook.OnDisable);\n			fx.disabled?.(s);", "			run(s, Hook.OnDisable);"]],
	],
	[
		"release leaves the interval",
		"pair",
		[75],
		LIFECYCLE,
		[["	if (disabled !== undefined) pcall(disabled, s);\n", ""]],
	],
	["boot never ends intervals", "pair", [21, 75], BOOT, [["		disabled: endInterval,\n", ""]]],
	[
		"boot holds nothing",
		"guard",
		[36, 76],
		BOOT,
		[["	holdBarrier();\n	useReactivePort(port);", "	useReactivePort(port);"]],
	],
	[
		"boot installs no reactive port",
		"pair",
		[36, 76],
		BOOT,
		[["	holdBarrier();\n	useReactivePort(port);", "	holdBarrier();"]],
	],
	[
		"committed boot keeps the barrier",
		"pair",
		[36, 76],
		BOOT,
		[["	side.booted();\n	releaseBarrier();", "	side.booted();"]],
	],
	[
		"failed boot keeps the barrier",
		"pair",
		[36, 76],
		BOOT,
		[["		releaseBarrier();\n		error(`Crucible boot failed", "		error(`Crucible boot failed"]],
	],
	[
		"barrier released before booted",
		"swap",
		[76],
		BOOT,
		[["	side.booted();\n	releaseBarrier();", "	releaseBarrier();\n	side.booted();"]],
	],
	[
		"barrier released before the rollback",
		"pair",
		[36, 76],
		BOOT,
		[
			['		stage = "rollback";\n', '		stage = "rollback";\n		releaseBarrier();\n'],
			["		owned.clear();\n		releaseBarrier();\n", "		owned.clear();\n"],
		],
	],
	[
		"computed frame never popped",
		"pair",
		[7, 13],
		GRAPH,
		[
			[
				"	const [ok, result] = xpcall(node.read, capture);\n	pop(frame);",
				"	const [ok, result] = xpcall(node.read, capture);",
			],
		],
	],
	["cycle mark never reset", "pair", [8], GRAPH, [["		node.cycle = undefined;\n", ""]]],
	[
		"attach stops at the first computed",
		"guard",
		[10],
		GRAPH,
		[
			[
				"			if (first && source.kind === NodeKind.Computed) pending.push(source);",
				"			if ((false as boolean) && source.kind === NodeKind.Computed) pending.push(source);",
			],
		],
	],
	[
		"stale edges never unlinked",
		"pair",
		[14],
		GRAPH,
		[["	for (const edge of previous) if (!keep.has(edge.source)) unlink(observer, edge.source);\n", ""]],
	],
	[
		"root scope adopted",
		"guard",
		[23],
		OWNERSHIP,
		[["	if (parent !== undefined) adopt(parent, scope);", "	adopt(parent!, scope);"]],
	],
	["scope adoption arguments swapped", "swap", [23], OWNERSHIP, [["adopt(parent, scope);", "adopt(scope, parent);"]]],
	[
		"first run ends a missing previous run",
		"guard",
		[26],
		FLUSH,
		[["	if (previous !== undefined) endLifetime(previous);", "	endLifetime(previous!);"]],
	],
	[
		"every cleanup reported as failed",
		"guard",
		[28],
		OWNERSHIP,
		[
			[
				"		if (completion.result !== CallResult.Ok)\n			runtimeDiagnostic(",
				"		runtimeDiagnostic(",
			],
		],
	],
	[
		"nested evaluation in its own boundary",
		"guard",
		[12, 13, 32],
		GRAPH,
		[
			[
				"	if (inBody()) evaluateInline(evaluation);",
				"	if (inBody() && (false as boolean)) evaluateInline(evaluation);",
			],
		],
	],
	[
		"busiest origins listed least first",
		"swap",
		[34],
		FLUSH,
		[["busiest.sort((a, b) => a.runs > b.runs);", "busiest.sort((a, b) => b.runs > a.runs);"]],
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
	for (const name of ["tsconfig.rbxts.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	fs.cpSync(path.join(repo, ".lune"), path.join(scratch, ".lune"), { recursive: true });
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
	const build = () =>
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
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
