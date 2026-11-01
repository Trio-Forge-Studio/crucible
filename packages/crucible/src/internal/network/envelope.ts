import { type Codec, type CodecFailure, Reader, Writer } from "./codec";
import * as L from "./limits";

/** @internal */
export const enum Opcode {
	Event = 0,
	Request = 1,
	Ok = 2,
	Error = 3,
	Ready = 4,
	Fail = 5,
}

/** @internal */
export const enum ErrorReason {
	Unavailable = 0,
	HandlerError = 1,
	Overloaded = 2,
	Invalid = 3,
}

/** @internal */
export const enum FailReason {
	Overflow = 0,
	Overloaded = 1,
	Closing = 2,
	Malformed = 3,
}

/** @internal */
export type Header =
	| { readonly opcode: Opcode.Event; readonly index: number; readonly slots: number; readonly payloadAt: number }
	| {
			readonly opcode: Opcode.Request;
			readonly index: number;
			readonly nonce: number;
			readonly slots: number;
			readonly payloadAt: number;
	  }
	| {
			readonly opcode: Opcode.Ok;
			readonly index: number;
			readonly nonce: number;
			readonly slots: number;
			readonly payloadAt: number;
	  }
	| { readonly opcode: Opcode.Error; readonly index: number; readonly nonce: number; readonly reason: number }
	| { readonly opcode: Opcode.Ready }
	| { readonly opcode: Opcode.Fail; readonly reason: number };

/** @internal One encoded packet, immutable once built; a broadcast shares one across peers. */
export interface Packet {
	readonly bytes: buffer;
	readonly sidecar: Readonly<Record<string, Instance>>;
	/** Buffer length plus QUEUED_SLOT_CHARGE_BYTES per Instance slot (rule 50). */
	readonly charge: number;
}

const EMPTY = table.freeze({}) as Readonly<Record<string, Instance>>;

function varuintSize(value: number): number {
	let size = 1;
	while (value >= 128) {
		value = math.floor(value / 128);
		size++;
	}
	return size;
}

function parse(packet: buffer): Header {
	const reader = new Reader(packet, 0, EMPTY, 0);
	const opcode = reader.u8();
	if (opcode === Opcode.Event) {
		const index = reader.varuint();
		const slots = reader.varuint();
		return { opcode, index, slots, payloadAt: 1 + varuintSize(index) + varuintSize(slots) };
	}
	if (opcode === Opcode.Request || opcode === Opcode.Ok) {
		const index = reader.varuint();
		const nonce = reader.varuint();
		const slots = reader.varuint();
		assert(nonce > 0, "nonce 0");
		const payloadAt = 1 + varuintSize(index) + varuintSize(nonce) + varuintSize(slots);
		return opcode === Opcode.Request
			? { opcode, index, nonce, slots, payloadAt }
			: { opcode, index, nonce, slots, payloadAt };
	}
	let header: Header;
	if (opcode === Opcode.Error) {
		const index = reader.varuint();
		const nonce = reader.varuint();
		assert(nonce > 0, "nonce 0");
		header = { opcode, index, nonce, reason: reader.u8() };
	} else if (opcode === Opcode.Ready) header = { opcode };
	else if (opcode === Opcode.Fail) header = { opcode, reason: reader.u8() };
	else error("unknown opcode");
	assert(reader.finished(), "trailing bytes");
	return header;
}

/**
 * @internal
 * The highest non-nil argument position. roblox-ts lowers `select("#", ...args)` through `unpack`, which stops at a
 * nil hole, so this walks the keys instead. A trailing nil argument carries nothing and stays invisible to it.
 */
export function argumentCount(args: readonly unknown[]): number {
	let count = 0;
	for (const [key] of pairs(args)) if (typeIs(key, "number") && key > count) count = key;
	return count;
}

/**
 * @internal
 * Wire.md decode steps 1 and 2: exactly two remote arguments, a buffer and a plain table, then the header. Returns
 * undefined for any envelope failure. Controls and ERROR carry no slots, so their sidecar must be empty here; a game
 * packet's sidecar is checked against its slot count when its payload decodes (step 4).
 */
