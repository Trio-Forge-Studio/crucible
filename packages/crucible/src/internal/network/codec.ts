import * as L from "./limits";
import { type Node, n, runtimeKind } from "./schema";

export interface CodecFailure {
	readonly codecFailure: true;
	readonly path: string;
	readonly reason: string;
	readonly missing?: true;
}

function fail(path: string, reason: string, missing?: true): never {
	error({ codecFailure: true, path, reason, missing });
}
function finite(value: unknown, path: string): number {
	if (!typeIs(value, "number") || value !== value || math.abs(value) === math.huge)
		fail(path, "expected finite number");
	return value;
}
function uint(value: number, maximum: number) {
	assert(value >= 0 && value <= maximum && value % 1 === 0, "integer out of range");
}
function validString(value: string) {
	assert(value.size() <= L.MAX_STRING_BYTES, "string too long");
	const bytes = buffer.fromstring(value);
	let offset = 0;
	while (offset < buffer.len(bytes)) {
		const lead = buffer.readu8(bytes, offset++);
		if (lead < 0x80) continue;
		const count =
			lead >= 0xc2 && lead <= 0xdf ? 1 : lead >= 0xe0 && lead <= 0xef ? 2 : lead >= 0xf0 && lead <= 0xf4 ? 3 : 0;
		assert(count > 0 && count <= buffer.len(bytes) - offset, "invalid UTF-8");
		const second = buffer.readu8(bytes, offset);
		assert(lead !== 0xe0 || second >= 0xa0, "invalid UTF-8");
		assert(lead !== 0xed || second < 0xa0, "invalid UTF-8");
		assert(lead !== 0xf0 || second >= 0x90, "invalid UTF-8");
		assert(lead !== 0xf4 || second < 0x90, "invalid UTF-8");
		for (let i = 0; i < count; i++) {
			const byte = buffer.readu8(bytes, offset++);
			assert(byte >= 0x80 && byte <= 0xbf, "invalid UTF-8");
		}
	}
}

export class Budget {
	visits = 0;
	entries = 0;
	depth = 0;
	chargeEntries(count: number, path: string) {
		if (count < 0 || count % 1 !== 0 || count > L.MAX_COLLECTION_ENTRIES - this.entries)
			fail(path, "entry budget exhausted");
		this.entries += count;
	}
	visit(path: string) {
		if (++this.visits > L.MAX_VALUE_VISITS) fail(path, "visit budget exhausted");
	}
	enter(path: string) {
		if (++this.depth > L.MAX_DEPTH) fail(path, "depth budget exhausted");
	}
	leave() {
		this.depth--;
	}
}

export class Writer {
	readonly budget = new Budget();
	readonly slots = new Array<Instance>();
	private data = buffer.create(64);
	private cursor = 0;
	private reserve(size: number): number {
		assert(size <= L.MAX_PACKET_BYTES - this.cursor, "packet too large");
		if (size > buffer.len(this.data) - this.cursor) {
			const capacity = math.min(L.MAX_PACKET_BYTES, math.max(buffer.len(this.data) * 2, this.cursor + size));
			const grown = buffer.create(capacity);
			buffer.copy(grown, 0, this.data, 0, this.cursor);
			this.data = grown;
		}
		const offset = this.cursor;
		this.cursor += size;
		return offset;
	}
	u8(value: number) {
		uint(value, 255);
		const offset = this.reserve(1);
		buffer.writeu8(this.data, offset, value);
	}
	u16(value: number) {
		uint(value, 65535);
		const offset = this.reserve(2);
		buffer.writeu16(this.data, offset, value);
	}
	u32(value: number) {
		uint(value, 4294967295);
		const offset = this.reserve(4);
		buffer.writeu32(this.data, offset, value);
	}
	varuint(value: number) {
		uint(value, 4294967295);
		do {
			const low = value % 128;
			value = math.floor(value / 128);
			this.u8(low + (value > 0 ? 128 : 0));
		} while (value > 0);
	}
	f32(value: number) {
		finite(value, "payload");
		const offset = this.reserve(4);
		buffer.writef32(this.data, offset, value);
		finite(buffer.readf32(this.data, offset), "payload");
	}
	f64(value: number) {
		finite(value, "payload");
		const offset = this.reserve(8);
		buffer.writef64(this.data, offset, value);
	}
	bytes(text: string) {
		validString(text);
		this.varuint(text.size());
		const offset = this.reserve(text.size());
		buffer.writestring(this.data, offset, text);
	}
	slot(instance: Instance) {
		assert(this.slots.size() < L.MAX_INSTANCE_SLOTS, "slot budget exhausted");
		this.slots.push(instance);
	}
	finish(header: buffer): LuaTuple<[buffer, Readonly<Record<string, Instance>>, number]> {
		const size = buffer.len(header) + this.cursor;
		assert(size <= L.MAX_PACKET_BYTES, "packet too large");
		const packet = buffer.create(size);
		buffer.copy(packet, 0, header);
		buffer.copy(packet, buffer.len(header), this.data, 0, this.cursor);
		const sidecar: Record<string, Instance> = {};
		for (let i = 0; i < this.slots.size(); i++) sidecar[tostring(i + 1)] = this.slots[i];
		return $tuple(packet, table.freeze(sidecar), this.slots.size());
	}
}

