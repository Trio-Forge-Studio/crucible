import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const src = "packages/crucible/src/";
const V = src + "internal/sync/values.ts";
const D = src + "internal/sync/declarations.ts";
const F = src + "internal/sync/frame.ts";
const SS = src + "internal/sync/server-sync.ts";
const CS = src + "internal/sync/client-sync.ts";
const G = src + "internal/reactive/graph.ts";
const CAT = src + "internal/network/catalog.ts";
const B = src + "internal/boot.ts";
const R = src + "replicated.ts";
const EM = src + "emit/replicated.ts";
const CI = src + "client/index.ts";
const specs = ["sync-frames", "sync-players", "sync-client", "sync-budgets", "sync-publish"];
const types = ["sync-types"];

// Ruling K3: each rule carries a guard (or reset), a swap and a pair mutant, or says why the code has no such site.
// Fixture values differ wherever two same-typed values could swap, so equal values cannot hide a swap.
const rules = [
	[
		"40",
		["guard", R, "\terror(UNTRANSFORMED);", "\treturn undefined as never;"],
		["swap", "each macro throws one constant and takes no second argument of its type"],
		["pair", "a macro is a single throw with nothing paired to it"],
	],
	[
		"41",
		["guard", D, "\tif (byName.has(name)) error(`Crucible replicated name ${name} is declared twice`, 0);\n", "\n"],
		["guard sealed", D, "\tif (sealed)\n\t\terror(", "\tif (false as boolean)\n\t\terror("],
		[
			"swap",
			D,
			"\tif (byName.size() >= MAX_REPLICATED_DECLARATIONS)",
			"\tif (MAX_REPLICATED_DECLARATIONS >= byName.size())",
		],
		["pair", B, '\t\tif (side.name === "server") sealDeclarations();\n', "\n"],
		["pair client seal", B, '\t\t\tif (side.name === "client") sealDeclarations();\n', "\n"],
	],
	[
		"43",
		[
			"guard",
			D,
			"() => (onServer() ? perPlayer(declaration) : readState(declaration.node))",
			"() => readState(declaration.node)",
		],
		["swap", "a read takes no arguments and reads one node"],
		["pair", "a read is a single tracked read with nothing paired to it"],
	],
	[
		"44",
		[
			"guard",
			R,
			'readonly [replicatedKind]: { readonly audience: "all"; readonly value: (value: T) => T };',
			'readonly [replicatedKind]: { readonly audience: "all"; readonly value: (value: T) => T };\n\tset(value: T): void;',
			types,
		],
		["swap", "read-only is the absence of a setter type, with no arguments to swap"],
		["pair", "read-only is a type with no runtime operation to pair"],
	],
	[
		"45",
		["guard", CS, '\t\t\t\tout.push([declaration.status, "ready"]);\n', "\n"],
		[
			"swap",
			CS,
			'\t\t\t\tout.push([declaration.status, "ready"]);',
			'\t\t\t\tout.push([declaration.status, "failed"]);',
		],
		["pair", D, '\twrite(declaration.status, "ready");\n\treturn setmetatable', "\treturn setmetatable"],
	],
	[
		"47",
		[
			"guard",
			D,
			"\tif (owner === undefined || owner.record.kind !== Kind.Subsystem)",
			"\tif (owner === undefined)",
		],
		["swap", "placement checks one claim with no second argument of its type"],
		["pair", "placement is one check with nothing paired to it"],
	],
	[
		"48",
		["guard", D, "\tif (previous !== undefined)\n\t\terror(", "\tif (false as boolean)\n\t\terror("],
		["swap", D, "is published by ${previous.label} and ${label}", "is published by ${label} and ${previous.label}"],
		["pair", "one publisher is a single check with nothing paired to it"],
	],
	[
		"49",
		[
			"guard",
			V,
			"\t\t\tif (field === undefined) {\n\t\t\t\tcopy = true;\n\t\t\t\tstopped = true;\n\t\t\t\tbreak;\n\t\t\t}",
			"\t\t\tif (field === undefined) continue;",
		],
		[
			"guard finite",
			V,
			'\tif (value !== value || math.abs(value) === math.huge) a.reject(path, "expected finite number");\n',
			"\n",
		],
		[
			"guard admission visits",
			V,
			'\t\tif (this.steps >= SYNC_ADMISSION_VISITS) this.reject(path, "over SYNC_ADMISSION_VISITS");\n',
			"\n",
		],
		[
			"guard entries",
			V,
			'\t\tif (this.entries > SYNC_FRAME_COLLECTION_ENTRIES) this.reject(path, "over SYNC_FRAME_COLLECTION_ENTRIES");\n',
			"\n",
		],
		["guard metatable", V, '\t\tif (getmetatable(value) !== undefined) this.reject(path, "metatable");\n', "\n"],
		["guard cycle", V, '\t\tif (this.active.has(value)) this.reject(path, "cycle");\n', "\n"],
		["guard density", V, '\t\tif (seen !== count) a.reject(path, "sparse array");\n', "\n"],
		["guard map key", V, '\t\t\t\ta.reject(path, "invalid map key");', "\t\t\t\tcount += 0;"],
		["guard freeze", V, "\t\tif (!table.isfrozen(entry.value)) table.freeze(entry.value);\n", "\n"],
		["swap", V, "\ta.charge(stringBytes(name), 1, 1, path);", "\ta.charge(1, stringBytes(name), 1, path);"],
		[
			"pair",
			V,
			"\t\t\tif (!rawequal(result, child)) copy = true;\n\t\t\tadmitted.set(key as string, result);",
			"\t\t\tadmitted.set(key as string, result);",
		],
	],
	[
		"50",
		["guard", D, '\tif (declaration.audience === "player") return table.freeze({});\n', "\n"],
		["swap", "a projection takes ServerSync's one peer with no second Player to swap"],
		["pair", SS, "\t\tthis.order.push(view);\n", "\n"],
	],
	[
		"51",
		["guard", SS, "\t\t\tif (view.reported.has(declaration)) continue;\n", "\n"],
		["swap", "a failure stages one version under one declaration, with no second value of its type"],
		["pair", SS, "\t\t\t\t\tstaged.recovered.push(declaration);\n", "\n"],
	],
	[
		"52",
		[
			"guard",
			SS,
			"\t\t\t\tif (failedAt === undefined) {\n\t\t\t\t\tstaged.failed.push(declaration.name);\n\t\t\t\t\tstaged.failing.set(declaration, RETIRED);",
			"\t\t\t\tif (failedAt === undefined) {\n\t\t\t\t\tstaged.failing.set(declaration, RETIRED);",
		],
		["swap", "a retirement latches one publication with no arguments to swap"],
		["pair", D, "\towner.subscriptions.push(() => retire(declaration, publication));\n", "\n"],
	],
	[
		"53",
		["guard", SS, "\t\t\tif (publication === undefined) continue;\n\t\t\tthis.examining", "\t\t\tthis.examining"],
		["swap", "an unpublished declaration is skipped with nothing to swap"],
		["pair", "skipping is one check with nothing paired to it"],
	],
	[
		"54",
		["guard", D, "\t\tif (!publication.live) ended(declaration, publication);\n", "\n"],
		[
			"swap",
			D,
			"error(`${declaration.name}'s publisher ${publication.label} has ended`, 0);",
			"error(`${publication.label}'s publisher ${declaration.name} has ended`, 0);",
		],
		["pair", "the refusal is one check with nothing paired to it"],
	],
	[
		"55",
		[
			"guard",
			CAT,
			'\tassert(segments[0] !== "Crucible" || FRAMEWORK_MESSAGES.has(name), "reserved message name");',
			'\tassert(true as boolean, "reserved message name");',
			["network-catalog"],
		],
		["swap", F, '["failed", n.array(n.string)],', '["failed", n.array(n.u32)],'],
		["pair", CI, "\tinstallClientSync((revision) => active().send(SyncFaultMessage, revision));\n", "\n"],
	],
	[
		"56",
		[
			"guard",
			SS,
			'\treadonly ready = claim(this, "ready", ClientReady, (player: Player) => this.snapshot(player));',
			"\treadonly ready = undefined;",
		],
		["swap", "ServerSync defines one record with no arguments of one type to swap"],
		["pair", "the framework Subsystem is one definition with nothing paired to it"],
	],
	[
		"57",
		[
			"guard",
			SS,
			"\t\tif (!declarations().some((declaration) => declaration.publication !== undefined)) return undefined;\n",
			"\n",
		],
		[
			"swap",
			SS,
			'\t\tif (staged !== undefined) this.send(view, staged, "snapshot");',
			'\t\tif (staged !== undefined) this.send(view, staged, "update");',
		],
		["pair", SS, "\t\treturn () => this.end(view);", "\t\treturn undefined;"],
	],
	[
		"58",
		[
			"guard",
			SS,
			"\t\t\t\tif (accepted !== undefined && accepted.version === version) continue;\n\t\t\t} else {",
			"\t\t\t} else {",
		],
		["swap", SS, "\t\t\t\tthis.commit(view, staged, false);", "\t\t\t\tthis.commit(view, staged, true);"],
		["pair", SS, "\t\tthis.commit(view, staged, true);\n\t\treturn cost.bytes;", "\t\treturn cost.bytes;"],
	],
	[
		"59",
		["guard", SS, "\t\tif (view.revision >= SYNC_MAX_REVISION) {", "\t\tif (false as boolean) {"],
		["swap", "a revision is one counter with no second value of its type"],
		["pair", SS, "\t\tif (sent) view.revision += 1;\n", "\n"],
	],
	[
		"60 62",
		[
			"guard",
			CS,
			"\t\tif (!ascending(keys) || !ascending(frame.failed) || !disjoint(keys, frame.failed))",
			"\t\tif (!ascending(frame.failed) || !disjoint(keys, frame.failed))",
		],
		[
			"swap",
			CS,
			"\t\t\t\tif (!rawequal(reused, declaration.node.value)) out.push([declaration.node, reused]);",
			"\t\t\t\tif (!rawequal(reused, declaration.node.value)) commitPrepared([[declaration.node, reused]]);",
		],
		["pair", CS, "\t\tthis.revision = frame.revision;\n", "\n"],
		["guard commit equal", G, "\t\tif (rawequal(value, node.value)) continue;\n", "\n"],
	],
	[
		"61",
		[
			"guard",
			SS,
			"\t\t\tif (accepted !== undefined && equal(accepted.value, value, work))",
			"\t\t\tif (false as boolean)",
		],
		["swap", "equality is symmetric: swapping its arguments changes only which side is iterated"],
		[
			"pair",
			V,
			"\t\t\tfor (let i = 0; i < ys.size(); i++) {\n\t\t\t\tvisit();\n",
			"\t\t\tfor (let i = 0; i < ys.size(); i++) {\n",
		],
	],
	[
		"63",
		["guard", V, "\t\tif (seen === count) return old;\n", "\n"],
		["swap", V, "\t\tconst result = reuse(before, child);", "\t\tconst result = reuse(child, before);"],
		["pair", V, "\treturn table.freeze(fresh);\n}", "\treturn fresh;\n}"],
	],
	[
		"64",
		["guard", CS, "\t\t\t\tif (declaration === undefined) continue;\n", "\n"],
		["swap", "an unknown key is skipped with nothing to swap"],
		["pair", "skipping is one check with nothing paired to it"],
	],
	[
		"65",
		["guard", CS, "\t\tif (this.failed) return;\n", "\n"],
		[
			"swap",
			CS,
			"return this.latch(`the server stopped this client's sync at revision ${frame.revision}`, false);",
			"return this.latch(`the server stopped this client's sync at revision ${frame.revision}`, true);",
		],
		["pair", CS, "\t\tif (fault) pcall(sendFault, this.revision);\n", "\n"],
		["pair server kick", SS, "\t\t}\n\t\tpcall(() => networkPort().kick(player, KICK_MESSAGE));", "\t\t}"],
	],
	[
		"66",
		[
			"guard",
			SS,
			'\t\tpcall(sender, player, { failed: [], kind: "failed", revision: view.revision, values: [] });\n\t\tthis.end(view);\n\t\truntimeDiagnostic(',
			"\t\tthis.end(view);\n\t\truntimeDiagnostic(",
		],
		["swap", "a failure sends one frame to one player with nothing of one type to swap"],
		[
			"pair",
			SS,
			"\t\t);\n\t\tpcall(() => networkPort().kick(player, KICK_MESSAGE));\n\t}\n\n\t/** Rule 65",
			"\t\t);\n\t}\n\n\t/** Rule 65",
		],
	],
	[
		"67",
		[
			"guard",
			SS,
			"\t\tif (!accepted) {\n\t\t\tthis.end(view);\n\t\t\treturn 0;",
			"\t\tif (!accepted) {\n\t\t\treturn 0;",
		],
		["swap", "send's result is one boolean with nothing to swap"],
		["pair", "ending the view is the whole response, with nothing paired to it"],
	],
	[
		"68",
		["guard", SS, "\t\tif (this.peers.get(view.player) === view) this.peers.delete(view.player);\n", "\n"],
		["swap", "a peer is keyed by one Player with nothing to swap"],
		["pair", "the cleanup ClientReady returns is rule 57's pair mutant"],
	],
	[
		"69",
		["guard", SS, "\t\tif (budget !== undefined) {", "\t\tif (false as boolean) {"],
		[
			"swap",
			F,
			"\t\tentries += cost.entries;\n\t\tvisits += cost.visits;",
			"\t\tentries += cost.visits;\n\t\tvisits += cost.entries;",
		],
		["pair", F, "\tbytes += varuintSize(frame.failed.size());\n", "\n"],
	],
	[
		"70",
		[
			"guard",
			SS,
			"(examined >= SYNC_HEARTBEAT_PEERS || visits >= SYNC_HEARTBEAT_VISITS)",
			"(examined >= SYNC_HEARTBEAT_PEERS)",
		],
		[
			"guard peers",
			SS,
			"(examined >= SYNC_HEARTBEAT_PEERS || visits >= SYNC_HEARTBEAT_VISITS)",
			"(visits >= SYNC_HEARTBEAT_VISITS)",
		],
		[
			"swap",
			SS,
			"(examined >= SYNC_HEARTBEAT_PEERS || visits >= SYNC_HEARTBEAT_VISITS)",
			"(examined >= SYNC_HEARTBEAT_VISITS || visits >= SYNC_HEARTBEAT_PEERS)",
		],
		["pair", SS, "\t\t\tthis.cursor = math.max(this.order.indexOf(stopped), 0);\n", "\n"],
		[
			"guard peer limit",
			SS,
			"\t\t\tconst work: Work = { visits: 0, limit: SYNC_PEER_VISITS };",
			"\t\t\tconst work: Work = { visits: 0, limit: math.huge };",
		],
		[
			"guard bytes",
			SS,
			"\t\tif (spent > 0 && spent + cost.bytes > SYNC_HEARTBEAT_BYTES) return undefined;\n",
			"\n",
		],
	],
	[
		"72",
		["guard", V, '\treturn value === undefined ? "nil" : typeOf(value);', "\treturn tostring(value);"],
		["swap", "a diagnostic names one declaration and path with nothing of one type to swap"],
		["pair", "metadata-only messages have no paired operation"],
	],
	[
		"73",
		[
			"guard",
			SS,
			'\t\t\tpcall(sender, view.player, { failed: [], kind: "failed", revision: view.revision, values: [] });\n',
			"\n",
		],
		["swap", "the end sends one frame per peer with nothing to swap"],
		["pair", SS, "\t\tstate.subscriptions.push(() => this.stop());\n", "\n"],
	],
	[
		"74",
		["guard", B, "\t\towned.clear();\n\t\trollbackDeclarations();\n", "\t\towned.clear();\n"],
		["guard unseal", D, "\tsealed = false;\n}", "}"],
		["swap", D, '\t\twrite(declaration.status, "pending");', '\t\twrite(declaration.status, "ready");'],
		["pair", D, "\t\twrite(declaration.node, declaration.initial);\n", "\n"],
	],
	[
		"78 K9",
		[
			"guard",
			EM,
			"export function replicated<T>(name: string, initial: NoInfer<T>, value: Node)",
			"export function replicated<T>(name: string, initial: unknown, value: Node)",
			types,
		],
		[
			"swap",
			EM,
			'\treturn declareReplicated("player", name, initial, value)',
			'\treturn declareReplicated("all", name, initial, value)',
		],
		["pair", "an emit form is one registration call with nothing paired to it"],
	],
	[
		"R-D1",
		["guard", D, '\tif ((segments[0] === "Crucible") !== framework)', "\tif (false as boolean)"],
		[
			"swap",
			D,
			"\treturn MAX_REPLICATED_DECLARATIONS - byName.size();",
			"\treturn byName.size() - MAX_REPLICATED_DECLARATIONS;",
		],
		["pair", "a framework declaration is one registration call with nothing paired to it"],
	],
];

