import { Subsystem } from "../../subsystem";
import { runtimeDiagnostic } from "../diagnostics";
import { isLive, stateOf } from "../lifecycle";
import { admitsNil, assertDescriptor, runtimeKind, type Branch, type Node } from "../network/schema";
import type { Disconnect } from "../port";
import { define, type TypeId } from "../registry";
import * as L from "./limits";
import { configPort, type ConfigSnapshotPort } from "./port";

export type ConfigKind = "global" | "player";

/** One value a reader sees. Commit is its only writer. */
interface Cell {
	value: unknown;
}

export interface Declaration {
	readonly name: string;
	readonly kind: ConfigKind;
	readonly node: Node;
	readonly fallback: unknown;
	/** The value admitted from the global snapshot. */
	readonly global: Cell;
}

const enum Phase {
	Fetching,
	Loaded,
	Gone,
}

interface Snapshot {
	readonly port: ConfigSnapshotPort;
	/** UpdateAvailable events seen. */
	updates: number;
	/** `updates` when the applied values were requested. */
	applied: number;
	readonly disconnect: Disconnect;
}

interface Scope {
	readonly player?: Player;
	phase: Phase;
	snapshot?: Snapshot;
	/** Advances with every snapshot result read, so each result diagnoses each key at most once. */
	generation: number;
	diagnosed: Set<string>;
	readonly ready: Cell;
	readonly values: Map<Declaration, Cell>;
}

interface Staged {
	readonly scope: Scope;
	readonly snapshot: Snapshot;
	readonly first: boolean;
	readonly updates: number;
	readonly values: Map<Declaration, unknown>;
}

const KEEP = table.freeze({});

const declarations = new Array<Declaration>();
const names = new Set<string>();
let sealed = false;

function newScope(player?: Player): Scope {
	return {
		player,
		phase: Phase.Fetching,
		generation: 0,
		diagnosed: new Set(),
		ready: { value: false },
		values: new Map(),
	};
}

const global = newScope();
const slots = new Map<Player, Scope>();
let owner: ConfigRuntime | undefined;
let retired = false;
let listeners = new Array<Disconnect>();
let flight: { readonly waiters: thread[] } | undefined;
let active = 0;
const queue = new Array<thread>();

interface Failure {
	readonly configFailure: true;
	readonly path: string;
	readonly reason: string;
}

function fail(path: string, reason: string): never {
	error({ configFailure: true, path, reason } satisfies Failure);
}

const F32_MAX = 3.4028234663852886e38;
const UINT_MAX = { u8: 255, u16: 65_535, u32: 4_294_967_295 };