export class Reader {
	readonly budget = new Budget();
	private nextSlot = 1;
	constructor(
		private readonly packet: buffer,
		private cursor: number,
		private readonly sidecar: Readonly<Record<string, Instance>>,
		private readonly slotCount: number,
	) {
		assert(typeIs(packet, "buffer") && buffer.len(packet) <= L.MAX_PACKET_BYTES, "invalid packet");
		uint(cursor, buffer.len(packet));
		uint(slotCount, L.MAX_INSTANCE_SLOTS);
		assert(typeIs(sidecar, "table") && getmetatable(sidecar) === undefined, "invalid sidecar");
		let entries = 0;
		for (const [key, value] of pairs(sidecar)) {
			assert(++entries <= L.MAX_INSTANCE_SLOTS, "sidecar too large");
			const slot = typeIs(key, "string") ? tonumber(key) : undefined;
			assert(
				slot !== undefined &&
					slot >= 1 &&
					slot <= slotCount &&
					slot % 1 === 0 &&
					tostring(slot) === key &&
					typeIs(value, "Instance"),
				"invalid sidecar slot",
			);
		}
	}
	private take(size: number): number {
		assert(size <= buffer.len(this.packet) - this.cursor, "truncated packet");
		const offset = this.cursor;
		this.cursor += size;
		return offset;
	}
	u8() {
		return buffer.readu8(this.packet, this.take(1));
	}
	u16() {
		return buffer.readu16(this.packet, this.take(2));
	}
	u32() {
		return buffer.readu32(this.packet, this.take(4));
	}
	varuint(): number {
		let value = 0;
		for (let i = 0; i < L.VARUINT_MAX_BYTES; i++) {
			const byte = this.u8();
			assert(i < 4 || byte <= 15, "varuint overflow");
			value += (byte % 128) * 2 ** (7 * i);
			if (byte < 128) {
				assert(i === 0 || byte !== 0, "non-minimal varuint");
				return value;
			}
		}
		error("varuint overflow");
	}
	f32() {
		return finite(buffer.readf32(this.packet, this.take(4)), "payload");
	}
	f64() {
		return finite(buffer.readf64(this.packet, this.take(8)), "payload");
	}
	string(): string {
		const size = this.varuint();
		assert(size <= L.MAX_STRING_BYTES, "string too long");
		const text = buffer.readstring(this.packet, this.take(size), size);
		validString(text);
		return text;
	}
	count(minElementBytes: number, path: string): number {
		const count = this.varuint();
		this.budget.chargeEntries(count, path);
		assert(
			minElementBytes === 0 || count <= math.floor((buffer.len(this.packet) - this.cursor) / minElementBytes),
			"count exceeds remaining bytes",
		);
		return count;
	}
	slot(path: string): Instance | undefined {
		if (this.nextSlot > this.slotCount) fail(path, "slot count exceeded");
		return this.sidecar[tostring(this.nextSlot++)];
	}
	finished(): boolean {
		return this.cursor === buffer.len(this.packet) && this.nextSlot === this.slotCount + 1;
	}
}

