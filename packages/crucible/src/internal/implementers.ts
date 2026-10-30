import { runtimeDiagnostic } from "./diagnostics";
import type { All } from "../all";
import { DenseList } from "./dense-list";
import { current, hasEnded, stateOf, within } from "./lifecycle";
import { Hook, type ObjectState } from "./state";
import { type EnginePort } from "./port";
import { CallResult } from "./runner";
import type { TypeId, ClassRecord } from "./registry";

interface Observer<T> {
	readonly added: (item: T) => void;
	readonly removed?: (item: T) => void;
	readonly owner?: ObjectState;
}

let port: EnginePort | undefined;

/** @internal */
export function useImplementerPort(value: EnginePort) {
	port = value;
}

function visit<T>(callback: (item: T) => void, item: T) {
	callback(item);
}
function label(state: ObjectState | undefined): string {
	return state === undefined ? "external" : (state.record as ClassRecord).id;
}

/** @internal */
export class ImplementerSet<T extends object> implements All<T> {
	private readonly members = new DenseList<T>();
	private readonly observers = new DenseList<Observer<T>>();
	constructor(readonly id: TypeId) {}
	size(): number {
		return this.members.size();
	}
	has(item: T): boolean {
		return this.members.has(item);
	}
	first(): T | undefined {
		return this.members.first();
	}
	forEach(visitor: (item: T) => void) {
		const owner = current();
		this.members.forEach((member) => this.deliver(owner, visitor, member));
	}
	observe(added: (item: T) => void, removed?: (item: T) => void): () => void {
		const owner = current();
		if (owner !== undefined && hasEnded(owner)) return () => {};
		const observer: Observer<T> = { added, removed, owner };
		this.observers.add(observer);
		let connected = true;
		const disconnect = () => {
			if (!connected) return;
			connected = false;
			this.observers.remove(observer);
		};
		if (owner !== undefined) {
			if (owner.subscriptions === undefined) owner.subscriptions = [];
			owner.subscriptions.push(disconnect);
		}
		this.members.forEach((member) => {
			if (connected) this.deliver(owner, added, member);
		});
		return disconnect;
	}
	join(item: T) {
		if (!this.members.add(item)) return;
		this.observers.forEach((observer) => this.deliver(observer.owner, observer.added, item));
	}
	leave(item: T) {
		if (!this.members.remove(item)) return;
		this.observers.forEach((observer) => {
			if (observer.removed !== undefined) this.deliver(observer.owner, observer.removed, item);
		});
	}
	private deliver(owner: ObjectState | undefined, callback: (item: T) => void, member: T) {
		const [result, failure] = within(owner, undefined, Hook.Callback, visit, callback, member);
		if (result !== CallResult.Ok) {
			const message = `Crucible All<${this.id}> subscriber ${label(owner)}, member ${label(stateOf(member))}: ${tostring(failure)}`;
			if (port === undefined) error("All callback failed before useImplementerPort: " + message);
			runtimeDiagnostic("error", message);
		}
	}
}
