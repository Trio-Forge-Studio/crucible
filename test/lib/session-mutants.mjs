import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const repo = process.cwd();
const src = "packages/crucible/src/";
const S = src + "internal/network/session.ts";
const SS = src + "internal/network/server-session.ts";
const CS = src + "internal/network/client-session.ts";
const CL = src + "internal/network/claims.ts";
const E = src + "internal/network/envelope.ts";
const RP = src + "internal/network/roblox-network-port.ts";
const B = src + "internal/boot.ts";
const N = src + "network.ts";
const EN = src + "emit/network.ts";
const CI = src + "client/index.ts";
const SI = src + "server/index.ts";
const specs = [
	"network-handshake",
	"network-ready",
	"network-ownership",
	"network-asks",
	"network-instances",
	"network-callbacks",
	"network-peers",
	"network-boot",
	"network-envelope",
	"boot-platform",
	"network-pins",
];

// Ruling K3: each rule carries a guard (or reset), a swap and a pair mutant, or says why the code has no such site.
// Fixture values differ wherever two same-typed arguments could swap, so equal values cannot hide a swap.
const rules = [
	[
		"7",
		[
			"guard",
			N,
			"export function toServer<P>(_name: string): ToServer<P> {\n\terror(UNTRANSFORMED);",
			"export function toServer<P>(_name: string): ToServer<P> {\n\treturn undefined as never;",
		],
		["swap", "each macro throws one constant and takes no second argument of its type"],
		["pair", "a macro is a single throw with nothing paired to it"],
	],
	[
		"8",
		[
			"guard",
			EN,
			'import { claim } from "../internal/network/claims";',
			'import { claim } from "../internal/network/claims";\nimport "../internal/network/port";',
		],
		["swap", "isolation is an import graph with no call to swap"],
		["pair", "isolation is an import graph with no paired operation"],
		["guard installed marker (A17 L03)", "test/lib/load.luau", "\t\t\t\tnetwork.installed = true\n", "\n"],
		[
			"swap config slot (A17 L05)",
			"test/lib/load.luau",
			"loaded[path].installNetworkPort(network)",
			"loaded[path].installNetworkPort(configPort or network)",
		],
	],
	[
		"12",
		["guard", SS, "\t\tthis.port.publish(L.PROTOCOL_VERSION, this.catalog);\n", "\n"],
		[
			"swap",
			RP,
			"\t\t\ttarget.SetAttribute(PROTOCOL_ATTRIBUTE, protocol);\n\t\t\ttarget.SetAttribute(CATALOG_ATTRIBUTE, catalog);\n\t\t\ttarget.Parent = storage;",
			"\t\t\ttarget.Parent = storage;\n\t\t\ttarget.SetAttribute(PROTOCOL_ATTRIBUTE, protocol);\n\t\t\ttarget.SetAttribute(CATALOG_ATTRIBUTE, catalog);",
		],
		["pair", SS, "\t\tthis.catalog = seal();", "\t\tseal();"],
	],
	[
		"13",
		["guard", CS, 'if (found.className !== "RemoteEvent")', "if (false as boolean)"],
		["swap", "discovery compares each attribute with one constant"],
		["pair", CS, "if (found.protocol !== L.PROTOCOL_VERSION)", "if (false as boolean)"],
	],
	[
		"16",
		[
			"guard",
			SS,
			'if (definition === undefined || definition.kind !== "to-client")',
			"if (definition === undefined)",
		],
		[
			"swap send",
			SI,
			"active().send(player, message, (payload as unknown[])[0])",
			"active().send(player, (payload as unknown[])[0], message)",
		],
		[
			"swap ask",
			CI,
			"active().ask(message, (request as unknown[])[0])",
			"active().ask((request as unknown[])[0], message)",
		],
		[
			"pair",
			CS,
			"if (definition === undefined || definition.kind !== kind) error(message, 0);",
			"if (definition === undefined) error(message, 0);",
		],
	],
	[
		"17",
		["guard", S, "if (this.stage === Stage.Idle || this.stage === Stage.Open)", "if (this.stage === Stage.Idle)"],
		["swap", "the readiness check compares the stage with constants"],
		["pair", SS, "\t\tthis.catalog = seal();\n\t\tthis.stage = Stage.Loaded;", "\t\tthis.catalog = seal();"],
	],
	[
		"18",
		["guard", "every payload is read only inside encodeBody at the call; nothing later reads it"],
		["swap", "encodeBody's arguments differ in type except name and path, swapped under rule 19"],
		["pair", "no deferred encode exists to pair with the call"],
	],
	[
		"19",
		["guard", E, "\tif (!ok) error(describe(name, path, err), 0);", ""],
		[
			"swap",
			E,
			"\tif (!ok) error(describe(name, path, err), 0);",
			"\tif (!ok) error(describe(path, name, err), 0);",
		],
		["pair", E, "\tif (!ok) error(describe(body.name, body.path, err), 0);", ""],
	],
	[
		"20",
		[
			"guard",
			SS,
			"return peer !== undefined && this.deliver(peer, frame(body, Opcode.Event, [definition.index!]));",
			"return this.deliver(peer!, frame(body, Opcode.Event, [definition.index!]));",
		],
		["swap", "deliver takes a peer and a packet"],
		[
			"pair",
			S,
			"\t\t\tthis.fail(peer, FailReason.Overflow);\n\t\t\treturn false;",
			"\t\t\tthis.fail(peer, FailReason.Overflow);\n\t\t\treturn true;",
		],
	],
	[
		"21",
		["guard", "broadcast has no guard of its own; deliver's guards belong to rules 20 and 50"],
		["swap", "broadcast delivers one packet to one peer at a time"],
		[
			"pair",
			SS,
			"for (const peer of [...this.order]) this.deliver(peer, packet);",
			"for (const peer of this.order) this.deliver(peer, packet);",
		],
	],
	[
		"22",
		[
			"guard",
			S,
			"\t\tif (peer.state === PeerState.Ready) {\n\t\t\tthis.fire(peer, packet);",
			"\t\tif (false as boolean) {\n\t\t\tthis.fire(peer, packet);",
		],
		["swap", "a ready send is one fire of one packet"],
		["pair", "nothing batches, so there is no flush to pair with a ready send"],
	],
	[
		"24",
		["guard", CL, 'if (serverSide !== (side === "server"))', "if (false as boolean)"],
		["swap", "the side check compares one kind with one side"],
		["pair", CS, 'openClaims({ side: "client"', 'openClaims({ side: "server"'],
	],
	[
		"25",
		[
			"guard K9",
			EN,
			"export function receive<T extends ReceiveTarget>(\n\towner: object,\n\tfield: string,\n\ttarget: T,\n\tcallback: ReceiveCallback<T>,\n): Receiver {",
			"export function receive(owner: object, field: string, target: unknown, callback: Callback): Receiver {",
			["network-api"],
		],
		["swap", "the target and its callback differ in type"],
		["pair", "the callback's type derives from the target; the guard mutant unpairs them"],
	],
	[
		"26",
		[
			"guard",
			CL,
			"if (other !== undefined) error(`${definition.name} has two receivers: ${other.label} and ${label}`);",
			"",
		],
		[
			"swap",
			CL,
			"has two receivers: ${other.label} and ${label}",
			"has two receivers: ${label} and ${other.label}",
		],
		["pair", CL, "else receivers.set(target as Definition, entry);", "else {}"],
	],
	[
		"27",
		["guard", CL, "return table.freeze({}) as unknown as Receiver;", "return callback as unknown as Receiver;"],
		["swap", "the field holds one value"],
		["pair", "a Receiver pairs with nothing"],
	],
	[
		"28",
		["guard", CL, "if (frozen) error(`${label}: receive() after the network commit`);", ""],
		[
			"guard before commit",
			SS,
			'if (this.stage !== Stage.Committed) return this.reject(peer, "envelope", "a packet before the network commit");',
			"",
		],
		["swap", "the freeze and the commit check take no arguments to swap"],
		["pair", SS, "\t\tfreezeClaims();\n\t\tthis.port.publish", "\t\tthis.port.publish"],
	],
	[
		"29",
		["guard", SS, "\t\t\t\tentry.settled = true;\n\t\t\t\tthis.fire(", "\t\t\t\tthis.fire("],
		[
			"swap",
			SS,
			"[request.definition.index!, request.nonce], ErrorReason.Unavailable",
			"[request.nonce, request.definition.index!], ErrorReason.Unavailable",
		],
		["pair", CL, "\tstate.subscriptions.push(() => retire(entry));", ""],
	],
	[
		"30",
		[
			"guard",
			SS,
			"return this.start(peer, receiver, [player, value]);",
			"return this.start(peer, receiver, [value, value]);",
		],
		["swap", "the engine sender and the payload differ in type"],
		["pair", "the sender is read once from the engine"],
	],
	[
		"31",
		[
			"guard",
			SS,
			'if (definition === undefined || definition.kind !== "to-client")\n\t\terror(`${call} needs a message declared with toClient`, 0);\n\treturn definition;',
			'if (definition !== undefined && definition.kind !== "to-client")\n\t\terror(`${call} needs a message declared with toClient`, 0);\n\treturn definition!;',
		],
		["swap", "ClientReady is one frozen value"],
		["pair", "ClientReady never enters the catalog, so nothing pairs with it there"],
	],
	[
		"32",
		["guard", CL, "\t\t\tif (other.owner === state)\n", "\t\t\tif (false as boolean)\n"],
		[
			"swap",
			CL,
			"${label}: ${other.label} already receives ClientReady",
			"${other.label}: ${label} already receives ClientReady",
		],
		["pair", CL, "if (target === ClientReady) ready.push(entry);", "if (target === ClientReady) {}"],
	],
	[
		"33",
		[
			"guard",
			SS,
			"\t\t\tif (peer.state !== PeerState.Ready) return;\n\t\t\tif (!isLive(claim)) continue;",
			"\t\t\tif (!isLive(claim)) continue;",
		],
		["swap", "a pass runs one claim for one player at a time"],
		["pair", SS, "\t\tpeer.state = PeerState.Ready;\n\t\tthis.flush(peer);", "\t\tpeer.state = PeerState.Ready;"],
	],
	[
		"34",
		[
			"guard",
			S,
			"\t\tif (result === CallResult.Ok) return $tuple(true, value);",
			"\t\treturn $tuple(true, value);",
		],
		[
			"swap",
			S,
			"within(claim.owner, undefined, Hook.Callback, invoke, work, argument)",
			"within(undefined, claim.owner, Hook.Callback, invoke, work, argument)",
		],
		["pair", SS, "\t\t\telse if (value !== undefined)\n", "\t\t\telse if (false as boolean)\n"],
	],
	[
		"35",
		[
			"guard",
			S,
			"if (isLive(entry.claim) && peer.state === PeerState.Ready) peer.cleanups.push(entry);",
			"if (true as boolean) peer.cleanups.push(entry);",
		],
		["swap", "deferCleanup takes a peer and a cleanup of different types"],
		[
			"order",
			S,
			"for (let i = cleanups.size() - 1; i >= 0; i--) this.deferCleanup(peer, cleanups[i]);",
			"for (let i = 0; i < cleanups.size(); i++) this.deferCleanup(peer, cleanups[i]);",
		],
		[
			"pair",
			S,
			"\t\tdefer(() => this.runReady(peer, entry.claim, entry.cleanup, undefined));",
			"\t\tthis.runReady(peer, entry.claim, entry.cleanup, undefined);",
		],
	],
	[
		"36",
		["guard", SS, "\t\t\tif (!isLive(claim)) continue;\n", "\n"],
		["swap", "the liveness check reads one claim"],
		["pair", CL, "if (!live.delete(claim)) return;", "if (!live.has(claim)) return;"],
		["guard copy", CL, "return [...ready];", "return ready;"],
	],
	[
		"37",
		["guard", CS, "\t\tentry.deadline = this.port.delay(definition.timeout, () => this.expire(nonce));\n", "\n"],
		["swap", "the deadline takes one timeout and one callback"],
		["pair", CS, "\t\tthis.pending.set(nonce, entry);\n", "\n"],
	],
	[
		"38",
		[
			"guard",
			CS,
			"if (reason === undefined) return this.violation(`an unknown ERROR reason ${header.reason}`);",
			"",
		],
		[
			"swap",
			CS,
			'["unavailable", "handler-error", "overloaded", "invalid"]',
			'["handler-error", "unavailable", "overloaded", "invalid"]',
		],
		["pair", CS, 'if (this.pending.size() >= L.MAX_PENDING_ASKS) return failed("overloaded");', ""],
	],
	[
		"39",
		["guard", "the writer's range check is the only guard, and it is the codec's"],
		["swap", CS, "frame(body, Opcode.Request, [index, nonce])", "frame(body, Opcode.Request, [nonce, index])"],
		["pair", CS, "\t\tthis.nextNonce = nonce + 1;\n", "\n"],
	],
	[
		"40",
		[
			"guard",
			CS,
			"\t\tif (index !== -1) {\n\t\t\tthis.peer.queue.remove(index);",
			"\t\tif (false as boolean) {\n\t\t\tthis.peer.queue.remove(index);",
		],
		["swap", "a queued ask leaves the queue by its own packet"],
		["pair", CS, '\t\tthis.settle(nonce, failed("timeout"));\n\t}', "\t}"],
		["pair charge", CS, "\t\t\tthis.peer.queuedBytes -= entry.packet.charge;\n", "\n"],
	],
	[
		"41",
		[
			"guard",
			SS,
			"\t\tif (nonce <= peer.lastRequestNonce)\n\t\t\treturn this.reject(peer, definition, `${definition.name} with a nonce already used`);\n",
			"",
		],
		[
			"swap",
			SS,
			"controlPacket(Opcode.Error, [header.index, nonce], reason)",
			"controlPacket(Opcode.Error, [nonce, header.index], reason)",
		],
		["pair", SS, "\t\tpeer.lastRequestNonce = nonce;\n", "\n"],
	],
	[
		"42",
		["guard", SS, "\t\tif (peer.counted >= L.MAX_ACTIVE_HANDLERS) return refuse(ErrorReason.Overloaded);\n", "\n"],
		[
			"swap",
			S,
			"\t\t\t\t\tdefinition.index!,\n\t\t\t\t\tnonce,\n",
			"\t\t\t\t\tnonce,\n\t\t\t\t\tdefinition.index!,\n",
		],
		[
			"pair",
			SS,
			"\t\tif (receiver === undefined) return refuse(ErrorReason.Unavailable);\n\t\tif (peer.counted >= L.MAX_ACTIVE_HANDLERS) return refuse(ErrorReason.Overloaded);\n\t\tthis.start(peer, receiver, [player, value], { definition, nonce });",
			"\t\tif (peer.counted >= L.MAX_ACTIVE_HANDLERS) return refuse(ErrorReason.Overloaded);\n\t\tthis.start(peer, receiver!, [player, value], { definition, nonce });",
		],
	],
	[
		"43",
		["guard", CS, "\t\tif (entry.definition !== definition)\n", "\t\tif (false as boolean)\n"],
		["swap", "a reply settles one nonce against one definition"],
		[
			"pair",
			CS,
			"\t\tif (seen.has(definition)) return;\n\t\tseen.add(definition);",
			"\t\tif (seen.has(definition)) return;",
		],
	],
	[
		"44",
		[
			"guard",
			CS,
			'\t\tif (coroutine.status(entry.thread) === "suspended") this.port.resume',
			"\t\tthis.port.resume",
		],
		["swap", "settle takes one nonce and one result"],
		["pair", CS, "\t\tthis.pending.delete(nonce);\n\t\tentry.deadline?.();", "\t\tentry.deadline?.();"],
	],
	[
		"45",
		[
			"guard",
			S,
			"const replies = request !== undefined && !entry.settled && peer.state !== PeerState.Ended;",
			"const replies = request !== undefined && !entry.settled;",
		],
		["swap", "networking cancels nothing, so no call takes two threads"],
		["pair", "no cancel exists to pair with a start"],
	],
	[
		"46",
		["guard", CS, "\t\tif (this.peer.state === PeerState.Waiting) {", "\t\tif (true as boolean) {"],
		["swap", "READY carries no fields"],
		[
			"pair",
			CS,
			"\t\t\tthis.peer.state = PeerState.Ready;\n\t\t\tthis.flush(this.peer);",
			"\t\t\tthis.peer.state = PeerState.Ready;",
		],
	],
	[
		"47",
		[
			"guard",
			SS,
			'\t\t\tif (peer.state !== PeerState.Waiting) return this.reject(peer, "envelope", "a second READY");\n',
			"",
		],
		["guard client", CS, '\t\tif (!this.fired) return this.violation("a game packet before READY");\n', ""],
		["swap", "the phase checks compare one state with one constant"],
		[
			"pair",
			SS,
			'\t\tif (peer.state !== PeerState.Ready) return this.reject(peer, "envelope", "a game packet before READY");\n',
			"",
		],
	],
	[
		"48",
		["guard", "order comes from one FIFO queue and one remote"],
		["swap", S, "\t\tpeer.queue.push(packet);", "\t\tpeer.queue.unshift(packet);"],
		["pair", "the queue drains in the order it filled"],
	],
	[
		"49",
		["guard", SS, "\t\t\tif (this.reportedNoPeer) return;\n", ""],
		["swap", "a peer is keyed by one engine Player"],
		["pair", SS, "\t\tfor (const player of this.port.players()) this.add(player);\n", "\n"],
		[
			"guard duplicate",
			SS,
			"if (this.stage === Stage.Closed || this.peers.has(player)) return;",
			"if (this.stage === Stage.Closed) return;",
		],
	],
	[
		"50",
		["guard", S, "peer.queue.size() >= L.MAX_QUEUED_PACKETS", "peer.queue.size() > L.MAX_QUEUED_PACKETS"],
		[
			"swap",
			E,
			"charge: buffer.len(bytes) + L.QUEUED_SLOT_CHARGE_BYTES * slots",
			"charge: slots + L.QUEUED_SLOT_CHARGE_BYTES * buffer.len(bytes)",
		],
		["pair", S, "\t\tpeer.queuedBytes += packet.charge;\n", "\n"],
	],
	[
		"51",
		[
			"guard",
			SS,
			"\t\t\tif (!decoded) {\n\t\t\t\tif (!missingInstance(value)) this.reject(peer, definition, definition.name, failureReason(value));\n\t\t\t\treturn;\n\t\t\t}\n",
			"",
		],
		["swap", "decode takes one codec and one reader"],
		["pair", "decode completes before the one start that follows it"],
	],
	[
		"52",
		["guard", "a callback thread has one owner, set once"],
		["swap", "own takes a thread and an ObjectState"],
		["pair", S, "\t\town(thread, claim.owner);\n", "\n"],
		["pair disown", S, "\t\t\tdisown(coroutine.running());\n", "\n"],
	],
	[
		"53",
		[
			"guard",
			S,
			"\t\t\tif (value !== undefined) failure = `returned a ${typeOf(value)}; event callbacks return nothing`;\n",
			"\n",
		],
		[
			"swap",
			S,
			"controlPacket(Opcode.Error, [request.definition.index!, request.nonce], ErrorReason.HandlerError)",
			"controlPacket(Opcode.Error, [request.nonce, request.definition.index!], ErrorReason.HandlerError)",
		],
		["pair", S, "} else if (isPromise(value)) failure", "} else if (false as boolean) failure"],
	],
	[
		"54",
		[
			"guard",
			SS,
			"\t\t\tif (peer.counted >= L.MAX_ACTIVE_HANDLERS) return this.fail(peer, FailReason.Overloaded);\n",
			"\n",
		],
		["swap", "the ceiling compares one count with one constant"],
		["pair", S, "\t\tpeer.counted += 1;\n", "\n"],
		[
			"guard client",
			CS,
			"\t\tif (this.peer.counted >= L.MAX_ACTIVE_HANDLERS) return this.fail(this.peer, FailReason.Overloaded);\n",
			"\n",
		],
	],
	[
		"55",
		["guard", S, "\t\tif (!entry.done && peer.state !== PeerState.Ended)\n", "\t\tif (true as boolean)\n"],
		["swap", "the lifetime timer takes one duration and one callback"],
		["guard once", S, "\t\tif (!entry.counted) return;\n", "\n"],
		[
			"pair",
			S,
			"\t\tthis.reportCallback(peer, entry.claim, entry, `still running after ${L.HANDLER_LIFETIME_SECONDS} seconds`);\n\t\tthis.uncount(peer, entry);",
			"\t\tthis.reportCallback(peer, entry.claim, entry, `still running after ${L.HANDLER_LIFETIME_SECONDS} seconds`);",
		],
	],
	[
		"56",
		[
			"guard",
			SS,
			"\t\t\tif (receiver === undefined) return this.unreceived(definition);\n\t\t\tif (peer.counted >= L.MAX_ACTIVE_HANDLERS) return this.fail(peer, FailReason.Overloaded);\n\t\t\treturn this.start(peer, receiver, [player, value]);",
			"\t\t\tif (peer.counted >= L.MAX_ACTIVE_HANDLERS) return this.fail(peer, FailReason.Overloaded);\n\t\t\treturn this.start(peer, receiver!, [player, value]);",
		],
		["swap", "the diagnostic names one message"],
		["pair", S, "\t\tthis.noReceiver.add(definition);\n", "\n"],
	],
	[
		"74",
		["guard", SS, "\t\tif (peer.rejected.has(key)) return;\n", "\n"],
		[
			"swap",
			SS,
			"this.reject(peer, definition, definition.name, failureReason(value));\n\t\t\t\treturn;",
			"this.reject(peer, definition, failureReason(value), definition.name);\n\t\t\t\treturn;",
		],
		["pair", SS, "\t\tpeer.rejected.add(key);\n", "\n"],
		["guard metatable", E, "\tif (getmetatable(sidecar) !== undefined) return undefined;\n", "\n"],
	],
	[
		"75",
		["guard", S, "\t\t\tif (entry.reported) return;\n", "\n"],
		[
			"swap",
			S,
			"const seen = live ? peer.failedClaims : this.lateFailures;",
			"const seen = live ? this.lateFailures : peer.failedClaims;",
		],
		["pair", S, "\t\t\tentry.reported = true;\n", "\n"],
	],
	[
		"76",
		[
			"guard",
			S,
			"\t\tfor (const entry of peer.running) {\n\t\t\tentry.lifetime?.();",
			"\t\tfor (const entry of peer.running) {\n\t\t\tif (entry === undefined) continue;",
		],
		["swap", "ending takes one peer"],
		["pair", SS, "\t\tif (this.peers.get(player) === peer) this.peers.delete(player);\n", "\n"],
		["pair order", SS, "\t\tif (index !== -1) this.order.remove(index);\n", "\n"],
		[
			"guard client ended",
			CS,
			"\t\tif (this.peer.state === PeerState.Ended) return;\n\t\tconst header",
			"\t\tconst header",
		],
		[
			"memory flush",
			"flush's queue clear and byte reset only release what a ready peer never reads again; end releases both",
		],
		[
			"memory end",
			"end's cleanup, rejected-key and failed-claim clears empty an unreachable server peer; the client peer has no player, so it never stores a cleanup or reads either set",
		],
	],
	[
		"77",
		[
			"guard",
			SS,
			"\t\t\tthis.end(peer);\n\t\t\tpcall(() => this.port.kick(player, L.KICK_MESSAGE));\n\t\t\treturn;",
			"\t\t\tpcall(() => this.port.kick(player, L.KICK_MESSAGE));\n\t\t\treturn;",
		],
		["swap", "a kick takes one Player and one message"],
		["pair", CS, "\t\tpcall(() => this.fire(peer, controlPacket(Opcode.Fail, [], reason)));\n", "\n"],
		[
			"order",
			SS,
			"\t\tpcall(() => this.fire(peer, packet));\n\t\tthis.end(peer);\n\t\tpcall(() => this.port.kick(player, L.KICK_MESSAGE));",
			"\t\tpcall(() => this.port.kick(player, L.KICK_MESSAGE));\n\t\tpcall(() => this.fire(peer, packet));\n\t\tthis.end(peer);",
		],
		["guard nonce 0", E, 'assert(nonce > 0, "nonce 0");', ""],
		["guard ERROR nonce 0", E, 'assert(nonce > 0, "nonce 0");\n\t\theader =', "header ="],
		[
			"guard catalog size",
			CS,
			'\t\tif (header.index >= catalogSize()) return this.violation("an unknown message index");\n',
			"\n",
		],
		[
			"guard reply kind",
			CS,
			'definition === undefined || definition.kind !== "ask-server"',
			"definition === undefined",
		],
	],
	[
		"78",
		[
			"guard",
			SS,
			"\t\t\tif (this.published) pcall(() => this.fire(peer, closing));",
			"\t\t\tif (false as boolean) pcall(() => this.fire(peer, closing));",
		],
		["swap", "close takes no arguments"],
		[
			"pair",
			B,
			"\t\t\t\t\tshutdown(port, singletons);\n\t\t\t\t\tside.network.close();",
			"\t\t\t\t\tshutdown(port, singletons);",
		],
		["pair end", SS, "\t\t\tpcall(() => this.end(peer));\n", "\n"],
		["guard committed indices", SS, "\t\tif (!committed) discardIndices();", "\t\tdiscardIndices();"],
		["guard committed claims", SS, "\t\tcloseClaims(!committed);", "\t\tcloseClaims(true);"],
		[
			"guard server close twice (A17 K01)",
			SS,
			"\tclose() {\n\t\tif (this.stage === Stage.Closed) return;\n\t\tconst committed",
			"\tclose() {\n\t\tconst committed",
		],
	],
	[
		"79",
		["guard", B, "const [undone, undoErr] = pcall(undo);", "const [undone, undoErr] = pcall(() => {});"],
		[
			"swap",
			B,
			'\t\t\tunbooted = side.booted();\n\t\t\tif (side.name === "client") sealDeclarations();\n\t\t\tside.network.commit();',
			'\t\t\tif (side.name === "client") sealDeclarations();\n\t\t\tside.network.commit();\n\t\t\tunbooted = side.booted();',
		],
		["pair", B, "\tif (ok) return;\n\tside.network.close();", "\tif (ok) return;"],
		["pair client end", CS, "\t\tpcall(() => this.end(this.peer));\n", "\n"],
		["guard client committed indices", CS, "\t\tif (!committed) discardIndices();", "\t\tdiscardIndices();"],
		["guard client committed claims", CS, "\t\tcloseClaims(!committed);", "\t\tcloseClaims(true);"],
		["guard client close twice", CS, "\t\tif (this.stage === Stage.Closed) return;\n", ""],
		[
			"guard undo protected (A17 B10)",
			B,
			"const [undone, undoErr] = pcall(undo);",
			"undo();\n\t\t\t\tconst [undone, undoErr] = [true as boolean, undefined as unknown];",
		],
	],
	[
		"80",
		["guard", B, '\t\t\t\t\tif (stage !== "committed") return;\n', "\n"],
		[
			"swap",
			B,
			"\t\tload(options.load);\n\t\tside.network.loaded();",
			"\t\tside.network.loaded();\n\t\tload(options.load);",
		],
		[
			"pair",
			CI,
			"\t\t\t\treturn () => player.SetAttribute(BOOTED_ATTRIBUTE, undefined);",
			"\t\t\t\treturn undefined;",
		],
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

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-session-mutants-"));
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
		const failure = (tested.stdout + tested.stderr).match(/FAIL ((?:network|boot)[^\n]*)/);
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