export interface Codec {
	readonly node: Node;
	readonly minBytes: number;
	readonly encode: (writer: Writer, value: unknown, path: string) => void;
	readonly decode: (reader: Reader, path: string) => unknown;
}

function tableValue(value: unknown, path: string): Record<string, unknown> {
	if (!typeIs(value, "table")) fail(path, "expected table");
	return value as Record<string, unknown>;
}
function dense(value: unknown, path: string): unknown[] {
	tableValue(value, path);
	const array = value as unknown[];
	let count = 0;
	for (const [key] of pairs(array)) {
		count++;
		if (!typeIs(key, "number") || key % 1 !== 0 || key < 1 || key > array.size())
			fail(path, "expected dense array");
	}
	if (count !== array.size()) fail(path, "expected dense array");
	return array;
}

function frame(values: readonly number[], path: string) {
	for (const v of values) finite(v, path);
	const r = values.filter((_, i) => i >= 3);
	const dot = (a: number, b: number) => r[a] * r[b] + r[a + 1] * r[b + 1] + r[a + 2] * r[b + 2];
	for (const i of [0, 3, 6]) if (math.abs(dot(i, i) - 1) > L.CFRAME_TOLERANCE) fail(path, "invalid CFrame rows");
	if (
		math.abs(dot(0, 3)) > L.CFRAME_TOLERANCE ||
		math.abs(dot(0, 6)) > L.CFRAME_TOLERANCE ||
		math.abs(dot(3, 6)) > L.CFRAME_TOLERANCE
	)
		fail(path, "invalid CFrame orthogonality");
	const det =
		r[0] * (r[4] * r[8] - r[5] * r[7]) - r[1] * (r[3] * r[8] - r[5] * r[6]) + r[2] * (r[3] * r[7] - r[4] * r[6]);
	if (math.abs(det - 1) > L.CFRAME_TOLERANCE) fail(path, "invalid CFrame determinant");
}