function admit(node: Node, value: unknown, root: string): unknown {
	let visits = 0;
	const open = new Set<object>();

	function enter(value: object, path: string, depth: number) {
		if (++visits > L.CONFIG_ADMISSION_VISITS) fail(path, `more than ${L.CONFIG_ADMISSION_VISITS} values`);
		if (depth > L.CONFIG_VALUE_DEPTH) fail(path, `deeper than ${L.CONFIG_VALUE_DEPTH}`);
		if (getmetatable(value) !== undefined) fail(path, "a table with a metatable");
		if (open.has(value)) fail(path, "a cycle");
		open.add(value);
	}

	function finish<T extends object>(source: object, copy: T): T {
		open.delete(source);
		return table.freeze(copy);
	}

	function scalar(value: unknown, path: string): unknown {
		if (++visits > L.CONFIG_ADMISSION_VISITS) fail(path, `more than ${L.CONFIG_ADMISSION_VISITS} values`);
		if (typeIs(value, "number")) {
			if (value !== value || math.abs(value) === math.huge) fail(path, "expected a finite number");
		} else if (typeIs(value, "string")) {
			if (value.size() > L.CONFIG_STRING_MAX_BYTES)
				fail(path, `a string over ${L.CONFIG_STRING_MAX_BYTES} bytes`);
			if (utf8.len(value)[0] === undefined) fail(path, "invalid UTF-8");
		} else if (!typeIs(value, "boolean")) fail(path, `expected JSON, got ${typeOf(value)}`);
		return value;
	}

	function isArray(value: object): boolean {
		const array = value as unknown[];
		let count = 0;
		for (const [key] of pairs(array)) {
			if (!typeIs(key, "number") || key % 1 !== 0 || key < 1 || key > array.size()) return false;
			count++;
		}
		return count === array.size();
	}

	function json(value: unknown, path: string, depth: number): unknown {
		if (!typeIs(value, "table")) return scalar(value, path);
		enter(value, path, depth);
		const copy = new Map<unknown, unknown>();
		if (isArray(value)) {
			const array = value as unknown[];
			for (let i = 0; i < array.size(); i++) copy.set(i + 1, json(array[i], `${path}[${i}]`, depth + 1));
		} else
			for (const [key, child] of pairs(value)) {
				if (!typeIs(key, "string")) fail(path, "a table mixing array and object keys");
				copy.set(key, json(child, `${path}.${key}`, depth + 1));
			}
		return finish(value, copy);
	}

	function tableAt(value: unknown, path: string, depth: number): object {
		if (!typeIs(value, "table")) fail(path, `expected a table, got ${typeOf(value)}`);
		enter(value, path, depth);
		return value;
	}

	function dense(value: unknown, path: string, depth: number): defined[] {
		const t = tableAt(value, path, depth);
		if (!isArray(t)) fail(path, "expected an array");
		return t as defined[];
	}

	function walk(node: Branch, value: unknown, path: string, depth: number): unknown {
		switch (node.kind) {
			case "nil":
				if (value !== undefined) fail(path, "expected nothing");
				return value;
			case "literal":
				if (value !== node.value) fail(path, `expected the literal ${tostring(node.value)}`);
				return scalar(value, path);
			case "bool":
				if (!typeIs(value, "boolean")) fail(path, `expected a boolean, got ${typeOf(value)}`);
				return scalar(value, path);
			case "string":
				if (!typeIs(value, "string")) fail(path, `expected a string, got ${typeOf(value)}`);
				return scalar(value, path);
			case "f64":
			case "f32":
			case "u8":
			case "u16":
			case "u32": {
				if (!typeIs(value, "number")) fail(path, `expected a number, got ${typeOf(value)}`);
				scalar(value, path);
				if (node.kind === "f32" && math.abs(value) > F32_MAX) fail(path, "out of f32 range");
				if (node.kind !== "f64" && node.kind !== "f32") {
					const maximum = UINT_MAX[node.kind];
					if (value % 1 !== 0 || value < 0 || value > maximum)
						fail(path, `expected an integer in [0, ${maximum}]`);
				}
				return value;
			}
			case "object": {
				const source = tableAt(value, path, depth) as Record<string, unknown>;
				const copy = new Map<unknown, unknown>();
				const declared = new Set<string>();
				for (const [name, child] of node.fields) {
					declared.add(name);
					const field = source[name];
					if (field !== undefined) copy.set(name, walk(child, field, `${path}.${name}`, depth + 1));
					else if (!admitsNil(child)) fail(`${path}.${name}`, "a required field is missing");
				}
				for (const [key, child] of pairs(source)) {
					if (!typeIs(key, "string")) fail(path, "expected an object");
					if (!declared.has(key)) copy.set(key, json(child, `${path}.${key}`, depth + 1));
				}
				return finish(source, copy);
			}
			case "array": {
				const source = dense(value, path, depth);
				const copy = source.map((element, i) => walk(node.element, element, `${path}[${i}]`, depth + 1));
				return finish(source, copy);
			}
			case "tuple": {
				const source = dense(value, path, depth);
				if (source.size() !== node.elements.size()) fail(path, `expected ${node.elements.size()} elements`);
				const copy = node.elements.map((element, i) => walk(element, source[i], `${path}[${i}]`, depth + 1));
				return finish(source, copy);
			}
			case "map": {
				const source = tableAt(value, path, depth);
				const copy = new Map<unknown, unknown>();
				for (const [key, child] of pairs(source)) {
					const at = `${path}.${tostring(key)}`;
					walk(node.key, key, at, depth + 1);
					copy.set(key, walk(node.value, child, at, depth + 1));
				}
				return finish(source, copy);
			}
			case "union": {
				for (const branch of node.branches) {
					if (branch.kind === "variant") {
						if (typeIs(value, "table") && (value as Record<string, unknown>)[branch.key] === branch.value)
							return walk(branch.body, value, path, depth);
					} else if (
						branch.kind === "literal"
							? value === branch.value
							: branch.kind === "nil"
								? value === undefined
								: typeOf(value) === runtimeKind(branch)
					)
						return walk(branch, value, path, depth);
				}
				return fail(path, `matches no member of the union, got ${typeOf(value)}`);
			}
			default:
				fail(path, `a ${node.kind} cannot be a config value`);
		}
	}

	return walk(node, value, root, 0);
}

