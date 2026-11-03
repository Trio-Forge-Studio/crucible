import { readClaim } from "../construction-claim";
import { compile } from "../network/codec";
import { MESSAGE_NAME_MAX_BYTES } from "../network/limits";
import { assertDescriptor, type Node } from "../network/schema";
import { createStateNode, guardPure, readState, readUntracked, write, type StateNode } from "../reactive/graph";
import { MAX_REPLICATED_DECLARATIONS } from "../reactive/limits";
import type { ClassRecord } from "../registry";
import { Kind, type ObjectState } from "../state";
import { admit, compilePlan, type Plan } from "./values";

/** @internal */
export type Audience = "all" | "player";

/** @internal */
export type Status = "pending" | "ready" | "failed";

/** @internal One declaration's server write side (rules 47 to 54). */
export interface Publication {
	readonly label: string;
	readonly project?: (player: Player) => unknown;
	live: boolean;
}

/** @internal */
export interface Declaration {
	readonly name: string;
	readonly audience: Audience;
	readonly plan: Plan;
	/** `initial`, admitted at declaration. Rollback restores it. */
	readonly initial: unknown;
	/** The value every reader goes through: the server's authoritative value, or the client's last applied one. */
	readonly node: StateNode;
	readonly status: StateNode;
	publication?: Publication;
}

const byName = new Map<string, Declaration>();
const handles = setmetatable(new Map<unknown, Declaration>(), { __mode: "k" });
let ordered = new Array<Declaration>();
let sealed = false;
let onRetired: (declaration: Declaration) => void = () => {};

function onServer(): boolean {
	return game.GetService("RunService").IsServer();
}

function perPlayer(declaration: Declaration): never {
	error(`${declaration.name} is per player; on the server, read the sources its publisher projects from`, 0);
}

function checkName(name: unknown, framework: boolean): asserts name is string {
	const grammar = `use 1 to ${MESSAGE_NAME_MAX_BYTES} bytes of dot-separated [A-Za-z][A-Za-z0-9_]* segments`;
	if (!typeIs(name, "string") || name.size() === 0 || name.size() > MESSAGE_NAME_MAX_BYTES)
		error(`Crucible replicated name ${tostring(name)} is invalid: ${grammar}`, 0);
	const segments = name.split(".");
	for (const segment of segments)
		if (segment.match("^[A-Za-z][A-Za-z0-9_]*$")[0] === undefined)
			error(`Crucible replicated name ${name} is invalid: ${grammar}`, 0);
	if ((segments[0] === "Crucible") !== framework)
		error(
			framework
				? `Crucible replicated name ${name} is invalid: a framework declaration starts with Crucible.`
				: `Crucible replicated name ${name} is invalid: Crucible. is reserved`,
			0,
		);
}

function register(audience: Audience, name: string, initial: unknown, descriptor: Node): unknown {
	if (sealed)
		error(
			`${name} was declared after the replicated registry sealed; declare replicated state in modules that boot loads`,
			0,
		);
	if (byName.has(name)) error(`Crucible replicated name ${name} is declared twice`, 0);
	if (byName.size() >= MAX_REPLICATED_DECLARATIONS)
		error(`${name} would pass ${MAX_REPLICATED_DECLARATIONS} replicated declarations in this realm`, 0);
	assertDescriptor(descriptor);
	compile(descriptor);
	const plan = compilePlan(descriptor);
	const admitted = admit(plan, name, initial);
	const declaration: Declaration = {
		name,
		audience,
		plan,
		initial: admitted,
		node: createStateNode(admitted),
		status: createStateNode("pending"),
	};
	byName.set(name, declaration);
	ordered.push(declaration);
	ordered.sort((a, b) => a.name < b.name);
	const handle =
		audience === "all"
			? () => readState(declaration.node)
			: () => (onServer() ? perPlayer(declaration) : readState(declaration.node));
	handles.set(handle, declaration);
	return handle;
}

/** @internal The emitted `replicated` and `replicatedPerPlayer` (rules 40, 41, 78). */
export function declareReplicated(audience: Audience, name: string, initial: unknown, descriptor: Node): unknown {
	checkName(name, false);
	return register(audience, name, initial, descriptor);
}

/**
 * @internal
 * R-D1 (data rule 133): a per-player declaration only the framework makes, under the reserved `Crucible.` segment.
 * It counts against MAX_REPLICATED_DECLARATIONS and the duplicate check like any other.
 */
