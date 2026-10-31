import { compile, type Codec } from "./codec";
import * as L from "./limits";
import type { Node } from "./schema";

export type MessageKind = "to-server" | "to-client" | "ask-server";
export interface MessageSpec {
	readonly kind: MessageKind;
	readonly name: string;
	readonly input: Node;
	readonly output?: Node;
	readonly timeout?: number;
}
export interface Definition {
	readonly kind: MessageKind;
	readonly name: string;
	readonly input: Codec;
	readonly output?: Codec;
	readonly timeout: number;
	readonly index?: number;
}

const definitions = new Map<string, Definition>();
const tokens = new Set<Definition>();
let names = new Array<string>();
let indices = new Map<string, number>();
let sealed = false;

function validName(name: string) {
	assert(
		typeIs(name, "string") && name.size() > 0 && name.size() <= L.MESSAGE_NAME_MAX_BYTES,
		"invalid message name",
	);
	const segments = name.split(".");
	assert(segments[0] !== "Crucible", "reserved message name");
	for (const segment of segments)
		assert(segment.match("^[A-Za-z][A-Za-z0-9_]*$")[0] !== undefined, "invalid message name");
}

export function define(spec: MessageSpec): Definition {
	validName(spec.name);
	assert(
		!sealed,
		`${spec.name} was declared after the network catalog sealed; declare messages in modules that boot loads.`,
	);
	assert(!definitions.has(spec.name), `duplicate message ${spec.name}`);
	assert(
		spec.kind === "to-server" || spec.kind === "to-client" || spec.kind === "ask-server",
		"invalid message kind",
	);
	assert(
		spec.kind === "ask-server"
			? spec.output !== undefined
			: spec.output === undefined && spec.timeout === undefined,
		"invalid message options",
	);
	const timeout = spec.timeout ?? L.DEFAULT_ASK_TIMEOUT_SECONDS;
	assert(timeout > 0 && timeout <= L.MAX_ASK_TIMEOUT_SECONDS, "invalid ask timeout");
	const tally = { nodes: 0 };
	const definition: Definition = table.freeze(
		setmetatable(
			{
				kind: spec.kind,
				name: spec.name,
				input: compile(spec.input, tally),
				output: spec.output === undefined ? undefined : compile(spec.output, tally),
				timeout,
			},
			{
				__index: (_: unknown, key: unknown) => (key === "index" ? indices.get(spec.name) : undefined),
			},
		),
	);
	definitions.set(spec.name, definition);
	tokens.add(definition);
	return definition;
}

function setNames(ordered: string[]) {
	names = ordered;
	indices = new Map<string, number>();
	for (let i = 0; i < names.size(); i++) indices.set(names[i], i);
}

export function seal(): string {
	assert(!sealed, "catalog already sealed");
	assert(definitions.size() <= L.MAX_MESSAGES, "too many messages");
	const ordered = new Array<string>();
	for (const [name] of definitions) ordered.push(name);
	ordered.sort();
	setNames(ordered);
	sealed = true;
	return names.join("\n");
}

export function adopt(catalog: string): void {
	assert(
		!sealed && typeIs(catalog, "string") && catalog.size() <= L.MAX_MESSAGES * (L.MESSAGE_NAME_MAX_BYTES + 1),
		"invalid catalog",
	);
	const ordered = catalog === "" ? [] : catalog.split("\n");
	assert(ordered.size() <= L.MAX_MESSAGES, "too many messages");
	for (let i = 0; i < ordered.size(); i++) {
		validName(ordered[i]);
		assert(i === 0 || ordered[i - 1] < ordered[i], "catalog not strictly ascending");
	}
	setNames(ordered);
}

export function definitionAt(index: number): Definition | undefined {
	if (index < 0 || index % 1 !== 0 || index >= names.size()) return undefined;
	return definitions.get(names[index]);
}
export function definitionOf(token: unknown): Definition | undefined {
	return tokens.has(token as Definition) ? (token as Definition) : undefined;
}
export function discardIndices(): void {
	sealed = false;
	setNames([]);
}
export function unpublished(): readonly Definition[] {
	const result = new Array<Definition>();
	for (const [, definition] of definitions) if (definition.index === undefined) result.push(definition);
	return result;
}
export function catalogSize(): number {
	return names.size();
}
