import { source, untrack } from "@rbxts/vide";
import {
	add,
	close,
	closeAll,
	dropClosing,
	emptyToasts,
	findToast,
	setPaused,
	tick,
	update,
	type ToastAction,
	type ToastCloseReason,
	type ToastId,
	type ToastOverflow,
	type ToastPolicy,
	type ToastState,
	type ToastStep,
} from "./toast-state";

export interface ToasterOptions {
	limit?: number;
	overflow?: ToastOverflow;
	duration?: number;
	exitTimeout?: number;
}
export interface ToastOptions<D> {
	id?: ToastId;
	data: D;
	duration?: number;
	action?: ToastAction;
	onClose?: (reason: ToastCloseReason) => void;
}
export interface ToastRecord<D> {
	readonly id: ToastId;
	readonly data: D;
	readonly phase: "Queued" | "Open" | "Closing";
}
export interface Toaster<D> {
	readonly add: (options: ToastOptions<D>) => ToastId;
	readonly update: (id: ToastId, data: D | ((previous: D) => D)) => boolean;
	readonly close: (id: ToastId) => void;
	readonly clear: () => void;
	readonly pause: () => () => void;
	readonly toasts: () => readonly ToastRecord<D>[];
}
export interface ToasterCore<D> {
	readonly state: () => ToastState<D>;
	readonly tick: (now: number) => void;
	readonly act: (id: ToastId) => boolean;
	readonly dismiss: (id: ToastId, reason: "Dismiss" | "Swipe") => void;
	readonly attach: () => () => void;
	readonly holdExit: (id: ToastId, held: () => boolean) => () => void;
}

const cores = new WeakMap<object, ToasterCore<unknown>>();
let generated = 0;

export function toasterCore<D>(toaster: Toaster<D>): ToasterCore<D> {
	const found = cores.get(toaster);
	assert(found, "Toast.Viewport needs a toaster from createToaster");
	return found as ToasterCore<D>;
}

function validDuration(duration: number): number {
	assert(duration > 0, "Toast duration must be positive; use math.huge for a sticky toast");
	return duration;
}

export function createToaster<D>(options: ToasterOptions = {}): Toaster<D> {
	const policy: ToastPolicy = { limit: options.limit ?? 3, overflow: options.overflow ?? "Close" };
	const duration = validDuration(options.duration ?? 4);
	const exitTimeout = options.exitTimeout ?? 1;
	assert(policy.limit >= 1 && policy.limit % 1 === 0 && policy.limit < math.huge, "Toaster limit must be a count");
	assert(exitTimeout >= 0 && exitTimeout < math.huge, "Toaster exitTimeout must be finite and not negative");
	const state = source(emptyToasts<D>());
	const holds = new Set<object>();
	const exits = new Map<ToastId, () => boolean>();
	const running = new Set<string>();
	let attached = false;
	const now = () => os.clock();
	// Vide notifies on every write of a table, even the same one, and a frame's tick usually changes nothing.
	const write = (value: ToastState<D>) => {
		if (value !== untrack(state)) state(value);
	};
	const commit = (step: ToastStep<D>) => {
		write(attached ? step.state : dropClosing(step.state));
		for (const closed of step.closed) closed.onClose?.(closed.reason);
	};
	const hold = () => {
		const token = {};
		holds.add(token);
		if (holds.size() === 1) write(setPaused(untrack(state), true, now()));
		return () =>
			untrack(() => {
				if (holds.delete(token) && holds.isEmpty()) write(setPaused(state(), false, now()));
			});
	};
	// Timers count only while a viewport shows the toasts.
	let detached: (() => void) | undefined = hold();

	const toaster: Toaster<D> = {
		add: (given) =>
			untrack(() => {
				let id = given.id;
				if (id === undefined) {
					do id = `toast-${++generated}`;
					while (findToast(state(), id));
				}
				const input = {
					id,
					data: given.data,
					duration: validDuration(given.duration ?? duration),
					action: given.action,
					onClose: given.onClose,
				};
				commit(add(state(), input, policy, now()));
				return id;
			}),
		update: (id, data) =>
			untrack(() => {
				const current = findToast(state(), id);
				if (!current || current.phase.kind === "Closing") return false;
				const value = typeIs(data, "function") ? (data as (previous: D) => D)(current.data) : data;
				const updated = update(state(), id, value);
				if (updated) state(updated);
				return updated !== undefined;
			}),
		close: (id) => untrack(() => commit(close(state(), id, "Close", policy, now()))),
		clear: () => untrack(() => commit(closeAll(state(), "Close", now()))),
		pause: () => untrack(hold),
		toasts: () => state().entries.map((entry) => ({ id: entry.id, data: entry.data, phase: entry.phase.kind })),
	};
	const step = (at: number) =>
		commit(tick(state(), at, policy, { timeout: exitTimeout, held: (id) => exits.get(id)?.() === true }));
	const core: ToasterCore<D> = {
		state: () => state(),
		tick: (at) => untrack(() => step(at)),
		act: (id) =>
			untrack(() => {
				step(now());
				const entry = findToast(state(), id);
				if (!entry || entry.phase.kind !== "Open" || !entry.action) return false;
				const key = `${entry.revision} ${id}`;
				if (running.has(key)) return false;
				running.add(key);
				const resume = hold();
				let result: boolean | void;
				try {
					result = entry.action();
				} finally {
					running.delete(key);
					resume();
				}
				const current = findToast(state(), id);
				if (result === false || current?.phase.kind !== "Open" || current.revision !== entry.revision)
					return false;
				commit(close(state(), id, "Action", policy, now()));
				return true;
			}),
		dismiss: (id, reason) => untrack(() => commit(close(state(), id, reason, policy, now()))),
		attach: () => {
			assert(!attached, "A toaster can be shown by one Toast.Viewport at a time");
			attached = true;
			detached?.();
			detached = undefined;
			return () =>
				untrack(() => {
					attached = false;
					detached = hold();
					write(dropClosing(state()));
				});
		},
		holdExit: (id, held) => {
			const registration = () => held();
			exits.set(id, registration);
			return () => {
				if (exits.get(id) === registration) exits.delete(id);
			};
		},
	};
	cores.set(toaster, core);
	return toaster;
}
