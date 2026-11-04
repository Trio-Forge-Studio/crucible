import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-profile-mutants-"));
const D = "packages/crucible/src/internal/data/";
const H = D + "handles.ts";
const DC = D + "declarations.ts";
const RT = D + "runtime.ts";
const PA = D + "paths.ts";
const S = D + "session.ts";
const P = D + "profiles.ts";
const E = D + "envelope.ts";
const ROOT = "packages/crucible/src/data.ts";
const SD = "packages/crucible/src/server/data.ts";
const HAN = "data-profile-handles.luau: ";
const VIEW = "data-profile-views.luau: ";
const DEC = "data-profile-declarations.luau: ";
const CL = "data-profile-claims.luau: ";
const RX = "data-profile-reactive.luau: ";
const m = (rule, kind, file, before, after, spec) => [rule, kind, file, before, after, spec];
// A pattern ignores how deep its lines are indented, so a row names the code and not the nesting around it.
const matcher = (before) =>
	new RegExp(
		before
			.split("\n")
			.map((line) => line.replace(/^\t*/, "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
			.map((line) => `\\t*${line}`)
			.join("\n"),
		"g",
	);
const occurrences = (text, before) => (text.match(matcher(before)) ?? []).length;
const apply = (text, before, after) => text.replace(matcher(before), () => after);
// K3: every rule with a site has a guard, a swap and a pair row below. A rule or operator with no site is listed here.
const K3 = [
	"rules 5, 7 and 112: one declaration per schema and the persist lowering are the transformer's (T2's campaign)",
	"rule 8: the types are checked by test/profile-types.test.mjs and the K9 consumer build, which compile TypeScript, not the Lune specs this campaign runs",
	"rules 15 and 16: frozen admission is D3's walk and writes never wait is D1's gate; the handles add no site of their own",
	"rule 1 swap: each untransformed macro throws one fixed message, with no operands to exchange",
	"rule 3 swap: the seal test is one boolean read",
	"rule 4 pair and rule 129 swap: unwrap records and unwraps in one step; its swap is checkMarks' argument row",
	"rule 6 swap: the claim callbacks' argument orders are exchanged in the rule 18 swap row",
	"rule 6 guard on releasing a retired claim's receiver name: only a second boot in one realm, after a rollback, claims the name again, and the profile world boots each realm once",
	"rule 9 swap and pair: drop() writes undefined into a node the seal already cleared, so its write is never observable; the slot map delete is its pair row",
	"rule 9 guard on `writable` in `loaded`: the seal timer clears the slot once `safeUntil` passes, and the scripted clock cannot move inside a data callback, which cannot yield; only a callback that runs past `safeUntil` on the CPU and reads `loaded` inside itself would see the difference",
	"rule 10 swap: refreshPlayers takes no operands",
	"rule 11: await is D1's (data campaign rule 11); the handles add no site of their own",
	"rule 17 swap and guard: the join loop has one operand and no guard",
	"rule 104 guard on the generation check: core closes a rolled-back boot's suspended start, so the check is a backstop no spec reaches",
	"rule 104 swap: activation's steps take no operands of one type",
	"rule 107 swap: retire takes no operands",
	"rule 110 swap: a diagnostic key is only compared for equality",
	"rule 115 swap: root(player) has one operand",
	"rule 116 union of several types: a union is a leaf, so keeping its last branch's children changes only how strongly a handle below it is cached, which no contracted read or write observes",
	"rule 120: a map entry's and an optional field's set(undefined) are setPath's rows (rule 118)",
	"rule 122 swap: the metatable functions take the handle and the key, never two of one type",
	"rule 124 swap: the client root is one cached value per declaration",
	"rule 125 swap: each projection reads one player and one field",
	"rule 127 visits measure: every admitted entry adds at most three visits (its key, its value and its frame wrapper), and each visits share is at least four times the entries share, so the entries measure always refuses first",
	"rule 134: the count and name checks are rule 2's and rule 4's rows",
];
// [rule, kind, file, before, after, spec]. Kinds: guard deletes a reset or guard, swap exchanges two values of one
// type, pair short-circuits one half of a paired operation.
const mutants = [
	m(
		"1",
		"guard",
		ROOT,
		'error("Crucible data needs the @trioforge/crucible/transformer plugin");',
		"return undefined as never;",
		DEC + "1 untransformed",
	),
	m(
		"1",
		"pair",
		SD,
		'export function profileMessage<P>(_name: string): ProfileMessage<P> {\n\terror("Crucible data needs the @trioforge/crucible/transformer plugin");',
		"export function profileMessage<P>(_name: string): ProfileMessage<P> {\n\treturn undefined as never;",
		DEC + "1 untransformed",
	),
	m("2", "guard", DC, "if (view.size() > VIEW_NAME_MAX_BYTES)", "if (false as boolean)", DEC + "2 134 A10"),
	m(
		"2",
		"swap",
		DC,
		"if (view.size() > VIEW_NAME_MAX_BYTES)",
		"if (VIEW_NAME_MAX_BYTES > view.size())",
		DEC + "2 134 A10",
	),
	m("2", "pair", DC, "if (storeNameTaken(name)) error(`${name} is already declared`, 3);", "", DEC + "2 3 names"),
	m("3", "guard", DC, "if (dataRegistrySealed())", "if (false as boolean)", DEC + "3 a declaration after"),
	m(
		"3",
		"guard",
		DC,
		"if (profiles.size() >= MAX_PROFILE_STORES)",
		"if (profiles.size() > MAX_PROFILE_STORES)",
		DEC + "2 3 names",
	),
	m(
		"3",
		"pair",
		RT,
		"\tif (!server.activated) {\n\t\tsealDeclarations(false);",
		"\tif (!server.activated) {",
		DEC + "3 a declaration after",
	),
	m("4", "guard", DC, "checkMarks(name, found, serverOnlyPaths);", "", DEC + "4 129 H1 E22"),
	m(
		"4",
		"swap",
		DC,
		"checkMarks(name, found, serverOnlyPaths);",
		"checkMarks(name, serverOnlyPaths as string[][], found);",
		DEC + "4 129 H1 E22",
	),
	m(
		"4",
		"pair",
		DC,
		"return unwrap(value.value, path, found);",
		"return value;",
		DEC + "4 129 a declaration touches",
	),
	m(
		"4",
		"pair",
		DC,
		"admitReplicated(compilePlan(replica), name, schema.defaults) as object;",
		"schema.defaults;",
		DEC + "4 129 a declaration touches",
	),
	m("134", "guard", DC, "if (room < needed)", "if (false as boolean)", VIEW + "134 a store whose declarations"),
	m("134", "swap", DC, "if (room < needed)", "if (needed < room)", VIEW + "134 a store whose declarations"),
	m(
		"6",
		"guard",
		RT,
		"if (owner === undefined || owner.record.kind !== Kind.Subsystem)",
		"if (owner === undefined)",
		CL + "6 a claim outside",
	),
	m(
		"6",
		"guard",
		RT,
		"if (server.activated) error(",
		"if (false as boolean) error(",
		CL + "6 a retired onLoad claim",
	),
	m(
		"6",
		"guard",
		RT,
		"const other = server.receivers.get(claim.message.name);\n\t\t\tif (other !== undefined)",
		"const other = server.receivers.get(claim.message.name);\n\t\t\tif (false as boolean)",
		CL + "6 a second receiver",
	),
	m(
		"6",
		"guard",
		RT,
		"if (claim.kind !== kind || !claim.live) continue;",
		"if (claim.kind !== kind) continue;",
		CL + "6 a retired onLoad claim",
	),
	m("6", "pair", RT, "this.runtime?.inbox.unreceive(claim.message.name);", "", CL + "6 107 a retired claim"),
	m("6", "pair", RT, "engine?.receipts.unroute(claim.productId);", "", CL + "6 107 a retired claim"),
	m(
		"9",
		"guard",
		RT,
		"if (slot === undefined || readComputed(slot.live) !== true) return false;",
		"if (slot === undefined) return false;",
		RX + "9 loaded(player)",
	),
	m(
		"9",
		"pair",
		RT,
		"\t\twrite(slot.node, undefined);\n\t\tthis.slots.delete(player);",
		"\t\twrite(slot.node, undefined);",
		RX + "9 43 a departed",
	),
	m(
		"10",
		"guard",
		RT,
		"if (before !== (value !== undefined)) this.refreshPlayers();",
		"this.refreshPlayers();",
		RX + "10 players()",
	),
	m("10", "pair", RT, "if (before !== (value !== undefined)) this.refreshPlayers();", "", RX + "10 players()"),
	m(
		"12",
		"swap",
		RT,
		'\t\tsession?.assertIdle();\n\t\tassertOutsideCallback();\n\t\tif (insideDataCallback()) error("a write inside a data callback; return the data instead", 0);',
		'\t\tif (insideDataCallback()) error("a write inside a data callback; return the data instead", 0);\n\t\tsession?.assertIdle();\n\t\tassertOutsideCallback();',
		HAN + "E15",
	),
	m("12", "pair", RT, '\t\tguardPure("write state");\n\t}', "\t}", HAN + "123 RX 7"),
	m(
		"13",
		"guard",
		S,
		"update(transaction, (current) => {\n\t\t\t\tresult = work(x, y);\n\t\t\t\treturn current;\n\t\t\t});",
		"result = work(x, y);",
		CL + "13 A4 K7",
	),
	m("13", "pair", S, "\tdepth -= 1;\n", "", HAN + "E3 E4 12"),
	m("13", "pair", S, "\tdepth += 1;\n", "", HAN + "123 a handle write to another player"),
	m(
		"13",
		"guard",
		RT,
		'\t\t\t\toutsideCallbacks();\n\t\t\t\tif (!activated(bootClock() + CALL_DEADLINE_SECONDS)) return storeResult({ kind: "timeout" });\n\t\t\t\treturn storeResult(binding.runtime!.inbox.send(',
		'\t\t\t\tif (!activated(bootClock() + CALL_DEADLINE_SECONDS)) return storeResult({ kind: "timeout" });\n\t\t\t\treturn storeResult(binding.runtime!.inbox.send(',
		CL + "13 send, clearQuarantined",
	),
	m("14", "guard", S, "this.schema.check?.(admitted, this.value);", "", VIEW + "127 14 E24"),
	m(
		"14",
		"swap",
		S,
		"this.schema.check?.(admitted, this.value);",
		"this.schema.check?.(this.value, admitted);",
		VIEW + "127 14 E24",
	),
	m(
		"17",
		"pair",
		RT,
		'for (const binding of live) engine.port.spawn(() => binding.runtime!.load(who, "join"));',
		'for (const binding of [live[0]]) engine.port.spawn(() => binding.runtime!.load(who, "join"));',
		CL + "17 104 one load",
	),
	m("18", "guard", RT, "\t\tif (!fits) {", "\t\tif (false as boolean) {", VIEW + "127 a load over a view bound"),
	m(
		"18",
		"pair",
		RT,
		"if (!rawequal(result, current)) session.commit(session.prepare(result));",
		"session.prepare(result);",
		CL + "6 18 onLoad claims run",
	),
	m(
		"18",
		"swap",
		RT,
		"(claim, data) => claim.callback(session.player, data, reason), true, reason);",
		"(claim, data) => claim.callback(data, session.player, reason), true, reason);",
		CL + "6 18 onLoad claims run",
	),
	m(
		"43",
		"swap",
		RT,
		"(claim, data) => claim.callback(session.player, data, reason), true, reason);\n\t\treturn session.value;",
		"(claim, data) => claim.callback(session.player, data, reason), false, reason);\n\t\treturn session.value;",
		HAN + "E23 123 a read of the player's own handles inside onLoad",
	),
	m(
		"43",
		"pair",
		RT,
		"\t\t\tif (fatal) {\n\t\t\t\tthis.claiming.delete(session.player);\n\t\t\t\terror(err, 0);\n\t\t\t}",
		"\t\t\tif (fatal) {\n\t\t\t\tthis.claiming.delete(session.player);\n\t\t\t}",
		HAN + "E23 123 a read of the player's own handles inside onLoad",
	),
	m("104", "pair", RT, "\tserver.activated = true;\n", "", CL + "65 66 67 73 115"),
	m(
		"104",
		"guard",
		RT,
		"if (granted) engine.receipts.install(engine.port.marketplace());",
		"engine.receipts.install(engine.port.marketplace());",
		CL + "57 104 without a grant",
	),
	m(
		"104",
		"pair",
		RT,
		"server.disconnect.push(players.onAdded(join), players.onRemoving(leave));",
		"server.disconnect.push(players.onAdded(join));",
		RX + "9 43 a departed",
	),
	m("107", "guard", RT, "\tif (!server.activated) {", "\tif (false as boolean) {", DEC + "3 a declaration after"),
	m("107", "pair", RT, "for (const disconnect of server.disconnect) disconnect();", "", CL + "104 107 a rolled-back"),
	m("107", "pair", RT, "\tengine?.retire();\n", "", CL + "104 107 a rolled-back"),
	m(
		"110",
		"guard",
		RT,
		"\t\t\tserver.engine!.diagnose(\n\t\t\t\t`${kind}:${name}:${session.token}:${claim.owner}`,",
		"\t\t\t((...args: unknown[]) => args)(\n\t\t\t\t`${kind}:${name}:${session.token}:${claim.owner}`,",
		HAN + "E23 123 a read of the player's own handles inside onLoad",
	),
	m(
		"110",
		"swap",
		RT,
		"\t\t\t\tfatal\n\t\t\t\t\t?",
		"\t\t\t\t!fatal\n\t\t\t\t\t?",
		HAN + "E23 123 a read inside onRelease",
	),
	m(
		"110",
		"pair",
		RT,
		"\t\t\tserver.engine!.diagnose(\n\t\t\t\t`views:${name}:${session.token}`,",
		"\t\t\t((...args: unknown[]) => args)(\n\t\t\t\t`views:${name}:${session.token}`,",
		VIEW + "127 a load over a view bound",
	),
	m(
		"115",
		"guard",
		RT,
		"if (slot === undefined && port().players.present(player)) {",
		"if (slot === undefined) {",
		HAN + "115 the root is cached",
	),
	m(
		"115",
		"pair",
		DC,
		"if (player === undefined) error(`${name} is per player on the server; call ${name}(player)`, 2);",
		"",
		HAN + "115 the root is cached",
	),
	m("116", "guard", H, "RESERVED.has(key) && !node.source.client", "RESERVED.has(key)", VIEW + "124 134 E17"),
	m(
		"116",
		"swap",
		PA,
		'return typeIs(segment, "number") ? segment + 1 : segment;',
		'return typeIs(segment, "number") ? segment : segment;',
		HAN + "E5 ",
	),
	m(
		"116",
		"pair",
		H,
		"RESERVED.has(key) && !node.source.client",
		"(false as boolean) && RESERVED.has(key)",
		HAN + "E1 ",
	),
	m(
		"117",
		"guard",
		RT,
		"return getPath(readState(this.node) ?? this.binding.declaration.schema.defaults, path);",
		"return getPath(readState(this.node), path);",
		HAN + "E13 ",
	),
	m(
		"117",
		"pair",
		DC,
		"if (live && field !== undefined) return getPath(",
		"if (field !== undefined) return getPath(",
		VIEW + "126 loaded() is false",
	),
	m(
		"118",
		"guard",
		H,
		"error(`${describePath(name, path)}: its parent does not exist; set the parent whole`, 0);",
		"return parent as object;",
		HAN + "E14 ",
	),
	m(
		"118",
		"swap",
		H,
		"if (last % 1 !== 0 || last < 0 || last > highest)",
		"if (last % 1 !== 0 || last < 0 || highest > last)",
		HAN + "E5 ",
	),
	m("118", "pair", PA, "if (rawequal(replaced, child)) return root;", "", HAN + "E3 E4 12"),
	m("118", "pair", PA, "const copy = table.clone(current);", "const copy = current;", HAN + "E8 "),
	m(
		"119",
		"guard",
		H,
		"check(node, root, [...node.path, position], 1);",
		"check(node, root, [...node.path, position], 0);",
		HAN + "E5 ",
	),
	m("119", "pair", H, "copy.remove(index as number);", "", HAN + "E5 "),
	m("119", "swap", H, "copy.insert(position, value as defined);", "copy.insert(0, value as defined);", HAN + "E5 "),
	m("121", "guard", H, "if (found !== undefined) return found;", "", HAN + "E6 "),
	m(
		"121",
		"swap",
		H,
		"const cache = strong ? node.fields : node.entries;",
		"const cache = strong ? node.entries : node.fields;",
		HAN + "E20 ",
	),
	m("121", "pair", H, "cache.set(key, made);", "", HAN + "E6 "),
	m(
		"122",
		"guard",
		H,
		'const handleNewindex = () => error("a handle has no fields to assign; write with set or update", 2);',
		"const handleNewindex = () => {};",
		HAN + "E19 ",
	),
	m("122", "pair", H, "__index: () => mapKey(name),", "__index: () => undefined,", HAN + "E12 "),
	m(
		"123",
		"guard",
		RT,
		"\t\tif (claim !== undefined)\n\t\t\terror(",
		"\t\tif (false as boolean)\n\t\t\terror(",
		HAN + "E23 123 a read of the player's own handles inside onLoad",
	),
	m(
		"123",
		"pair",
		RT,
		"\t\t}\n\t\tthis.claiming.delete(session.player);\n\t}",
		"\t\t}\n\t}",
		HAN + "E23 123 a read from a thread",
	),
	m(
		"123",
		"pair",
		H,
		"\tsource.beforeRead();\n",
		"",
		HAN + "E23 123 a read of the player's own handles inside onLoad",
	),
	m(
		"124",
		"guard",
		DC,
		"if (args.size() > 0) error(`a client reads only its own profile; call ${name}()`, 2);",
		"",
		VIEW + "124 134 E17",
	),
	m("124", "pair", DC, "clientRoots.set(declaration, root);", "", VIEW + "124 134 E17"),
	m(
		"125",
		"guard",
		RT,
		"return slot !== undefined && readComputed(slot.live) === true;",
		"return slot !== undefined;",
		VIEW + "125 129 K1",
	),
	m(
		"125",
		"pair",
		RT,
		"return value === undefined ? undefined : (value as Record<string, unknown>)[field.name];",
		"return undefined;",
		VIEW + "125 a currency write",
	),
	m(
		"126",
		"guard",
		DC,
		'for (const field of declaration.fields) if (readState(field.declaration.status) !== "ready") return false;',
		"",
		VIEW + "126 M3 one failed field",
	),
	m(
		"126",
		"swap",
		DC,
		'if (readState(live.status) !== "ready" || readState(live.node) !== true) return false;',
		'if (readState(live.status) !== "ready" || readState(live.node) !== false) return false;',
		VIEW + "126 loaded() is false",
	),
	m(
		"127",
		"guard",
		RT,
		'if (bytes > share.bytes) over("bytes", bytes, share.bytes);',
		"",
		VIEW + "127 a write over the store's share",
	),
	m(
		"127",
		"swap",
		RT,
		'if (bytes > share.bytes) over("bytes", bytes, share.bytes);',
		'if (share.bytes > bytes) over("bytes", bytes, share.bytes);',
		VIEW + "127 a write over the store's share",
	),
	m(
		"127",
		"pair",
		RT,
		"binding.checkViews(declaration.schema.defaults, undefined);",
		"",
		VIEW + "127 with two profile stores",
	),
	m(
		"127",
		"pair",
		RT,
		"bytes: math.floor(OWNER_VIEWS_MAX_BYTES / count),",
		"bytes: OWNER_VIEWS_MAX_BYTES,",
		VIEW + "127 with two profile stores",
	),
	m(
		"131",
		"guard",
		PA,
		'return key === "[]" || key === "*" || key.sub(1, 1) === "\\\\" ? `\\\\${key}` : key;',
		"return key;",
		DEC + "131 replicatedPaths",
	),
	m(
		"131",
		"swap",
		PA,
		"if (a[i] !== b[i]) return bytesLess(a[i], b[i]);\n\treturn a.size() < b.size();",
		"if (a[i] !== b[i]) return bytesLess(a[i], b[i]);\n\treturn b.size() < a.size();",
		DEC + "131 replicatedPaths",
	),
	m(
		"131",
		"pair",
		PA,
		"\t\t\t\t\tadd(path);\n\t\t\t\t\twalk(child, path);",
		"\t\t\t\t\twalk(child, path);",
		DEC + "131 replicatedPaths",
	),
	m(
		"65",
		"swap",
		RT,
		'return result.kind === "ok" ? { ok: true, value: result.value } : { ok: false, reason: result.kind };',
		'return result.kind !== "ok" ? { ok: true, value: result.value } : { ok: false, reason: result.kind };',
		CL + "65 66 67 73 115",
	),
	m(
		"37",
		"swap",
		RT,
		'return result === "ok" ? { ok: true } : { ok: false, reason: result };',
		'return result !== "ok" ? { ok: true } : { ok: false, reason: result };',
		CL + "65 66 67 73 115",
	),
	// A18: D1's two surviving mutants and D3's four header echoes.
	m(
		"26",
		"guard",
		P,
		"dispatchDeadline: release ? session.safeUntil - LOCAL_AUTHORITY_SECONDS + LEASE_SECONDS : session.safeUntil,",
		"dispatchDeadline: release ? session.safeUntil - LOCAL_AUTHORITY_SECONDS + LEASE_SECONDS : session.safeUntil + 0.1,",
		"data-sessions.luau: rule 26 K15 A18",
	),
	m(
		"51",
		"swap",
		P,
		"\t\tthis.requestWrite(session, Priority.Release);\n\t}",
		"\t\tthis.requestWrite(session, Priority.Renew);\n\t}",
		"data-shutdown.luau: rule 51 A18",
	),
	m(
		"110",
		"guard",
		E,
		'if (!typeIs(raw, "table")) fail("not a table");',
		'if (!typeIs(raw, "table")) fail(`not a table ${tostring(raw)}`);',
		"data-migrations.luau: rule 110 A18",
	),
	m(
		"110",
		"guard",
		E,
		'if (payload !== undefined && !typeIs(payload, "table")) fail("bad payload");',
		'if (payload !== undefined && !typeIs(payload, "table")) fail(`bad payload ${tostring(payload)}`);',
		"data-migrations.luau: rule 110 A18",
	),
	m(
		"110",
		"guard",
		E,
		'if (!typeIs(value, "number") || value < 0 || value % 1 !== 0) fail(`bad ${name}`);',
		'if (!typeIs(value, "number") || value < 0 || value % 1 !== 0) fail(`bad ${name} ${tostring(value)}`);',
		"data-migrations.luau: rule 110 A18",
	),
	m(
		"110",
		"guard",
		E,
		'if (payload !== undefined && !typeIs(payload, "table")) fail("bad payload");',
		'if (payload !== undefined && !typeIs(payload, "table")) fail(typeIs(payload, "string") ? `bad payload ${payload}` : "bad payload");',
		"data-migrations.luau: rule 110 A18",
	),
	m(
		"13",
		"guard",
		RT,
		'const target = player(name, "handoff", who);\nbinding.guard(binding.runtime?.sessions.get(target));',
		'const target = player(name, "handoff", who);',
		HAN + "13 a data handoff inside a computed",
	),
	m(
		"126",
		"guard",
		DC,
		"return getPath(declaration.clientTemplate, path);",
		"return getPath(declaration.schema.defaults, path);",
		VIEW + "125 129 a public object's serverOnly child",
	),
	m(
		"124",
		"guard",
		DC,
		"if (!live) return declaration.clientTemplate;\nconst root: Record<string, unknown> = {};",
		"const root: Record<string, unknown> = {};",
		VIEW + "125 129 a public object's serverOnly child",
	),
	m(
		"127",
		"swap",
		RT,
		'if (entries > share.entries) over("entries", entries, share.entries);',
		'if (entries > share.visits) over("entries", entries, share.entries);',
		VIEW + "127 a write within the store's bytes but over its entries share",
	),
	m(
		"121",
		"guard",
		H,
		'const strong = schema !== undefined && tableBranch(node.schema)?.kind === "object";',
		"const strong = false;",
		HAN + "121 an object-field child nothing else holds",
	),
	m(
		"12",
		"guard",
		RT,
		"if (runtime === undefined) return false;",
		"if (runtime === undefined) return true;",
		HAN + "12 117 a write before DataRuntime constructs",
	),
	m(
		"4",
		"guard",
		PA,
		"if (a.size() !== b.size()) return false;",
		"",
		DEC + "4 129 H1 E22 a template whose marks disagree",
	),
	m(
		"131",
		"guard",
		PA,
		"for (const segment of path) identity += `${segment.size()}:${segment}`;",
		"for (const segment of path) identity += segment;",
		DEC + "131 replicatedPaths",
	),
];
// Type-checks every mutant in one language service, so a mutant that cannot compile fails before the slow runs.
function typecheck(originals) {
	const ts = require(require.resolve("typescript", { paths: [path.join(scratch, "packages/crucible")] }));
	const configPath = path.join(scratch, "packages/crucible/tsconfig.json");
	const config = ts.parseJsonConfigFileContent(
		ts.readConfigFile(configPath, ts.sys.readFile).config,
		ts.sys,
		path.dirname(configPath),
	);
	const texts = new Map();
	const versions = new Map();
	const service = ts.createLanguageService({
		getScriptFileNames: () => config.fileNames,
		getCompilationSettings: () => config.options,
		getScriptVersion: (file) => String(versions.get(path.resolve(file)) ?? 0),
		getScriptSnapshot: (file) => {
			const text =
				texts.get(path.resolve(file)) ?? (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : undefined);
			return text === undefined ? undefined : ts.ScriptSnapshot.fromString(text);
		},
		getCurrentDirectory: () => path.dirname(configPath),
		getDefaultLibFileName: (options) => ts.getDefaultLibFilePath(options),
		fileExists: ts.sys.fileExists,
		readFile: ts.sys.readFile,
		readDirectory: ts.sys.readDirectory,
		directoryExists: ts.sys.directoryExists,
		getDirectories: ts.sys.getDirectories,
	});
	const errors = (file) =>
		[...service.getSyntacticDiagnostics(file), ...service.getSemanticDiagnostics(file)].map((d) =>
			ts.flattenDiagnosticMessageText(d.messageText, " "),
		);
	const baseline = new Map();
	const broken = [];
	for (const [index, [rule, kind, file, before, after]] of mutants.entries()) {
		if (!file.endsWith(".ts")) continue;
		const absolute = path.resolve(scratch, file);
		if (!baseline.has(absolute)) baseline.set(absolute, errors(absolute).length);
		texts.set(absolute, apply(originals.get(file), before, after));
		versions.set(absolute, (versions.get(absolute) ?? 0) + 1);
		const found = errors(absolute);
		texts.delete(absolute);
		versions.set(absolute, versions.get(absolute) + 1);
		if (found.length > baseline.get(absolute))
			broken.push(`${index} rule ${rule} ${kind} does not compile: ${found[0]}`);
	}
	assert.deepEqual(broken, [], "every mutant compiles");
}
function run(command, args, timeout = 600000) {
	const result = spawnSync(command, args, { cwd: scratch, encoding: "utf8", timeout });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}
const targetsOf = (target) => (Array.isArray(target) ? target : [target]);
// A mutant that hangs its spec is detected, so a timed-out spec counts as killed.
function spec(target) {
	const result = spawnSync("lune", ["run", "test", ...targetsOf(target)], {
		cwd: scratch,
		encoding: "utf8",
		timeout: 180000,
	});
	if (result.error?.code === "ETIMEDOUT") return { hung: true, output: "" };
	assert.equal(result.error, undefined, String(result.error));
	return { hung: false, status: result.status, output: result.stdout + result.stderr };
}
if (process.env.SHOW !== undefined) {
	for (const index of process.env.SHOW.split(",").map(Number))
		console.log(index, JSON.stringify(mutants[index].slice(0, 5), null, 1));
	process.exit(0);
}
try {
	for (const line of K3) console.log(`K3 ${line}`);
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
	const files = [...new Set(mutants.map(([, , file]) => file))];
	const originals = new Map(files.map((file) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const unmatched = [];
	for (const [index, [rule, kind, file, before]] of mutants.entries()) {
		const count = occurrences(originals.get(file), before);
		if (count !== 1) unmatched.push(`${index} rule ${rule} ${kind}: ${count} matches in ${file}: ${before}`);
	}
	assert.deepEqual(unmatched, [], "every pattern occurs once");
	typecheck(originals);
	if (process.env.DRY !== undefined) {
		console.log(`${mutants.length} mutants match once and compile`);
		process.exit(0);
	}
	const build = () =>
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	assert.equal(build().status, 0, "baseline build");
	const targets = [...new Set(mutants.flatMap((row) => targetsOf(row[5])))];
	const baseline = run("lune", ["run", "test", ...targets]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	const start = Number(process.argv[2] ?? 0);
	const only = process.env.ONLY?.split(",").map(Number);
	const end = Number(process.env.END ?? mutants.length);
	let killed = 0;
	const survivors = [];
	for (const [index, [rule, kind, file, before, after, target]] of mutants.entries()) {
		if (index < start || index >= end || (only !== undefined && !only.includes(index))) continue;
		const original = originals.get(file);
		fs.writeFileSync(path.join(scratch, file), apply(original, before, after));
		const compiled = build();
		assert.equal(
			compiled.status,
			0,
			`${index} rule ${rule} ${kind} build failed\n${compiled.stdout}${compiled.stderr}`,
		);
		const tested = spec(target);
		fs.writeFileSync(path.join(scratch, file), original);
		const output = tested.output;
		const failed = output.split("\n").find((line) => line.startsWith("FAIL "));
		const label = targetsOf(target).join(" + ");
		const cutShort = !tested.hung && !/[0-9]+ passed, [0-9]+ failed/.test(output);
		if (
			!tested.hung &&
			!cutShort &&
			(tested.status === 0 || failed === undefined || !targetsOf(target).some((t) => output.includes(t)))
		) {
			survivors.push(`${index} rule ${rule} ${kind}`);
			console.log(`SURVIVED ${index} | rule ${rule} | ${kind} | ${label}`);
			if (process.env.VERBOSE !== undefined) console.log(`status ${tested.status}`, output.slice(-600));
			continue;
		}
		console.log(
			`KILLED ${index} | rule ${rule} | ${kind} | ${label} | ${tested.hung ? "hung" : (failed ?? "no summary")}`,
		);
		killed++;
	}
	assert.equal(build().status, 0, "restored build");
	console.log(
		`${killed}/${only?.length ?? Math.min(end, mutants.length) - start} profile store mutants killed; restored build 0`,
	);
	assert.deepEqual(survivors, [], "every mutant is killed");
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
