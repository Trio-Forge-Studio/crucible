import { assertDescriptor, n, type Node, type ObjectNode } from "../network/schema";
import { MAX_REPLICATED_DECLARATIONS } from "../reactive/limits";
import { declarationOf, declareFrameworkReplicated, replicatedCapacity, type Declaration } from "../sync/declarations";
import { admit as admitReplicated, compilePlan } from "../sync/values";
import { sealSchema, type MigrationStep, type Schema } from "./admission";
import { rootHandle, type HandleSource } from "./handles";
import {
	LIVE_NAME_PREFIX,
	MAX_DATA_DECLARATIONS,
	MAX_PROFILE_STORES,
	VIEW_NAME_MAX_BYTES,
	VIEW_NAME_PREFIX,
} from "./limits";
import { fieldPaths, getPath, pathLess, samePath, showPath, type Segment } from "./paths";
import {
	checkStoreName,
	dataDeclarationCount,
	dataRegistrySealed,
	registerProfileName,
	storeNameTaken,
} from "./record-declarations";

/** @internal One top-level field the owner's client receives, and its view declaration (rules 125, 134). */
export interface ViewField {
	readonly name: string;
	readonly handle: unknown;
	readonly declaration: Declaration;
}

/** @internal A profile store as both realms know it, built once at module evaluation (rule 4). */
export interface ProfileDeclaration {
	readonly name: string;
	/** Every field, serverOnly ones included. */
	readonly persist: Node;
	/** `ClientData<T>`: no serverOnly field and no object left with no field (rules 125, 129). */
	readonly replica: ObjectNode;
	/** The durability engine's schema; its defaults are the template, unwrapped, admitted and frozen (rule 76). */
	readonly schema: Schema;
	/** The template admitted through the replica: what a client reads while nothing loaded was received (rule 126). */
	readonly clientTemplate: object;
	readonly live: unknown;
	readonly liveDeclaration: Declaration;
	readonly fields: readonly ViewField[];
	readonly paths: readonly (readonly string[])[];
}

/** @internal What the server entry installs so a shared declaration reaches the durability engine (rule 115). */
export interface ServerSide {
	readonly root: (declaration: ProfileDeclaration, player: Player) => unknown;
	readonly member: (declaration: ProfileDeclaration, name: string) => unknown;
}

/**
 * The record a store carries in its metatable. The key is a string, not a private table, so `replicatedPaths` reads a
 * declaration made by another module set (rule 131).
 */
const RECORD = "__crucibleProfileStore";

const profiles = new Array<ProfileDeclaration>();
const clientRoots = new Map<ProfileDeclaration, object>();
let serverSide: ServerSide | undefined;

const markMeta = {};

/** @internal `serverOnly(value)`: a frozen mark recognized by its private metatable (rule 129). */
export function markServerOnly(value: unknown): unknown {
	return table.freeze(setmetatable({ value }, markMeta as LuaMetatable<{ value: unknown }>));
}

function isMark(value: unknown): value is { readonly value: unknown } {
	return typeIs(value, "table") && getmetatable(value) === markMeta;
}

/** Replaces every mark by the value it wraps, recording where each one sat as an array of keys (rule 4). */
function unwrap(value: unknown, path: string[], found: string[][]): unknown {
	if (isMark(value)) {
		found.push(path);
		return unwrap(value.value, path, found);
	}
	if (!typeIs(value, "table") || getmetatable(value) !== undefined) return value;
	let copy: Record<string | number, unknown> | undefined;
	for (const [key, entry] of pairs(value)) {
		const inner = unwrap(entry, [...path, tostring(key)], found);
		if (rawequal(inner, entry)) continue;
		copy ??= table.clone(value as Record<string | number, unknown>);
		copy[key as string] = inner;
	}
	return copy ?? value;
}