export function declareFrameworkReplicated(name: string, initial: unknown, descriptor: Node): unknown {
	checkName(name, true);
	return register("player", name, initial, descriptor);
}

/** @internal How many more replicated declarations this realm admits (data rule 134). */
export function replicatedCapacity(): number {
	return MAX_REPLICATED_DECLARATIONS - byName.size();
}

/** @internal */
export function declarationOf(handle: unknown): Declaration | undefined {
	return handles.get(handle);
}

/** @internal */
export function declarationNamed(name: string): Declaration | undefined {
	return byName.get(name);
}

/** @internal Every declaration in this realm, by name in byte order. */
export function declarations(): readonly Declaration[] {
	return ordered;
}

/** @internal `syncStatus` (rule 45): a tracked read of the status node. */
export function statusOf(handle: unknown): Status {
	const declaration = declarationOf(handle) ?? error("syncStatus takes a replicated declaration", 0);
	if (declaration.audience === "player" && onServer()) perPlayer(declaration);
	return readState(declaration.status) as Status;
}

/** @internal ServerSync registers how a retirement reaches every peer (rule 52). */
export function useRetired(listener: (declaration: Declaration) => void) {
	onRetired = listener;
}

function retire(declaration: Declaration, publication: Publication) {
	if (!publication.live) return;
	publication.live = false;
	if (declaration.audience === "all") write(declaration.status, "failed");
	onRetired(declaration);
}

function ended(declaration: Declaration, publication: Publication): never {
	error(`${declaration.name}'s publisher ${publication.label} has ended`, 0);
}

interface PublishedState {
	readonly declaration: Declaration;
	readonly publication: Publication;
}

const publishedMethods = {
	set(this: PublishedState, value: unknown) {
		guardPure("write state");
		const { declaration, publication } = this;
		if (!publication.live) ended(declaration, publication);
		write(declaration.node, admit(declaration.plan, declaration.name, value));
	},
	update(this: PublishedState, change: (current: unknown) => unknown) {
		guardPure("write state");
		const { declaration, publication } = this;
		if (!publication.live) ended(declaration, publication);
		const changed = readUntracked(() => change(declaration.node.value), "update's callback");
		if (!publication.live) ended(declaration, publication);
		write(declaration.node, admit(declaration.plan, declaration.name, changed));
	},
};

const publishedMeta: LuaMetatable<PublishedState> = {
	__index: publishedMethods as never,
	__call: (handle) => readState(handle.declaration.node),
};

/** @internal `publish`'s runtime half (rules 47 to 50). */
export function claimPublication(target: unknown, project?: (player: Player) => unknown): unknown {
	const owner: ObjectState | undefined = readClaim();
	if (owner === undefined || owner.record.kind !== Kind.Subsystem)
		error("publish runs while a server Subsystem is constructed; make it a field initializer", 0);
	const declaration = declarationOf(target) ?? error("publish takes a replicated declaration", 0);
	const label = (owner.record as ClassRecord).id;
	const previous = declaration.publication;
	if (previous !== undefined)
		error(
			`${declaration.name} is published by ${previous.label} and ${label}; one Subsystem publishes each value`,
			0,
		);
	if ((declaration.audience === "player") !== (project !== undefined))
		error(
			declaration.audience === "player"
				? `${declaration.name} is per player; publish it with a projection`
				: `${declaration.name} is public; publish it without a projection`,
			0,
		);
	if (project !== undefined && !typeIs(project, "function"))
		error(`${declaration.name}: publish needs a projection function`, 0);
	const publication: Publication = { label, project, live: true };
	declaration.publication = publication;
	if (owner.subscriptions === undefined) owner.subscriptions = [];
	owner.subscriptions.push(() => retire(declaration, publication));
	if (declaration.audience === "player") return table.freeze({});
	write(declaration.status, "ready");
	return setmetatable({ declaration, publication }, publishedMeta);
}

/** @internal The server's registry seals with the catalog; the client's at its network commit (rule 41). */
export function sealDeclarations() {
	sealed = true;
}

/**
 * @internal
 * Rule 74: retire every publication silently, restore every value to its admitted `initial` and status to
 * "pending", and unseal. Declarations stay registered, because their modules stay cached.
 */
export function rollbackDeclarations() {
	for (const declaration of ordered) {
		const publication = declaration.publication;
		if (publication !== undefined) publication.live = false;
		declaration.publication = undefined;
		write(declaration.node, declaration.initial);
		write(declaration.status, "pending");
	}
	sealed = false;
}