export function compile(node: Node, tally = { nodes: 0 }): Codec {
	const active = new Set<Node>();
	function build(node: Node, depth: number, empty = false, nilable = false): Codec {
		assert(
			++tally.nodes <= L.MAX_SCHEMA_NODES && depth <= L.MAX_DEPTH && !active.has(node),
			"schema budget exhausted",
		);
		active.add(node);
		let minBytes = 0;
		let encode: Codec["encode"];
		let decode: Codec["decode"];
		switch (node.kind) {
			case "nil":
				encode = (_, v, p) => {
					if (v !== undefined) fail(p, "expected undefined");
				};
				decode = () => undefined;
				break;
			case "literal":
				encode = (_, v, p) => {
					if (v !== node.value) fail(p, "expected literal");
				};
				decode = () => node.value;
				break;
			case "bool":
				minBytes = 1;
				encode = (w, v, p) => {
					if (!typeIs(v, "boolean")) fail(p, "expected boolean");
					w.u8(v ? 1 : 0);
				};
				decode = (r, p) => {
					const b = r.u8();
					if (b > 1) fail(p, "invalid boolean");
					return b === 1;
				};
				break;
			case "u8":
			case "u16":
			case "u32":
			case "f32":
			case "f64": {
				const kind = node.kind;
				minBytes = kind === "u8" ? 1 : kind === "u16" ? 2 : kind === "f64" ? 8 : 4;
				encode = (w, v, p) => {
					const number = finite(v, p);
					if (kind === "u8") w.u8(number);
					else if (kind === "u16") w.u16(number);
					else if (kind === "u32") w.u32(number);
					else if (kind === "f32") w.f32(number);
					else w.f64(number);
				};
				decode = (r) =>
					kind === "u8"
						? r.u8()
						: kind === "u16"
							? r.u16()
							: kind === "u32"
								? r.u32()
								: kind === "f32"
									? r.f32()
									: r.f64();
				break;
			}
			case "string":
				minBytes = 1;
				encode = (w, v, p) => {
					if (!typeIs(v, "string")) fail(p, "expected string");
					w.bytes(v);
				};
				decode = (r) => r.string();
				break;
			case "vector3":
			case "color3":
			case "cframe": {
				const kind = node.kind;
				minBytes = kind === "cframe" ? 96 : 24;
				encode = (w, v, p) => {
					let values: number[];
					if (kind === "vector3" && typeIs(v, "Vector3")) values = [v.X, v.Y, v.Z];
					else if (kind === "color3" && typeIs(v, "Color3")) values = [v.R, v.G, v.B];
					else if (kind === "cframe" && typeIs(v, "CFrame")) {
						values = [...v.GetComponents()];
						frame(values, p);
					} else return fail(p, "expected datatype");
					for (const component of values) w.f64(component);
				};
				decode = (r, p) => {
					const values = new Array<number>();
					for (let i = 0; i < minBytes / 8; i++) values.push(r.f64());
					if (kind === "vector3") {
						const result = new Vector3(values[0], values[1], values[2]);
						for (const value of [result.X, result.Y, result.Z]) finite(value, p);
						return result;
					}
					if (kind === "color3") {
						const result = new Color3(values[0], values[1], values[2]);
						for (const value of [result.R, result.G, result.B]) finite(value, p);
						return result;
					}
					frame(values, p);
					const result = new CFrame(
						values[0],
						values[1],
						values[2],
						values[3],
						values[4],
						values[5],
						values[6],
						values[7],
						values[8],
						values[9],
						values[10],
						values[11],
					);
					for (const value of result.GetComponents()) finite(value, p);
					return result;
				};
				break;
			}
			case "instance": {
				const check = (v: unknown, p: string): Instance => {
					if (!typeIs(v, "Instance") || !node.classes.some((name) => v.IsA(name)))
						fail(p, "expected declared Instance class");
					return v;
				};
				encode = (w, v, p) => w.slot(check(v, p));
				decode = (r, p) => {
					const value = r.slot(p);
					if (value === undefined) {
						if (nilable) return undefined;
						return fail(p, "missing Instance", true);
					}
					return check(value, p);
				};
				break;
			}
			case "object": {
				assert(empty || node.fields.size() > 0, "empty object type");
				const fields = node.fields.map(([name, child]) => [name, build(child, depth + 1)] as const);
				for (const [, codec] of fields) minBytes += codec.minBytes;
				encode = (w, v, p) => {
					const object = tableValue(v, p);
					for (const [name, c] of fields) c.encode(w, object[name], `${p}.${name}`);
				};
				decode = (r, p) => {
					const object: Record<string, unknown> = {};
					for (const [name, c] of fields) object[name] = c.decode(r, `${p}.${name}`);
					return object;
				};
				break;
			}
			case "variant": {
				assert(empty, "variant outside union");
				const body = build(node.body, depth + 1, true);
				minBytes = body.minBytes;
				encode = (w, v, p) => {
					if (tableValue(v, p)[node.key] !== node.value) fail(p, "invalid discriminant");
					body.encode(w, v, p);
				};
				decode = (r, p) => {
					const v = body.decode(r, p) as Record<string, unknown>;
					v[node.key] = node.value;
					return v;
				};
				break;
			}
			case "union": {
				n.union(node.branches);
				const branches = node.branches.map((b) =>
					build(
						b,
						depth + 1,
						b.kind === "variant",
						node.branches.some((b) => b.kind === "nil"),
					),
				);
				minBytes = 1 + math.min(...branches.map((c) => c.minBytes));
				const choose = (v: unknown, p: string): number => {
					for (let i = 0; i < node.branches.size(); i++) {
						const b = node.branches[i];
						if ((b.kind === "nil" && v === undefined) || (b.kind === "literal" && v === b.value)) return i;
					}
					if (v !== undefined)
						for (let i = 0; i < node.branches.size(); i++) {
							const b = node.branches[i];
							if (b.kind === "variant") {
								if (typeIs(v, "table") && (v as Record<string, unknown>)[b.key] === b.value) return i;
							} else if (b.kind !== "nil" && b.kind !== "literal" && runtimeKind(b) === typeOf(v))
								return i;
						}
					return fail(p, "no union branch");
				};
				encode = (w, v, p) => {
					w.budget.visit(p);
					const tag = choose(v, p);
					w.varuint(tag);
					branches[tag].encode(w, v, p);
				};
				decode = (r, p) => {
					r.budget.visit(p);
					const tag = r.varuint();
					if (tag >= branches.size()) fail(p, "bad union tag");
					return branches[tag].decode(r, p);
				};
				break;
			}
			case "array":
			case "tuple": {
				const array = node.kind === "array";
				const children = array
					? [build(node.element, depth + 1)]
					: node.elements.map((b) => build(b, depth + 1));
				minBytes = array ? 1 : children.reduce((sum, c) => sum + c.minBytes, 0);
				encode = (w, v, p) => {
					const values = dense(v, p);
					if (!array && values.size() !== children.size()) fail(p, "tuple length");
					if (array) {
						w.budget.chargeEntries(values.size(), p);
						w.varuint(values.size());
					}
					for (let i = 0; i < values.size(); i++) {
						if (values[i] === undefined) fail(p, "nil element");
						children[array ? 0 : i].encode(w, values[i], `${p}[]`);
					}
				};
				decode = (r, p) => {
					const count = array ? r.count(children[0].minBytes, p) : children.size();
					const values = new Array<defined>();
					for (let i = 0; i < count; i++) {
						const value = children[array ? 0 : i].decode(r, `${p}[]`);
						if (value === undefined) fail(p, "nil element");
						values.push(value as defined);
					}
					return values;
				};
				break;
			}
			case "map": {
				const key = build(node.key, depth + 1);
				const value = build(node.value, depth + 1);
				minBytes = 1;
				const order = (v: unknown): string | number => {
					if (node.key.kind === "union") {
						const index = node.key.branches.findIndex((b) => b.kind === "literal" && b.value === v);
						assert(index >= 0, "invalid map key");
						return index;
					}
					return v as string | number;
				};
				const less = (a: unknown, b: unknown) => {
					const x = order(a),
						y = order(b);
					return typeIs(x, "number") && typeIs(y, "number") ? x < y : (x as string) < (y as string);
				};
				encode = (w, v, p) => {
					const entries = new Array<readonly [unknown, unknown]>();
					for (const [k, val] of pairs(tableValue(v, p))) entries.push([k, val]);
					w.budget.chargeEntries(entries.size(), p);
					entries.sort((a, b) => less(a[0], b[0]));
					w.varuint(entries.size());
					for (const [k, val] of entries) {
						key.encode(w, k, `${p}.key`);
						if (val === undefined) fail(p, "nil map value");
						value.encode(w, val, `${p}[]`);
					}
				};
				decode = (r, p) => {
					const count = r.count(key.minBytes + value.minBytes, p);
					const values = new Map<unknown, unknown>();
					let last: unknown;
					for (let i = 0; i < count; i++) {
						const k = key.decode(r, `${p}.key`);
						if (i > 0 && !less(last, k)) fail(p, "map keys not strictly ascending");
						last = k;
						const v = value.decode(r, `${p}[]`);
						if (v === undefined) fail(p, "nil map value");
						values.set(k, v);
					}
					return values;
				};
				break;
			}
			case "dynamic":
				minBytes = 1;
				encode = (w, v, p) => encodeDynamic(w, v, p, new Set<object>());
				decode = (r, p) => decodeDynamic(r, p, true);
				break;
			default:
				error("unknown descriptor kind");
		}
		active.delete(node);
		const container =
			node.kind === "object" || node.kind === "array" || node.kind === "tuple" || node.kind === "map";
		return {
			node,
			minBytes,
			encode: (w, v, p) => {
				if (node.kind !== "union" && node.kind !== "dynamic") w.budget.visit(p);
				if (container) w.budget.enter(p);
				const [ok, reason] = pcall(() => encode(w, v, p));
				if (!ok) {
					if (typeIs(reason, "table") && (reason as Partial<CodecFailure>).codecFailure === true)
						error(reason);
					fail(p, tostring(reason));
				}
				if (container) w.budget.leave();
			},
			decode: (r, p) => {
				if (node.kind !== "union" && node.kind !== "dynamic") r.budget.visit(p);
				if (container) r.budget.enter(p);
				const value = decode(r, p);
				if (container) r.budget.leave();
				return value;
			},
		};
	}
	return build(node, 1);
}