function checkMarks(name: string, found: string[][], expected: readonly (readonly string[])[]) {
	const marked = [...found].sort(pathLess);
	const typed = [...expected].sort(pathLess);
	for (const path of marked)
		if (!typed.some((other) => samePath(other, path)))
			error(`${name} ${showPath(path)} is marked serverOnly in its template value but not in its type`, 0);
	for (const path of typed)
		if (!marked.some((other) => samePath(other, path)))
			error(`${name} ${showPath(path)} is serverOnly in its type but its template value is not marked`, 0);
}

/** A field view's descriptor: the field's replica node admitting undefined (rule 125). */
function admittingUndefined(node: Node): Node {
	if (node.kind === "union") {
		for (const branch of node.branches) if (branch.kind === "nil") return node;
		return n.union([n.nil, ...node.branches]);
	}
	return n.union([n.nil, node]);
}

/** Rules 2 and 134: every view name is checked, and the realm's room counted, before any is declared. */
function declareViews(name: string, replica: ObjectNode) {
	const prefix = `${VIEW_NAME_PREFIX}${name}.`;
	for (const [field] of replica.fields) {
		if (field.match("^[A-Za-z][A-Za-z0-9_]*$")[0] === undefined)
			error(
				`${name}["${field}"]: a field the owner's client receives is named by one segment, [A-Za-z][A-Za-z0-9_]*; rename it or mark it serverOnly`,
				0,
			);
		const view = prefix + field;
		if (view.size() > VIEW_NAME_MAX_BYTES)
			error(
				`${name}.${field}: its view name ${view} is ${view.size()} bytes, over ${VIEW_NAME_MAX_BYTES}; this store allows field names of at most ${VIEW_NAME_MAX_BYTES - prefix.size()} bytes`,
				0,
			);
	}
	const needed = 1 + replica.fields.size();
	const room = replicatedCapacity();
	if (room < needed)
		error(
			`${name} needs ${needed} replicated declarations; ${MAX_REPLICATED_DECLARATIONS - room} of ${MAX_REPLICATED_DECLARATIONS} are in use`,
			0,
		);
	const live = declareFrameworkReplicated(`${LIVE_NAME_PREFIX}${name}`, false, n.bool);
	const fields = replica.fields.map(([field, node]): ViewField => {
		const handle = declareFrameworkReplicated(prefix + field, undefined, admittingUndefined(node));
		return { name: field, handle, declaration: declarationOf(handle)! };
	});
	return { live, fields };
}

function clientSource(declaration: ProfileDeclaration): HandleSource {
	const byName = new Map<string, ViewField>();
	for (const field of declaration.fields) byName.set(field.name, field);
	return {
		name: declaration.name,
		client: true,
		beforeRead() {},
		write() {
			error(`${declaration.name}() is read-only on the client`, 3);
		},
		read(path: readonly Segment[]) {
			const live = declaration.liveDeclaration.node() === true;
			if (path.size() === 0) {
				if (!live) return declaration.clientTemplate;
				const root: Record<string, unknown> = {};
				for (const field of declaration.fields) root[field.name] = field.declaration.node();
				return table.freeze(root);
			}
			const field = byName.get(path[0] as string);
			if (live && field !== undefined) return getPath(field.declaration.node(), path, 1);
			return getPath(declaration.clientTemplate, path);
		},
	};
}

/** @internal Rule 124: the local player's root handle, made once per realm. */
export function clientRoot(declaration: ProfileDeclaration): object {
	let root = clientRoots.get(declaration);
	if (root === undefined) {
		root = rootHandle(clientSource(declaration), declaration.replica);
		clientRoots.set(declaration, root);
	}
	return root;
}

/** @internal Rule 126: tracked; the lifecycle is ready and true, and every field view is ready. */
export function clientLoaded(declaration: ProfileDeclaration): boolean {
	const live = declaration.liveDeclaration;
	if (live.status() !== "ready" || live.node() !== true) return false;
	for (const field of declaration.fields) if (field.declaration.status() !== "ready") return false;
	return true;
}