function acceptsString(node: Branch): boolean {
	if (node.kind === "union") return node.branches.some(acceptsString);
	return node.kind === "string" || (node.kind === "literal" && typeIs(node.value, "string"));
}

function admitRaw(declaration: Declaration, raw: unknown): unknown {
	const port = configPort();
	let value = raw;
	if (typeIs(raw, "string")) {
		if (raw.size() > L.CONFIG_VALUE_MAX_BYTES) fail(declaration.name, `over ${L.CONFIG_VALUE_MAX_BYTES} bytes`);
		if (!acceptsString(declaration.node)) {
			const [ok, decoded] = pcall(() => port.decodeJson(raw));
			if (!ok) fail(declaration.name, "invalid JSON");
			value = decoded;
		}
	}
	if (typeIs(value, "table")) {
		const [ok, bytes] = pcall(() => port.jsonBytes(value as object));
		if (!ok) fail(declaration.name, "not a JSON value");
		if (bytes > L.CONFIG_VALUE_MAX_BYTES) fail(declaration.name, `over ${L.CONFIG_VALUE_MAX_BYTES} bytes`);
	}
	return admit(declaration.node, value, declaration.name);
}

function failure(err: unknown): Failure {
	if (typeIs(err, "table") && (err as Failure).configFailure === true) return err as Failure;
	return { configFailure: true, path: "", reason: "admission failed" };
}

function deepEqual(a: unknown, b: unknown): boolean {
	if (a === b) return true;
	if (!typeIs(a, "table") || !typeIs(b, "table")) return false;
	for (const [key, value] of pairs(a))
		if (!deepEqual(value, (b as Record<string, unknown>)[key as string])) return false;
	for (const [key] of pairs(b)) if ((a as Record<string, unknown>)[key as string] === undefined) return false;
	return true;
}

function validName(name: unknown): name is string {
	if (!typeIs(name, "string") || name.size() === 0) return false;
	const segments = name.split(".");
	return segments[0] !== "Crucible" && segments.every((s) => s.match("^[A-Za-z][A-Za-z0-9_]*$")[0] !== undefined);
}

export function declare(name: string, kind: ConfigKind, fallback: unknown, node: Node): Declaration {
	if (!validName(name))
		error(
			`Config name ${tostring(name)} is invalid: use dot-separated [A-Za-z][A-Za-z0-9_]* segments, not Crucible.*`,
			3,
		);
	if (name.size() > L.CONFIG_NAME_MAX_BYTES)
		error(`Config name ${name} is ${name.size()} bytes; the limit is ${L.CONFIG_NAME_MAX_BYTES}`, 3);
	if (names.has(name)) error(`Config ${name} is declared twice`, 3);
	if (declarations.size() >= L.MAX_CONFIG_DECLARATIONS)
		error(`Config ${name} passes the limit of ${L.MAX_CONFIG_DECLARATIONS} configs`, 3);
	if (sealed)
		error(`${name} was declared after the config registry sealed; declare configs in modules that boot loads`, 3);
	assertDescriptor(node);
	const [ok, admitted] = pcall(() => admit(node, fallback, name));
	if (!ok) {
		const f = failure(admitted);
		error(`Config ${name} has an invalid fallback at ${f.path}: ${f.reason}`, 3);
	}
	const declaration: Declaration = { name, kind, node, fallback: admitted, global: { value: admitted } };
	names.add(name);
	declarations.push(declaration);
	return declaration;
}

export function readGlobal(declaration: Declaration): unknown {
	return declaration.global.value;
}

export function readPlayer(declaration: Declaration, player: Player): unknown {
	const scope = slots.get(player);
	if (scope === undefined) return declaration.fallback;
	if (scope.phase !== Phase.Loaded) return declaration.global.value;
	return scope.values.get(declaration)!.value;
}

export function ready(player?: Player): boolean {
	if (player === undefined) return global.ready.value as boolean;
	return slots.get(player)?.ready.value === true;
}

