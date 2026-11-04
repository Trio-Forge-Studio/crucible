import type { CrucibleBehaviour } from "../crucible-behaviour";
import { clearClaim, setClaim } from "../internal/construction-claim";
import { track } from "../internal/lifecycle";
import { closeClaims, openClaims, receiverOf } from "../internal/network/claims";
import { definitionOf, seal, type Definition, type MessageKind } from "../internal/network/catalog";
import { Reader, Writer, type CodecFailure } from "../internal/network/codec";
import type { Branch } from "../internal/network/schema";
import { recordOf, type ClassRecord } from "../internal/registry";
import { Kind, type ObjectState } from "../internal/state";
import type { Subsystem } from "../subsystem";

export { n } from "../internal/network/schema";

function recordFor(klass: object): ClassRecord {
	return (
		recordOf(klass) ??
		error(`${tostring(klass)} is not a registered Crucible class; compile it with the Crucible transformer`, 0)
	);
}

function definitionFor(token: unknown): Definition {
	return definitionOf(token) ?? error("not a message declared with toServer, toClient or askServer", 0);
}

function construct<T, A extends unknown[]>(klass: new (...args: A) => T, state: ObjectState, args: A): T {
	const id = (state.record as ClassRecord).id;
	const thread = coroutine.create(() => {
		setClaim(state);
		return new klass(...args);
	});
	const [ok, result] = coroutine.resume(thread);
	const yielded = ok && coroutine.status(thread) !== "dead";
	if (yielded) coroutine.close(thread);
	clearClaim(thread);
	if (yielded) error(`${id}'s constructor yielded; boot faults a constructor that yields`, 0);
	if (!ok) error(result, 0);
	return result as T;
}

/**
 * Builds one Subsystem with the constructor arguments given, without `boot`. Its `receive()` fields register, and
 * the spec calls its hooks as methods.
 */
export function subsystem<T extends Subsystem, A extends unknown[]>(klass: new (...args: A) => T, ...args: A): T {
	const record = recordFor(klass);
	if (record.kind !== Kind.Subsystem) error(`${record.id} is a behaviour; build it with behaviour()`, 0);
	openClaims({ side: game.GetService("RunService").IsClient() ? "client" : "server", retired: () => {} });
	const [ok, result] = pcall(construct<T, A>, klass, track(record), args);
	closeClaims(false);
	if (!ok) error(result, 0);
	return result as T;
}

/**
 * Builds one behaviour bound to `instance`, which may be a plain table. Each `attribute()` field reads the instance
 * through `GetAttribute` and is refused as binding refuses it.
 */
export function behaviour<T extends CrucibleBehaviour<string>, A extends unknown[]>(
	klass: new (...args: A) => T,
	instance: Instance,
	...args: A
): T {
	const record = recordFor(klass);
	if (record.kind !== Kind.Behaviour) error(`${record.id} is a Subsystem; build it with subsystem()`, 0);
	let attributes: Record<string, unknown> | undefined;
	if (record.attributes !== undefined) {
		attributes = {};
		for (const [field, descriptor] of record.attributes) {
			const value = instance.GetAttribute(descriptor.name);
			if (!(value === undefined ? !descriptor.required : descriptor.check(value)))
				error(`${record.id}: attribute ${descriptor.name} is invalid (${typeOf(value)})`, 0);
			attributes[field] = value;
		}
	}
	return construct(klass, track(record, instance, attributes), args);
}

/** The callback a built Subsystem's `receive()` field registered for `message`. */
export function receiver(message: unknown): ((...args: never[]) => unknown) | undefined {
	return receiverOf(definitionFor(message))?.callback;
}

/** The name, kind and schema nodes of a declared message. */
export function message(
	token: unknown,
): { name: string; kind: MessageKind; input: Branch; output?: Branch } | undefined {
	const definition = definitionOf(token);
	if (definition === undefined) return undefined;
	return {
		name: definition.name,
		kind: definition.kind,
		input: definition.input.node,
		output: definition.output?.node,
	};
}

function failure(err: unknown): string {
	if (typeIs(err, "table") && (err as Partial<CodecFailure>).codecFailure === true) {
		const { path, reason } = err as CodecFailure;
		return `${path}: ${reason}`;
	}
	return tostring(err);
}

/** Sends `value` through the message's compiled codec and back, as the wire does, and returns what arrives. */
export function transfer<T>(token: unknown, value: T, which: "input" | "output" = "input"): T {
	const definition = definitionFor(token);
	const codec = definition[which] ?? error(`${definition.name} has no ${which} codec`, 0);
	const writer = new Writer();
	const [encoded, encodeError] = pcall(codec.encode, writer, value, "payload");
	if (!encoded) error(`${definition.name} refused to encode ${failure(encodeError)}`, 0);
	const [packet, sidecar, slots] = writer.finish(buffer.create(0));
	const reader = new Reader(packet, 0, sidecar, slots);
	const [decoded, result] = pcall(codec.decode, reader, "payload");
	if (!decoded) error(`${definition.name} failed to decode ${failure(result)}`, 0);
	if (!reader.finished()) error(`${definition.name} left trailing bytes or slots`, 0);
	return result as T;
}

/** Seals the message catalog and lists the names in wire order. It seals once per module graph. */
export function sealMessages(): readonly string[] {
	const catalog = seal();
	return catalog === "" ? [] : catalog.split("\n");
}

/** What the transformer registered for a class. */
export function registration(
	klass: object,
):
	| { id: string; kind: "subsystem" }
	| { id: string; kind: "behaviour"; tag: string; instanceClasses: readonly string[] } {
	const record = recordFor(klass);
	if (record.kind === Kind.Subsystem) return { id: record.id, kind: "subsystem" };
	return { id: record.id, kind: "behaviour", tag: record.tag, instanceClasses: record.instanceClasses };
}