const datatypes = { vector3: compile(n.vector3), color3: compile(n.color3), cframe: compile(n.cframe) };

export function encodeDynamic(
	w: Writer,
	value: unknown,
	path: string,
	active: Set<object>,
	topLevel = true,
	arrayElement = false,
): void {
	const datatype = typeIs(value, "Vector3") || typeIs(value, "Color3") || typeIs(value, "CFrame");
	if (!datatype) w.budget.visit(path);
	if (value === undefined) {
		if (!topLevel) fail(path, "nil nested dynamic value");
		w.u8(0);
	} else if (typeIs(value, "boolean")) w.u8(value ? 2 : 1);
	else if (typeIs(value, "number")) {
		w.u8(3);
		w.f64(value);
	} else if (typeIs(value, "string")) {
		w.u8(4);
		w.bytes(value);
	} else if (typeIs(value, "Instance")) {
		if (arrayElement) fail(path, "Instance array element");
		w.u8(10);
		w.slot(value);
	} else if (typeIs(value, "Vector3") || typeIs(value, "Color3") || typeIs(value, "CFrame")) {
		const kind = typeIs(value, "Vector3") ? "vector3" : typeIs(value, "Color3") ? "color3" : "cframe";
		w.u8(kind === "vector3" ? 7 : kind === "color3" ? 8 : 9);
		datatypes[kind].encode(w, value, path);
	} else {
		const object = tableValue(value, path);
		if (getmetatable(object) !== undefined) fail(path, "dynamic metatable");
		if (active.has(object)) fail(path, "cycle");
		active.add(object);
		w.budget.enter(path);
		const keys = new Array<string>();
		let numeric = 0;
		for (const [k] of pairs(object)) {
			if (typeIs(k, "number")) numeric++;
			else if (typeIs(k, "string")) keys.push(k);
			else fail(path, "invalid dynamic key");
		}
		if (numeric > 0) {
			if (!keys.isEmpty()) fail(path, "mixed dynamic table");
			const values = dense(value, path);
			w.budget.chargeEntries(values.size(), path);
			w.u8(5);
			w.varuint(values.size());
			for (const v of values) encodeDynamic(w, v, `${path}[]`, active, false, true);
		} else {
			keys.sort();
			w.budget.chargeEntries(keys.size(), path);
			w.u8(6);
			w.varuint(keys.size());
			for (const key of keys) {
				w.budget.visit(path);
				w.bytes(key);
				encodeDynamic(w, object[key], `${path}[]`, active, false);
			}
		}
		w.budget.leave();
		active.delete(object);
	}
}