function ended(runtime: ConfigRuntime): boolean {
	const s = stateOf(runtime);
	return s === undefined || !isLive(s);
}

function retire() {
	retired = true;
	for (const disconnect of listeners) disconnect();
	listeners = [];
	global.snapshot?.disconnect();
	for (const [, scope] of slots) scope.snapshot?.disconnect();
}

function live(): boolean {
	if (owner === undefined || retired) return false;
	if (ended(owner)) retire();
	return !retired;
}

/** `subject` is a config name, or undefined for the snapshot request itself. */
function diagnose(scope: Scope, subject: string | undefined, reason: string) {
	const key = subject ?? "";
	if (scope.diagnosed.has(key)) return;
	scope.diagnosed.add(key);
	const where = scope.player === undefined ? "global snapshot" : `snapshot of player ${scope.player.UserId}`;
	runtimeDiagnostic("warning", `Config ${subject ?? "request"} (${where}): ${reason}`);
}

function acquire() {
	if (active < L.CONFIG_REQUEST_CONCURRENCY) {
		active++;
		return;
	}
	queue.push(coroutine.running());
	coroutine.yield();
}

function release() {
	const waiting = queue.shift();
	if (waiting !== undefined) task.spawn(waiting);
	else active--;
}

function abandoned(scope: Scope): boolean {
	return scope.phase === Phase.Gone || !live();
}

/** One ConfigService request through the shared limit. Undefined when the scope is gone or configs retired. */
function request<T>(scope: Scope, call: () => T): { readonly ok: boolean; readonly value: unknown } | undefined {
	acquire();
	if (abandoned(scope)) {
		release();
		return undefined;
	}
	const [ok, value] = pcall(call);
	release();
	if (abandoned(scope)) return undefined;
	return { ok, value };
}

function stage(scope: Scope, snapshot: Snapshot, first: boolean, updates: number): Staged {
	scope.generation++;
	scope.diagnosed = new Set();
	const values = new Map<Declaration, unknown>();
	for (const declaration of declarations) {
		if (scope.player !== undefined && declaration.kind !== "player") continue;
		const [found, raw] = pcall(() => snapshot.port.get(declaration.name));
		if (!found) {
			diagnose(scope, declaration.name, "GetValue failed");
			values.set(declaration, KEEP);
		} else if (raw === undefined) values.set(declaration, declaration.fallback);
		else {
			const [ok, admitted] = pcall(() => admitRaw(declaration, raw));
			if (ok) values.set(declaration, admitted);
			else {
				const f = failure(admitted);
				diagnose(scope, declaration.name, `${f.path}: ${f.reason}; keeping the last good value`);
				values.set(declaration, KEEP);
			}
		}
	}
	return { scope, snapshot, first, updates, values };
}

function failed(snapshot: Snapshot): boolean {
	const [ok, value] = pcall(() => snapshot.port.failed());
	return !ok || value === true;
}

function stale(snapshot: Snapshot): boolean {
	if (snapshot.updates !== snapshot.applied || failed(snapshot)) return true;
	const [ok, outdated] = pcall(() => snapshot.port.outdated());
	return !ok || outdated === true;
}

function fetch(scope: Scope): Staged | undefined {
	const player = scope.player;
	const port = configPort();
	const result = request(scope, () => (player === undefined ? port.global() : port.forPlayer(player)));
	if (result === undefined) return undefined;
	if (!result.ok) {
		diagnose(scope, undefined, `the fetch failed: ${tostring(result.value)}`);
		return undefined;
	}
	const snapshotPort = result.value as ConfigSnapshotPort;
	const snapshot: Snapshot = {
		port: snapshotPort,
		updates: 0,
		applied: 0,
		disconnect: snapshotPort.onUpdateAvailable(() => {
			if (live()) snapshot.updates++;
		}),
	};
	if (failed(snapshot)) {
		snapshot.disconnect();
		diagnose(scope, undefined, "the snapshot failed to load");
		return undefined;
	}
	return stage(scope, snapshot, true, 0);
}

function refreshScope(scope: Scope): Staged | undefined {
	if (scope.phase === Phase.Fetching) return fetch(scope);
	const snapshot = scope.snapshot;
	if (scope.phase !== Phase.Loaded || snapshot === undefined || !stale(snapshot)) return undefined;
	const updates = snapshot.updates;
	const result = request(scope, () => snapshot.port.refresh());
	if (result === undefined) return undefined;
	if (!result.ok || failed(snapshot)) {
		diagnose(
			scope,
			undefined,
			result.ok ? "the snapshot failed to load" : `the refresh failed: ${tostring(result.value)}`,
		);
		return undefined;
	}
	return stage(scope, snapshot, false, updates);
}