const mutants = [];
const absent = [];
for (const [rule, ...operators] of rules)
	for (const [operator, file, before, after, extra = []] of operators)
		if (before === undefined) absent.push(`${rule} ${operator}: not applicable, ${file}`);
		else mutants.push([`${rule} ${operator}`, file, before, after, extra]);

const missing = mutants.filter(([, file, before]) => !fs.readFileSync(path.join(repo, file), "utf8").includes(before));
assert.deepEqual(
	missing.map(([name]) => name),
	[],
	"every mutant pattern exists in its file",
);
if (process.argv.includes("--check")) {
	console.log(`${mutants.length} patterns found; ${absent.length} operators not applicable`);
	process.exit(0);
}

function run(cwd, command, args) {
	const result = spawnSync(command, args, { cwd, encoding: "utf8", timeout: 600000 });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}

// A Lune run that printed no summary never ran its specs, so it decides nothing; run it again.
function specRun(cwd, extra = []) {
	for (let attempt = 1; ; attempt++) {
		const result = run(cwd, "lune", ["run", "test", ...specs, ...extra]);
		if (/\d+ passed, \d+ failed/.test(result.stdout)) return result;
		assert.ok(attempt < 3, `lune printed no summary three times\n${result.stdout}${result.stderr}`);
	}
}

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-sync-mutants-"));
try {
	for (const name of ["tsconfig.rbxts.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	for (const name of [".lune", "test"])
		fs.cpSync(path.join(repo, name), path.join(scratch, name), { recursive: true });
	fs.cpSync(path.join(repo, "packages/crucible"), path.join(scratch, "packages/crucible"), {
		recursive: true,
		filter: (p) => !p.split(path.sep).includes("node_modules"),
	});
	for (const name of ["node_modules", "packages/crucible/node_modules", "build"])
		fs.symlinkSync(path.join(repo, name), path.join(scratch, name), "junction");
	const originals = new Map(mutants.map(([, file]) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const build = () =>
		run(scratch, process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	let built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	const baseline = specRun(scratch, [...new Set(mutants.flatMap(([, , , , extra]) => extra))]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	for (const line of absent) console.log(line);
	let killed = 0;
	const from = Number(process.argv.find((arg) => /^\d+$/.test(arg)) ?? 0);
	const only = process.argv.find((arg) => arg.startsWith("--only="))?.slice("--only=".length);
	let ran = 0;
	for (const [index, [name, file, before, after, extra]] of mutants.entries()) {
		if (index < from || (only !== undefined && !only.split(",").includes(name))) continue;
		ran++;
		const original = originals.get(file);
		fs.writeFileSync(path.join(scratch, file), original.replaceAll(before, after));
		built = build();
		assert.equal(built.status, 0, `${name} must compile\n${built.stdout}${built.stderr}`);
		const tested = specRun(scratch, extra);
		assert.equal(tested.status, 1, `${name} survived\n${tested.stdout}${tested.stderr}`);
		const failure = (tested.stdout + tested.stderr).match(/FAIL ((?:sync|network)[^\n]*)/);
		assert.ok(failure, `${name} must fail a spec assertion`);
		console.log(`KILLED ${name} | ${path.basename(file)} | first failure: ${failure[1]}`);
		killed++;
		fs.writeFileSync(path.join(scratch, file), original);
	}
	built = build();
	assert.equal(built.status, 0, built.stdout + built.stderr);
	console.log(`${killed}/${ran} source mutants killed; ${absent.length} operators not applicable; restored build 0`);
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