function storeMeta(declaration: ProfileDeclaration): LuaMetatable<object> {
	const name = declaration.name;
	const meta: Record<string, unknown> = {
		__call: (_: unknown, ...args: unknown[]) => {
			const player = args[0];
			if (serverSide !== undefined) {
				if (player === undefined) error(`${name} is per player on the server; call ${name}(player)`, 2);
				return serverSide.root(declaration, player as Player);
			}
			if (args.size() > 0) error(`a client reads only its own profile; call ${name}()`, 2);
			return clientRoot(declaration);
		},
		__index: (_: unknown, key: unknown) => {
			if (serverSide !== undefined) return serverSide.member(declaration, key as string);
			if (key === "loaded")
				return (_store: unknown, player?: unknown) => {
					if (player !== undefined)
						error(`${name}.loaded(player) is server-only; the client reads ${name}.loaded()`, 2);
					return clientLoaded(declaration);
				};
			return () => error(`${name}.${tostring(key)} is server-only`, 2);
		},
		__newindex: () => error(`${name} is a profile store; it has no fields to assign`, 2),
	};
	meta[RECORD] = declaration;
	return meta as LuaMetatable<object>;
}

/**
 * @internal The emitted `profileStore` (rules 1 to 4, 129, 131, 134). Shared-safe: it touches no service and starts
 * nothing, and every check runs before anything registers.
 */
export function declareProfileStore(
	name: string,
	template: unknown,
	migrations: readonly MigrationStep[] | undefined,
	persist: Node,
	replica: Node,
	serverOnlyPaths: readonly (readonly string[])[],
): object {
	checkStoreName(name);
	if (dataRegistrySealed())
		error(`${name} was declared after the data registry sealed; declare stores in modules that boot loads`, 3);
	if (storeNameTaken(name)) error(`${name} is already declared`, 3);
	if (profiles.size() >= MAX_PROFILE_STORES) error(`${name}: more than ${MAX_PROFILE_STORES} profile stores`, 3);
	if (dataDeclarationCount() >= MAX_DATA_DECLARATIONS)
		error(`${name}: more than ${MAX_DATA_DECLARATIONS} data declarations`, 3);
	assertDescriptor(persist);
	assertDescriptor(replica);
	if (replica.kind !== "object") error(`${name}: a profile store's value is an object with declared fields`, 3);
	const found = new Array<string[]>();
	const unwrapped = unwrap(template, [], found);
	checkMarks(name, found, serverOnlyPaths);
	const steps = [...(migrations ?? [])];
	const schema = sealSchema({
		name,
		version: steps.size(),
		defaults: unwrapped as object,
		node: persist,
		migrations: steps,
	});
	const clientTemplate = admitReplicated(compilePlan(replica), name, schema.defaults) as object;
	const views = declareViews(name, replica);
	const declaration: ProfileDeclaration = {
		name,
		persist,
		replica,
		schema,
		clientTemplate,
		live: views.live,
		liveDeclaration: declarationOf(views.live)!,
		fields: views.fields,
		paths: fieldPaths(replica),
	};
	registerProfileName(name);
	profiles.push(declaration);
	return table.freeze(setmetatable({}, storeMeta(declaration)));
}

/** @internal Rule 131: the replica's paths, read from the store's own metatable, never a registry. */
export function replicatedPathsOf(store: unknown): readonly (readonly string[])[] {
	const meta = typeIs(store, "table") ? (getmetatable(store) as Record<string, unknown> | undefined) : undefined;
	const declaration = typeIs(meta, "table") ? (meta[RECORD] as ProfileDeclaration | undefined) : undefined;
	if (declaration === undefined) error("replicatedPaths takes a profile store", 2);
	return declaration.paths;
}

/** @internal This realm's profile stores, in registration order; DataRuntime binds each when it constructs (104). */
export function profileDeclarations(): readonly ProfileDeclaration[] {
	return profiles;
}

/** @internal The server entry installs its side once per realm, when its module loads (rule 115). */
export function installServerSide(side: ServerSide) {
	serverSide = side;
}