/** Writes every changed value of every result in one pass that never yields. */
function commit(results: readonly Staged[]) {
	const apply = live();
	for (const result of results) {
		const scope = result.scope;
		if (!apply || scope.phase === Phase.Gone || (result.first && scope.phase === Phase.Loaded)) {
			if (result.first) result.snapshot.disconnect();
			continue;
		}
		if (result.first) {
			scope.snapshot = result.snapshot;
			scope.phase = Phase.Loaded;
		}
		result.snapshot.applied = result.updates;
		for (const [declaration, value] of result.values) {
			let cell: Cell;
			if (scope.player === undefined) cell = declaration.global;
			else {
				cell = scope.values.get(declaration) ?? { value: declaration.fallback };
				scope.values.set(declaration, cell);
			}
			if (value !== KEEP && !deepEqual(cell.value, value)) cell.value = value;
		}
		scope.ready.value = true;
	}
}

function fetchLoop(scope: Scope) {
	const port = configPort();
	for (let attempt = 0; ; attempt++) {
		if (attempt > 0)
			port.wait(
				port.random() *
					math.min(L.CONFIG_FETCH_RETRY_CAP_SECONDS, L.CONFIG_FETCH_RETRY_BASE_SECONDS * 2 ** (attempt - 1)),
			);
		if (scope.phase !== Phase.Fetching || !live()) return;
		const result = fetch(scope);
		if (result !== undefined) {
			commit([result]);
			return;
		}
	}
}

export function refresh(): boolean {
	if (!live()) return false;
	if (flight !== undefined) {
		flight.waiters.push(coroutine.running());
		return coroutine.yield()[0] as boolean;
	}
	const current = { waiters: new Array<thread>() };
	flight = current;
	const scopes = [global];
	for (const [, scope] of slots) scopes.push(scope);
	const results = new Array<Staged>();
	const caller = coroutine.running();
	let pending = scopes.size();
	let closed = false;
	let parked = false;
	const close = () => {
		if (closed) return;
		closed = true;
		if (parked) task.spawn(caller);
	};
	for (const scope of scopes)
		task.spawn(() => {
			const result = refreshScope(scope);
			if (closed) {
				if (result?.first) result.snapshot.disconnect();
				return;
			}
			if (result !== undefined) results.push(result);
			if (--pending === 0) close();
		});
	if (!closed) {
		task.spawn(() => {
			configPort().wait(L.CONFIG_REFRESH_TIMEOUT_SECONDS);
			close();
		});
		parked = true;
		coroutine.yield();
	}
	commit(results);
	const ok =
		live() &&
		scopes.every(
			(scope) => scope.phase === Phase.Gone || (scope.phase === Phase.Loaded && !stale(scope.snapshot!)),
		);
	flight = undefined;
	for (const waiter of current.waiters) task.spawn(waiter, ok);
	return ok;
}

function join(player: Player) {
	if (!live() || slots.has(player)) return;
	const scope = newScope(player);
	slots.set(player, scope);
	task.spawn(fetchLoop, scope);
}

function leave(player: Player) {
	if (!live()) return;
	const scope = slots.get(player);
	if (scope === undefined) return;
	slots.delete(player);
	scope.phase = Phase.Gone;
	scope.snapshot?.disconnect();
}

function activate(runtime: ConfigRuntime) {
	owner = runtime;
	retired = false;
	sealed = true;
	task.spawn(fetchLoop, global);
	if (!declarations.some((d) => d.kind === "player")) return;
	const port = configPort();
	listeners.push(port.onPlayerAdded(join), port.onPlayerRemoving(leave));
	for (const player of port.players()) join(player);
}

class ConfigRuntime extends Subsystem {
	protected start() {
		if (declarations.size() === 0) return;
		configPort().wait(0);
		if (ended(this)) return;
		activate(this);
	}
}

export function installConfigRuntime() {
	define(ConfigRuntime, { kind: "subsystem", id: "Crucible.ConfigRuntime" as TypeId, deps: [], provides: [] });
}