export function readHeader(argumentCount: number, packet: unknown, sidecar: unknown): Header | undefined {
	if (argumentCount !== 2 || !typeIs(packet, "buffer") || !typeIs(sidecar, "table")) return undefined;
	if (getmetatable(sidecar) !== undefined) return undefined;
	const [ok, header] = pcall(parse, packet);
	if (!ok) return undefined;
	if (header.opcode !== Opcode.Event && header.opcode !== Opcode.Request && header.opcode !== Opcode.Ok)
		if (next(sidecar)[0] !== undefined) return undefined;
	return header;
}

function headerBytes(opcode: Opcode, fields: readonly number[], reason?: number): buffer {
	const writer = new Writer();
	writer.u8(opcode);
	for (const field of fields) writer.varuint(field);
	if (reason !== undefined) writer.u8(reason);
	return writer.finish(buffer.create(0))[0];
}

/** @internal A packet with no payload: READY, FAIL or ERROR. */
export function controlPacket(opcode: Opcode, fields: readonly number[], reason?: number): Packet {
	const bytes = headerBytes(opcode, fields, reason);
	return table.freeze({ bytes, sidecar: EMPTY, charge: buffer.len(bytes) });
}

/** An assert's message carries the asserting chunk and line, which say nothing about the payload. */
function plain(err: unknown): string {
	return tostring(err).gsub("^[^\n]-:%d+: ", "")[0];
}

function describe(name: string, path: string, err: unknown): string {
	if (typeIs(err, "table") && (err as CodecFailure).codecFailure === true) {
		const failure = err as CodecFailure;
		return `${name} ${failure.path}: ${plain(failure.reason)}`;
	}
	return `${name} ${path}: ${plain(err)}`;
}

/** @internal An encoded payload waiting for its header. */
export interface Body {
	readonly name: string;
	readonly path: string;
	readonly writer: Writer;
}

/**
 * @internal
 * Validates and encodes a payload now, before anything is queued or yields (rules 18, 19). A payload that does not fit
 * its declaration throws at the call site, naming the message, the property path and the reason.
 */
export function encodeBody(name: string, codec: Codec, path: string, value: unknown): Body {
	const writer = new Writer();
	const [ok, err] = pcall(() => codec.encode(writer, value, path));
	if (!ok) error(describe(name, path, err), 0);
	return { name, path, writer };
}

/** @internal Puts the header in front of an encoded payload. `fields` precede the slot count. */
export function frame(body: Body, opcode: Opcode, fields: readonly number[]): Packet {
	const slots = body.writer.slots.size();
	let bytes = buffer.create(0);
	let sidecar = EMPTY;
	const [ok, err] = pcall(() => {
		[bytes, sidecar] = body.writer.finish(headerBytes(opcode, [...fields, slots]));
	});
	if (!ok) error(describe(body.name, body.path, err), 0);
	return table.freeze({ bytes, sidecar, charge: buffer.len(bytes) + L.QUEUED_SLOT_CHARGE_BYTES * slots });
}

/** @internal Decode step 4 for a game packet: the sidecar against the header's slot count. Throws on failure. */
export function openBody(
	packet: buffer,
	sidecar: Readonly<Record<string, Instance>>,
	header: { readonly payloadAt: number; readonly slots: number },
): Reader {
	return new Reader(packet, header.payloadAt, sidecar, header.slots);
}

/** @internal Decode steps 6 and 7: the payload, then nothing left over. Throws on failure. */
export function decodeBody(codec: Codec, path: string, reader: Reader): unknown {
	const value = codec.decode(reader, path);
	assert(reader.finished(), "trailing bytes or unconsumed slots");
	return value;
}

/** @internal True when a decode failed first on a missing Instance (rules 41, 67, 68). */
export function missingInstance(err: unknown): boolean {
	return typeIs(err, "table") && (err as CodecFailure).missing === true;
}

/** @internal The reason a decode failed, for a diagnostic. Never payload content (rule 73). */
export function failureReason(err: unknown): string {
	if (typeIs(err, "table") && (err as CodecFailure).codecFailure === true) {
		const failure = err as CodecFailure;
		return `${failure.path}: ${plain(failure.reason)}`;
	}
	return plain(err);
}
