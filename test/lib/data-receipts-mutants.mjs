import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-receipts-mutants-"));
const D = "packages/crucible/src/internal/data/";
const R = D + "receipts.ts";
const I = D + "inbox.ts";
const T = D + "inbox-transforms.ts";
const O = D + "operation-id.ts";
const L = D + "lease.ts";
const S = D + "session.ts";
const P = D + "profiles.ts";
const H = D + "shutdown.ts";
const FAKE = "test/lib/data-backend.luau";
const m = (rule, kind, file, before, after, spec) => [rule, kind, file, before, after, spec];
// [rule, kind, file, before, after, spec]. Kinds: guard deletes a reset or guard, swap exchanges two values of one
// type, pair short-circuits one half of a paired operation.
const mutants = [
	// Receipts: rules 57 to 64.
	m("57", "guard", R, "service.ProcessReceipt = (info) => this.decision(this.process(info));", "", "rule 57 install"),
	m(
		"57",
		"swap",
		R,
		"\t\t\t? Enum.ProductPurchaseDecision.PurchaseGranted\n\t\t\t: Enum.ProductPurchaseDecision.NotProcessedYet;",
		"\t\t\t? Enum.ProductPurchaseDecision.NotProcessedYet\n\t\t\t: Enum.ProductPurchaseDecision.PurchaseGranted;",
		"rule 57 install",
	),
	m(
		"57",
		"pair",
		R,
		"\t\tif (retire && this.service !== undefined)\n\t\t\tthis.service.ProcessReceipt = () => Enum.ProductPurchaseDecision.NotProcessedYet;\n",
		"",
		"rule 57 install",
	),
	m("57", "guard", R, "\t\tthis.closed = true;\n", "", "rule 109 shutdown a purchase during shutdown"),
	m(
		"51",
		"pair",
		H,
		"\tengine.wake();\n\tconst others",
		"\tconst others",
		"rule 109 shutdown a held receipt is woken",
	),
	m("58", "guard", R, '\t\t\tif (this.closed) return "notYet";\n', "", "rule 58 refuses at once"),
	m(
		"58",
		"swap",
		R,
		"this.routes.get(info.ProductId);",
		"this.routes.get(info.PlayerId);",
		"rule 58 a purchase id of exactly 64 bytes",
	),
	m("58", "guard", R, "\t\t\t\t\t`product:${info.ProductId}`,", "\t\t\t\t\t`product`,", "rule 58 refuses at once"),
	m("58", "guard", R, "id.size() < 1 || ", "", "rule 58 refuses at once"),
	m("58", "guard", R, " || id.size() > PURCHASE_ID_MAX_BYTES", "", "rule 58 refuses at once"),
	m("58", "guard", R, '!typeIs(id, "string") || ', "", "rule 58 refuses at once"),
	m(
		"58",
		"swap",
		R,
		"id.size() > PURCHASE_ID_MAX_BYTES",
		"PURCHASE_ID_MAX_BYTES > id.size()",
		"rule 58 refuses at once",
	),
	m(
		"58",
		"swap",
		R,
		"engine.port.players.byUserId(info.PlayerId)",
		"engine.port.players.byUserId(info.ProductId)",
		"rule 58 a purchase id of exactly 64 bytes",
	),
	m(
		"58",
		"guard",
		R,
		'if (player === undefined) return "notYet";',
		'if (player === undefined) return "granted";',
		"rule 58 refuses at once",
	),
	m("58", "guard", R, ' || session.sealedBy === "handoff"', "", "rule 58 a purchase during a handoff that resumes"),
	m("58", "guard", R, "session.phase === Phase.Loading || ", "", "rule 58 a purchase while the profile loads"),
	m(
		"58",
		"swap",
		R,
		'if (bootClock() >= holdUntil) return "notYet";',
		'if (holdUntil >= bootClock()) return "notYet";',
		"rule 58 a purchase before the load starts",
	),
	m(
		"58",
		"guard",
		R,
		"runtime.changed.park(holdUntil);",
		"runtime.changed.park();",
		"rule 58 a held purchase gives up",
	),
	m("58", "guard", R, '\t\t\tif (!runtime.writable(player)) return "notYet";\n', "", "rule 58 refuses at once"),
	m("58", "pair", R, "\t\tthis.routes.delete(productId);\n", "", "rule 58 a woken purchase starts again"),
	m(
		"59",
		"guard",
		R,
		"\t\t\tif (session.durable.has(id)) return this.granted(session, id);\n",
		"",
		"rule 59 an id among the durable receipts",
	),
	m("59", "guard", R, "if (!session.receipts.includes(id)) {", "if (true as boolean) {", "rule 59 a staged id waits"),
	m("59", "pair", R, "\t\t\t\t\t\tsession.stage(id);\n", "", "rule 59 concurrent callbacks"),
	m(
		"59",
		"swap",
		R,
		"const purchase = { id, productId: info.ProductId, currencySpent: info.CurrencySpent };",
		"const purchase = { id, productId: info.CurrencySpent, currencySpent: info.ProductId };",
		"rule 63 a new id grants once in one revision",
	),
	m(
		"60",
		"guard",
		R,
		"\t\t\tif (!ok) {\n\t\t\t\tengine.diagnose(\n\t\t\t\t\t`grant:",
		"\t\t\tif (false as boolean) {\n\t\t\t\tengine.diagnose(\n\t\t\t\t\t`grant:",
		"rule 60 a grant that throws",
	),
	m(
		"60",
		"swap",
		R,
		"(data) => route.grant(player, data, purchase, afterGrant)",
		"(data) => route.grant(player, data as never, afterGrant as never, purchase as never)",
		"rule 61 afterGrant effects run once each",
	),
	m(
		"61",
		"pair",
		R,
		"\t\t\t\t\t\tsession.effects.set(id, effects);\n",
		"",
		"rule 61 afterGrant effects run once each",
	),
	m("61", "pair", R, "\t\t\t\teffects.push(effect);\n", "", "rule 61 afterGrant effects run once each"),
	m("61", "guard", R, "\t\tsession.effects.delete(id);\n", "", "rule 61 afterGrant effects run once each"),
	m(
		"61",
		"guard",
		R,
		"this.engine.port.spawn(() => {",
		"((work: () => void) => work())(() => {",
		"rule 61 afterGrant effects run once each",
	),
	m(
		"61",
		"guard",
		R,
		"\t\t\t\tif (!ok)\n\t\t\t\t\tthis.engine.diagnose(\n\t\t\t\t\t\t`effect:",
		"\t\t\t\tif (false as boolean)\n\t\t\t\t\tthis.engine.diagnose(\n\t\t\t\t\t\t`effect:",
		"rule 61 a throwing effect is diagnosed",
	),
	m(
		"61",
		"swap",
		R,
		"`an afterGrant effect for ${session.schema.name} ${session.userId} threw`",
		"`an afterGrant effect for ${session.userId} ${session.schema.name} threw`",
		"rule 61 a throwing effect is diagnosed",
	),
	m(
		"62",
		"guard",
		R,
		"\t\t\t\tsession.receipts.size() >= MAX_PURCHASE_IDS ||\n",
		"",
		"rule 62 a new id is refused before its grant at the id count",
	),
	m(
		"62",
		"guard",
		R,
		"session.receipts.size() >= MAX_PURCHASE_IDS",
		"session.receipts.size() > MAX_PURCHASE_IDS",
		"rule 62 a new id is refused before its grant at the id count",
	),
	m(
		"62",
		"guard",
		R,
		"jsonBytes(session.receipts) + jsonBytes(id) + 1 > RECEIPTS_MAX_BYTES",
		"jsonBytes(session.receipts) + jsonBytes(id) > RECEIPTS_MAX_BYTES",
		"rule 62 a new id is refused before its grant at RECEIPTS_MAX_BYTES",
	),
	m(
		"62",
		"guard",
		R,
		" ||\n\t\t\t\tjsonBytes(session.receipts) + jsonBytes(id) + 1 > RECEIPTS_MAX_BYTES",
		"",
		"rule 62 a new id is refused before its grant at RECEIPTS_MAX_BYTES",
	),
	m(
		"62",
		"swap",
		R,
		"`receipt capacity reached for ${runtime.schema.name} ${session.userId}`",
		"`receipt capacity reached for ${session.userId} ${runtime.schema.name}`",
		"rule 62 a new id is refused before its grant at the id count",
	),
	m(
		"62",
		"guard",
		R,
		"`receipt-capacity:${session.token}`",
		"`receipt-capacity:${info.PurchaseId}`",
		"rule 62 a new id is refused before its grant at the id count",
	),
	m(
		"63",
		"guard",
		R,
		"!session.durable.has(id) && session.phase !== Phase.Ended",
		"session.phase !== Phase.Ended",
		"rule 59 concurrent callbacks",
	),
	m("63", "guard", R, "session.phase !== Phase.Ended && ", "", "rule 63 a session that ends before"),
	m("63", "guard", R, " && !engine.settled)", ")", "rule 107 retirement wakes held callbacks"),
	m("63", "swap", R, "bootClock() < timeout &&", "timeout < bootClock() &&", "rule 63 a save that cannot land"),
	m(
		"63",
		"guard",
		R,
		"bootClock() + RECEIPT_SAVE_TIMEOUT_SECONDS",
		"bootClock() - RECEIPT_SAVE_TIMEOUT_SECONDS",
		"rule 63 a save that cannot land",
	),
	m(
		"63",
		"guard",
		R,
		'session.durable.has(id) ? this.granted(session, id) : "notYet"',
		"this.granted(session, id)",
		"rule 63 a save that cannot land",
	),
	m("63", "pair", R, "\t\truntime.requestWrite(session, Priority.Receipt);\n", "", "rule 59 concurrent callbacks"),
	m(
		"63",
		"swap",
		R,
		"runtime.requestWrite(session, Priority.Receipt);",
		"runtime.requestWrite(session, Priority.Ordinary);",
		"rule 34 and 63 a receipt save outranks",
	),
	m(
		"64",
		"guard",
		P,
		"\t\tfor (const id of envelope.receipts) session.stage(id);\n",
		"",
		"rule 62 a new id is refused before its grant at the id count",
	),
	m(
		"64",
		"pair",
		P,
		"\t\tsession.durable = new Set(envelope.receipts);\n\t\tconst [ok, claimed]",
		"\t\tconst [ok, claimed]",
		"rule 59 an id among the durable receipts",
	),
	m("107", "pair", P, "\t\tthis.receipts.close(true);\n", "\t\tthis.receipts.close();\n", "rule 57 install"),
	m("107", "guard", P, "\t\tthis.receipts.close(true);\n", "", "rule 107 retirement wakes held callbacks"),
	m("51", "pair", H, "\tengine.receipts.close();\n", "", "rule 109 shutdown a receipt held before the load starts"),
	// Inbox: rules 66 to 73.
	m(
		"66",
		"guard",
		I,
		"\t\tif (!admitted.ok) error(`${this.store}: the payload of message ${message.name} is invalid`, 2);\n",
		'\t\tif (!admitted.ok) return { kind: "unavailable" };\n',
		"rule 66 an invalid or oversize",
	),
	m(
		"66",
		"guard",
		I,
		"\t\tif (bytes > MESSAGE_PAYLOAD_MAX_BYTES)\n",
		"\t\tif (false as boolean)\n",
		"rule 66 an invalid or oversize",
	),
	m(
		"66",
		"swap",
		I,
		"if (bytes > MESSAGE_PAYLOAD_MAX_BYTES)",
		"if (MESSAGE_PAYLOAD_MAX_BYTES > bytes)",
		"rule 66 an invalid or oversize",
	),
	m("66", "guard", I, '!typeIs(sent, "string") || ', "", "rule 66 an invalid or oversize"),
	m("66", "guard", I, "sent.size() > OPERATION_ID_MAX_BYTES || ", "", "rule 66 an invalid or oversize"),
	m("66", "guard", I, " || createdAt(sent) === undefined)", ")", "rule 66 an invalid or oversize"),
	m("66", "guard", I, "version: this.runtime.schema.version,", "version: 0,", "rule 66 a send to an absent key"),
	m(
		"66",
		"pair",
		I,
		'\t\tthis.prompt(userId);\n\t\treturn { kind: "ok", value: undefined };',
		'\t\treturn { kind: "ok", value: undefined };',
		"rule 66 and 71 a send to a player on this server",
	),
	m(
		"66",
		"guard",
		I,
		"if (session.userId === userId && session.phase === Phase.Active)",
		"if (session.phase === Phase.Active)",
		"rule 66 a send prompts only the target user",
	),
	m(
		"66",
		"guard",
		I,
		'\t\t\tkey: tostring(userId),\n\t\t\tpriority: Priority.Ordinary,\n\t\t\tkind: "update",\n\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: bytes + 1024,',
		'\t\t\tkey: tostring(userId + 1),\n\t\t\tpriority: Priority.Ordinary,\n\t\t\tkind: "update",\n\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: bytes + 1024,',
		"rule 66 a send to an absent key",
	),
	m(
		"67",
		"guard",
		I,
		'if (decision.kind !== "appended" && decision.kind !== "duplicate") return { kind: decision.kind };',
		'if (decision.kind !== "appended" && decision.kind !== "duplicate") return { kind: "ok", value: undefined };',
		"rule 68 the same id",
	),
	m(
		"67",
		"guard",
		I,
		' && decision.kind !== "duplicate") return { kind: decision.kind };',
		') return { kind: decision.kind === "duplicate" ? "reused" : decision.kind };',
		"rule 68 the same id",
	),
	m(
		"67",
		"swap",
		I,
		'{ kind: ticket.dispatched ? "uncertain" : "timeout" };\n\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error" || decision === undefined)',
		'{ kind: ticket.dispatched ? "timeout" : "uncertain" };\n\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error" || decision === undefined)',
		"rule 67 timeout when never dispatched",
	),
	m(
		"67",
		"guard",
		I,
		'if (decision.kind === "unreadable") return { kind: "unavailable" };',
		'if (decision.kind === "unreadable") return { kind: "ok", value: undefined };',
		"rule 67 unavailable",
	),
	m(
		"67",
		"guard",
		I,
		"\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: bytes + 1024,",
		"\t\t\tdispatchDeadline: math.huge,\n\t\t\tbytes: bytes + 1024,",
		"rule 67 timeout when never dispatched",
	),
	m(
		"69",
		"guard",
		I,
		'\t\tif (ageOf(sent, port.wall(), MESSAGE_ID_MAX_AGE_SECONDS) !== "fresh") return { kind: "expired" };\n',
		"",
		"rule 69 an id outside its age horizon",
	),
	m(
		"69",
		"swap",
		I,
		"ageOf(sent, port.wall(), MESSAGE_ID_MAX_AGE_SECONDS)",
		"ageOf(sent, MESSAGE_ID_MAX_AGE_SECONDS, port.wall())",
		"rule 69 an id outside its age horizon",
	),
	m(
		"73",
		"guard",
		I,
		'if (parsed.kind === "absent") return { kind: "ok", value: [] };',
		'if (parsed.kind === "absent") return { kind: "unavailable" };',
		"rule 73 quarantined reads an absent",
	),
	m(
		"73",
		"guard",
		I,
		'if (parsed.kind === "unreadable") return { kind: "unavailable" };',
		'if (parsed.kind === "unreadable") return { kind: "ok", value: [] };',
		"rule 73 quarantined reads an absent",
	),
	m("73", "guard", I, "sentAt: createdAt(item.id) ?? 0,", "sentAt: 0,", "rule 73 quarantined lists"),
	m(
		"73",
		"swap",
		I,
		"\t\t\t\tattempts: item.attempts,\n\t\t\t\tsentAt:",
		"\t\t\t\tattempts: item.attempts + 1,\n\t\t\t\tsentAt:",
		"rule 73 quarantined lists",
	),
	m(
		"73",
		"guard",
		I,
		'if (decision.kind === "full") return { kind: "full" };',
		'if (decision.kind === "full") return { kind: "ok", value: 0 };',
		"rule 73 clearQuarantined retry returns full",
	),
	m(
		"73",
		"pair",
		I,
		"\t\tif (decision.changed > 0) this.prompt(userId);\n",
		"",
		"rule 73 a retried item is attempted at once",
	),
	m(
		"73",
		"guard",
		I,
		'if (!typeIs(id, "string") || id.size() > OPERATION_ID_MAX_BYTES)',
		"if (false as boolean)",
		"rule 73 clearQuarantined refuses an id",
	),
	m(
		"73",
		"swap",
		I,
		"clearQuarantine(current, userId, listed, action, (d) => (decision = d))",
		'clearQuarantine(current, userId, listed, action === "retry" ? "discard" : "retry", (d) => (decision = d))',
		"rule 73 clearQuarantined retry returns full",
	),
	m(
		"71",
		"guard",
		I,
		"if (revision <= session.confirmed) session.acks.delete(id);",
		"",
		"rule 72 a confirmed envelope retires",
	),
	m(
		"71",
		"swap",
		I,
		"if (revision <= session.confirmed) session.acks.delete(id);",
		"if (session.confirmed <= revision) session.acks.delete(id);",
		"rule 72 a confirmed envelope retires",
	),
	m(
		"72",
		"guard",
		I,
		"\t\t\tif (item?.generation === mark.generation) continue;\n",
		"",
		"rule 72 a confirmed envelope retires",
	),
	m(
		"72",
		"swap",
		I,
		"if (item?.generation === mark.generation) continue;",
		"if (item?.generation === mark.attempts) continue;",
		"rule 72 a confirmed envelope retires",
	),
	m("72", "pair", I, "\t\t\tsession.marks.delete(id);\n", "", "rule 72 a confirmed envelope retires"),
	m("72", "guard", I, "\t\t\t\tmark.quarantine !== undefined &&\n", "", "rule 72 a confirmed envelope retires"),
	m(
		"72",
		"swap",
		I,
		"item?.generation === mark.generation + 1 &&\n\t\t\t\titem.attempts === mark.attempts",
		"item?.generation === mark.attempts + 1 &&\n\t\t\t\titem.attempts === mark.generation",
		"rule 72 a confirmed envelope retires",
	),
	m("72", "pair", I, "\t\tsession.inbox = envelope.inbox;\n", "", "rule 72 items apply in stored order"),
	m(
		"72",
		"guard",
		I,
		"\t\tif (!this.runtime.accept(session)) return;\n",
		"",
		"rule 72 a sealed session applies nothing",
	),
	m(
		"72",
		"pair",
		I,
		"\t\tfor (const [id] of session.retryAt) if (!present.has(id)) session.retryAt.delete(id);\n",
		"",
		"rule 73 a retried item is attempted at once",
	),
	m(
		"72",
		"guard",
		I,
		"receiver === undefined || session.acks.has(item.id) || session.marks.has(item.id)",
		"receiver === undefined || session.marks.has(item.id)",
		"rule 72 a stale scan never reapplies",
	),
	m(
		"72",
		"guard",
		I,
		"receiver === undefined || session.acks.has(item.id) || session.marks.has(item.id)",
		"receiver === undefined || session.acks.has(item.id)",
		"rule 73 a marked item is not attempted again",
	),
	m(
		"72",
		"guard",
		I,
		"if (item.version > this.runtime.schema.version) {",
		"if (false as boolean) {",
		"rule 72 an item from a newer build",
	),
	m(
		"72",
		"swap",
		I,
		"if (item.version > this.runtime.schema.version) {",
		"if (this.runtime.schema.version > item.version) {",
		"rule 72 an item from a newer build",
	),
	m(
		"72",
		"swap",
		I,
		"`inbox-version:${session.token}:${item.id}`",
		"`inbox-version:${session.token}`",
		"rule 72 an item from a newer build",
	),
	m(
		"72",
		"guard",
		I,
		"if (at === undefined && item.attempts > 0) {",
		"if (at === undefined) {",
		"rule 72 items apply in stored order",
	),
	m(
		"72",
		"swap",
		I,
		"if (at !== undefined && now < at) {",
		"if (at !== undefined && at < now) {",
		"rule 73 a throwing receiver",
	),
	m(
		"72",
		"guard",
		I,
		'\t\t\t\t\tquarantine: "payload",\n',
		"\t\t\t\t\tquarantine: undefined,\n",
		"rule 72 a payload this build refuses",
	),
	m(
		"72",
		"swap",
		I,
		'generation: item.generation,\n\t\t\t\t\tattempts: item.attempts,\n\t\t\t\t\tquarantine: "payload",',
		'generation: item.attempts,\n\t\t\t\t\tattempts: item.generation,\n\t\t\t\t\tquarantine: "payload",',
		"rule 72 a payload this build refuses",
	),
	m(
		"72",
		"swap",
		I,
		"generation: item.generation,\n\t\t\t\t\tattempts,\n",
		"generation: attempts,\n\t\t\t\t\tattempts: item.generation,\n",
		"rule 73 a throwing receiver",
	),
	m(
		"73",
		"guard",
		I,
		'quarantine: attempts >= INBOX_MAX_ATTEMPTS ? "attempts" : undefined,',
		"quarantine: undefined,",
		"rule 73 a throwing receiver",
	),
	m(
		"73",
		"guard",
		I,
		'quarantine: attempts >= INBOX_MAX_ATTEMPTS ? "attempts" : undefined,',
		'quarantine: attempts > INBOX_MAX_ATTEMPTS ? "attempts" : undefined,',
		"rule 73 a throwing receiver",
	),
	m(
		"73",
		"pair",
		I,
		"\t\t\t\tsession.retryAt.set(item.id, now + retryAfter(attempts));\n",
		"",
		"rule 73 a throwing receiver",
	),
	m(
		"73",
		"swap",
		I,
		"return INBOX_RETRY_SECONDS[math.min(attempts, INBOX_RETRY_SECONDS.size()) - 1];",
		"return INBOX_RETRY_SECONDS[math.max(attempts, INBOX_RETRY_SECONDS.size()) - 1];",
		"rule 73 a throwing receiver",
	),
	m(
		"72",
		"pair",
		I,
		"\t\tif (staged) this.runtime.requestWrite(session, Priority.Renew);\n",
		"",
		"rule 71 an owner's write removes only",
	),
	m("72", "pair", I, "\t\t\t} else staged = true;\n", "\t\t\t}\n", "rule 71 an owner's write removes only"),
	m(
		"72",
		"pair",
		I,
		"\t\t\t\t() => session.acks.set(item.id, session.revision + 1),\n",
		"\t\t\t\t() => session.acks.set(item.id, session.revision),\n",
		"rule 72 an item applied by a timer while an older save",
	),
	m(
		"73",
		"pair",
		I,
		"\t\tif (soonest !== undefined) {",
		"\t\tif (soonest !== undefined && (false as boolean)) {",
		"rule 73 a throwing receiver",
	),
	m(
		"73",
		"swap",
		I,
		"soonest = math.min(soonest ?? at, at);",
		"soonest = math.max(soonest ?? at, at);",
		"rule 73 a marked item is not attempted again",
	),
	m(
		"72",
		"guard",
		P,
		"\t\tthis.hooks.delivered(session, envelope);\n\t\tthis.inbox.deliver(session, envelope);\n\t\tsession.renewAt",
		"\t\tthis.hooks.delivered(session, envelope);\n\t\tsession.renewAt",
		"rule 72 items apply in stored order",
	),
	m(
		"71",
		"pair",
		P,
		"\t\tthis.hooks.delivered(session, envelope);\n\t\tthis.inbox.deliver(session, envelope);\n\t}",
		"\t\tthis.hooks.delivered(session, envelope);\n\t}",
		"rule 71 an owner's write removes only",
	),
	// Pure envelope transforms: rules 68 to 73.
	m(
		"72",
		"guard",
		T,
		"\t\tinbox: envelope.inbox.filter((item) => !gone.has(item.id)),\n",
		"\t\tinbox: envelope.inbox,\n",
		"rule 72 items apply in stored order",
	),
	m(
		"72",
		"guard",
		T,
		"\t\tquarantine: envelope.quarantine.filter((item) => !gone.has(item.id)),\n",
		"\t\tquarantine: envelope.quarantine,\n",
		"rule 72 an ack removes its id from the quarantine",
	),
	m("72", "pair", T, "\t\t...addSeen(envelope, acks),\n", "", "rule 72 items apply in stored order"),
	m(
		"69",
		"guard",
		T,
		"while (seen.size() > SEEN_MAX_MESSAGES) {",
		"while (seen.size() > SEEN_MAX_MESSAGES + 1) {",
		"rule 69 an ack evicts",
	),
	m(
		"69",
		"swap",
		T,
		"floor = math.max(floor, createdAt(evicted) ?? 0);",
		"floor = math.min(floor, createdAt(evicted) ?? 0);",
		"rule 69 an ack evicts",
	),
	m("69", "guard", T, "const evicted = seen.shift()!;", "const evicted = seen.pop()!;", "rule 69 an ack evicts"),
	m("72", "guard", T, " && item.generation === mark.generation);", ");", "rule 72 R3 a stale payload"),
	m(
		"72",
		"swap",
		T,
		"attempts: mark.attempts, generation: item.generation + 1 }",
		"attempts: item.attempts, generation: item.generation + 1 }",
		"rule 73 a throwing receiver",
	),
	m(
		"72",
		"swap",
		T,
		"attempts: mark.attempts, generation: item.generation + 1 }",
		"attempts: mark.attempts, generation: item.generation + 2 }",
		"rule 72 a payload this build refuses",
	),
	m(
		"70",
		"guard",
		T,
		"moved.size() <= QUARANTINE_MAX_MESSAGES",
		"moved.size() < QUARANTINE_MAX_MESSAGES",
		"rule 72 a full quarantine",
	),
	m(
		"70",
		"guard",
		T,
		"\t\t\tmoved.size() <= QUARANTINE_MAX_MESSAGES &&\n\t\t\tjsonBytes(moved) <= QUARANTINE_MAX_BYTES;",
		"\t\t\tmoved.size() <= QUARANTINE_MAX_MESSAGES;",
		"rule 72 a full quarantine",
	),
	m("72", "guard", T, "\t\t\tmark.quarantine !== undefined &&\n", "\t\t\ttrue &&\n", "rule 73 a throwing receiver"),
	m("72", "pair", T, "\t\t\tquarantine = moved;\n", "", "rule 72 a payload this build refuses"),
	m(
		"68",
		"guard",
		T,
		"\t\tif (!sameValue(value, (b as Record<string, unknown>)[key as string])) return false;\n",
		"",
		"rule 68 the same id",
	),
	m("68", "guard", T, "\treturn count === 0;", "\treturn true;", "rule 68 the same id"),
	m("68", "guard", T, "held.name === item.name && sameValue", "sameValue", "rule 68 the same id"),
	m(
		"68",
		"swap",
		T,
		'sameValue(held.payload, item.payload) ? "duplicate" : "reused"',
		'sameValue(held.payload, item.payload) ? "reused" : "duplicate"',
		"rule 68 the same id",
	),
	m(
		"68",
		"guard",
		T,
		"\tif (envelope.seen.includes(item.id)) {\n",
		"\tif (false as boolean) {\n",
		"rule 68 a quarantined id",
	),
	m(
		"69",
		"guard",
		T,
		' !== "fresh" || createdAt(item.id)! <= envelope.seenFloor',
		' !== "fresh"',
		"rule 69 an id at or below the seen floor",
	),
	m(
		"69",
		"guard",
		T,
		'ageOf(item.id, now, MESSAGE_ID_MAX_AGE_SECONDS) !== "fresh" || ',
		"",
		"rule 69 an id outside its age horizon",
	),
	m(
		"69",
		"guard",
		T,
		"createdAt(item.id)! <= envelope.seenFloor",
		"createdAt(item.id)! < envelope.seenFloor",
		"rule 69 an id at or below the seen floor",
	),
	m(
		"69",
		"swap",
		T,
		"ageOf(item.id, now, MESSAGE_ID_MAX_AGE_SECONDS)",
		"ageOf(item.id, MESSAGE_ID_MAX_AGE_SECONDS, now)",
		"rule 69 an id outside its age horizon",
	),
	m(
		"70",
		"guard",
		T,
		'\tif (inbox.size() > INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\trecord({ kind: "full" });\n\t\treturn undefined;\n\t}\n\tif (envelope.seen',
		'\tif (jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\trecord({ kind: "full" });\n\t\treturn undefined;\n\t}\n\tif (envelope.seen',
		"rule 70 a full inbox",
	),
	m(
		"70",
		"guard",
		T,
		'\tif (inbox.size() > INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\trecord({ kind: "full" });\n\t\treturn undefined;\n\t}\n\tif (envelope.seen',
		'\tif (inbox.size() >= INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\trecord({ kind: "full" });\n\t\treturn undefined;\n\t}\n\tif (envelope.seen',
		"rule 70 a full inbox",
	),
	m(
		"70",
		"guard",
		T,
		'\tif (inbox.size() > INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\trecord({ kind: "full" });\n\t\treturn undefined;\n\t}\n\tif (envelope.seen',
		'\tif (inbox.size() > INBOX_MAX_MESSAGES) {\n\t\trecord({ kind: "full" });\n\t\treturn undefined;\n\t}\n\tif (envelope.seen',
		"rule 70 a full inbox",
	),
	m(
		"69",
		"guard",
		T,
		"envelope.seen.size() >= SEEN_MAX_MESSAGES && !evictable(envelope.seen[0], now)",
		"envelope.seen.size() > SEEN_MAX_MESSAGES && !evictable(envelope.seen[0], now)",
		"rule 69 a ring full of young ids",
	),
	m(
		"69",
		"guard",
		T,
		"envelope.seen.size() >= SEEN_MAX_MESSAGES && !evictable(envelope.seen[0], now)",
		"envelope.seen.size() >= SEEN_MAX_MESSAGES && evictable(envelope.seen[0], now)",
		"rule 69 a ring full of young ids",
	),
	m(
		"69",
		"swap",
		T,
		"!evictable(envelope.seen[0], now)",
		"!evictable(envelope.seen[1], now)",
		"rule 69 a ring full of young ids",
	),
	m(
		"73",
		"guard",
		T,
		"envelope.quarantine.filter((item) => ids.has(item.id))",
		"envelope.quarantine.filter((item) => !ids.has(item.id))",
		"rule 73 quarantined lists",
	),
	m(
		"73",
		"guard",
		T,
		'\t\tif (inbox.size() > INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\t\trecord({ kind: "full" });\n\t\t\treturn undefined;\n\t\t}\n\t\twritten',
		'\t\tif (inbox.size() >= INBOX_MAX_MESSAGES || jsonBytes(inbox) > INBOX_MAX_BYTES) {\n\t\t\trecord({ kind: "full" });\n\t\t\treturn undefined;\n\t\t}\n\t\twritten',
		"rule 73 clearQuarantined refuses an id",
	),
	m(
		"73",
		"guard",
		T,
		"\t\t\t\tgeneration: item.generation + 1,\n\t\t\t\tattempts: 0,",
		"\t\t\t\tgeneration: item.generation,\n\t\t\t\tattempts: 0,",
		"rule 72 R3 a stale payload",
	),
	m(
		"73",
		"guard",
		T,
		"\t\t\t\tgeneration: item.generation + 1,\n\t\t\t\tattempts: 0,",
		"\t\t\t\tgeneration: item.generation + 1,\n\t\t\t\tattempts: item.attempts,",
		"rule 72 R3 a stale attempts",
	),
	m(
		"73",
		"swap",
		T,
		"\t\t\t\tgeneration: item.generation + 1,\n\t\t\t\tattempts: 0,",
		"\t\t\t\tgeneration: 0,\n\t\t\t\tattempts: item.generation + 1,",
		"rule 72 R3 a stale payload",
	),
	m(
		"73",
		"pair",
		T,
		"\t\t\t...addSeen(\n\t\t\t\tenvelope,\n\t\t\t\tlisted.map((item) => item.id),\n\t\t\t),\n",
		"",
		"rule 73 clearQuarantined discard",
	),
	m(
		"73",
		"guard",
		T,
		"\t\twritten = { ...envelope, inbox, quarantine };",
		"\t\twritten = { ...envelope, inbox };",
		"rule 73 quarantined lists",
	),
	// Operation ids: rule 69.
	m(
		"69",
		"swap",
		O,
		'if (created > now + ID_FUTURE_SKEW_SECONDS) return "future";',
		'if (created >= now + ID_FUTURE_SKEW_SECONDS) return "future";',
		"rule 69 an id outside its age horizon",
	),
	m(
		"69",
		"swap",
		O,
		'if (now - created > maxAgeSeconds) return "expired";',
		'if (now - created >= maxAgeSeconds) return "expired";',
		"rule 69 an id outside its age horizon",
	),
	m(
		"69",
		"swap",
		O,
		"return `o1-${base36(port.wall())}-${guid}`;",
		"return `o1-${guid}-${base36(port.wall())}`;",
		"rule 69 a send without an id",
	),
	m(
		"69",
		"guard",
		O,
		"let rest = math.floor(value);",
		"let rest = math.floor(value) + 1;",
		"rule 69 a send without an id",
	),
	m("69", "guard", O, "const digit = rest % 36;", "const digit = rest % 35;", "rule 69 a send without an id"),
	// The snapshot seam and the acquisition's carry.
	m(
		"72",
		"pair",
		L,
		"\t\t\t: applyMarks(envelope, input.snapshot.marks)),",
		"\t\t\t: envelope),",
		"rule 73 a throwing receiver",
	),
	m("72", "pair", L, "\t\t\tsnapshot.acks,\n", "\t\t\t[],\n", "rule 72 items apply in stored order"),
	m("72", "pair", L, "\t\tsnapshot.marks,\n", "\t\t[],\n", "rule 29 and 72 a sealed session's pending mark"),
	m(
		"72",
		"swap",
		S,
		"applied.sort((a, b) => a[1] < b[1]);",
		"applied.sort((a, b) => a[1] > b[1]);",
		"rule 72 items apply in stored order",
	),
	m(
		"72",
		"pair",
		S,
		"\t\t\tacks,\n\t\t\tmarks,\n",
		"\t\t\tacks: [],\n\t\t\tmarks,\n",
		"rule 72 items apply in stored order",
	),
	// The rule 108 fake's failure-plan offset, which the stale-mark specs rely on.
	m(
		"108",
		"guard",
		FAKE,
		"if plan.remaining > 0 and call.index > plan.after and matches(plan.filter, call) then",
		"if plan.remaining > 0 and matches(plan.filter, call) then",
		"rule 72 R3 a stale payload",
	),
	m("108", "swap", FAKE, "after = options and options.after or 0,", "after = 0,", "rule 72 R3 a stale payload"),
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
		texts.set(absolute, originals.get(file).replace(before, after));
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
// A mutant that hangs its spec is detected, so a timed-out spec counts as killed.
function spec(target) {
	const result = spawnSync("lune", ["run", "test", target], { cwd: scratch, encoding: "utf8", timeout: 180000 });
	if (result.error?.code === "ETIMEDOUT") return { hung: true, output: "" };
	assert.equal(result.error, undefined, String(result.error));
	return { hung: false, status: result.status, output: result.stdout + result.stderr };
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
	const files = [...new Set(mutants.map(([, , file]) => file))];
	const originals = new Map(files.map((file) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	for (const [rule, kind, file, before] of mutants) {
		const count = originals.get(file).split(before).length - 1;
		assert.equal(count, 1, `rule ${rule} ${kind}: pattern must occur once in ${file}: ${before}`);
	}
	typecheck(originals);
	const build = () =>
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	assert.equal(build().status, 0, "baseline build");
	const targets = [...new Set(mutants.map((m) => m[5]))];
	const baseline = run("lune", ["run", "test", ...targets]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	const start = Number(process.argv[2] ?? 0);
	const only = process.env.ONLY?.split(",").map(Number);
	let killed = 0;
	const survivors = [];
	for (const [index, [rule, kind, file, before, after, target]] of mutants.entries()) {
		if (index < start || (only !== undefined && !only.includes(index))) continue;
		const original = originals.get(file);
		fs.writeFileSync(path.join(scratch, file), original.replace(before, after));
		const compiled = build();
		assert.equal(
			compiled.status,
			0,
			`${index} rule ${rule} ${kind} build failed\n${compiled.stdout}${compiled.stderr}`,
		);
		const tested = spec(target);
		fs.writeFileSync(path.join(scratch, file), original);
		const output = tested.output;
		if (!tested.hung && (tested.status === 0 || !output.includes("FAIL ") || !output.includes(target))) {
			survivors.push(`${index} rule ${rule} ${kind}`);
			console.log(`SURVIVED ${index} | rule ${rule} | ${kind} | ${target}`);
			continue;
		}
		console.log(`KILLED ${index} | rule ${rule} | ${kind} | ${target}${tested.hung ? " | hung" : ""}`);
		killed++;
	}
	assert.equal(build().status, 0, "restored build");
	console.log(
		`${killed}/${only?.length ?? mutants.length - start} receipt and inbox mutants killed; restored build 0`,
	);
	assert.deepEqual(survivors, [], "every mutant is killed");
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