export function decodeDynamic(r: Reader, path: string, topLevel: boolean, arrayElement = false): unknown {
	const tag = r.u8();
	if (tag >= 7 && tag <= 9) return datatypes[tag === 7 ? "vector3" : tag === 8 ? "color3" : "cframe"].decode(r, path);
	r.budget.visit(path);
	if (tag === 0) {
		if (!topLevel) fail(path, "nil nested dynamic value");
		return undefined;
	}
	if (tag === 1 || tag === 2) return tag === 2;
	if (tag === 3) return r.f64();
	if (tag === 4) return r.string();
	if (tag === 10) {
		if (arrayElement) fail(path, "Instance array element");
		return r.slot(path);
	}
	if (tag !== 5 && tag !== 6) return fail(path, "bad dynamic tag");
	r.budget.enter(path);
	const count = r.count(tag === 5 ? 1 : 2, path);
	const values: Record<string, unknown> = {};
	const array = new Array<defined>();
	let last: string | undefined;
	for (let i = 0; i < count; i++) {
		if (tag === 5) array.push(decodeDynamic(r, `${path}[]`, false, true) as defined);
		else {
			r.budget.visit(path);
			const key = r.string();
			if (last !== undefined && key <= last) fail(path, "record keys not strictly ascending");
			last = key;
			values[key] = decodeDynamic(r, `${path}[]`, false);
		}
	}
	r.budget.leave();
	return tag === 5 ? array : values;
}
