export type ToastId = string;
export type ToastAction = () => boolean | void;
export type ToastCloseReason = "Timeout" | "Action" | "Dismiss" | "Swipe" | "Overflow" | "Close";
export type ToastOverflow = "Close" | "Queue";

type OpenPhase = { readonly kind: "Open"; readonly remaining: number; readonly since: number | undefined };
export type ToastPhase =
	| { readonly kind: "Queued" }
	| OpenPhase
	| { readonly kind: "Closing"; readonly reason: ToastCloseReason; readonly at: number };

export interface ToastEntry<D> {
	readonly id: ToastId;
	readonly serial: number;
	readonly revision: number;
	readonly data: D;
	readonly action: ToastAction | undefined;
	readonly onClose: ((reason: ToastCloseReason) => void) | undefined;
	readonly duration: number;
	readonly phase: ToastPhase;
}
export interface ToastState<D> {
	readonly entries: readonly ToastEntry<D>[];
	readonly paused: boolean;
	readonly serial: number;
}
export interface ToastPolicy {
	readonly limit: number;
	readonly overflow: ToastOverflow;
}
export interface ToastInput<D> {
	readonly id: ToastId;
	readonly data: D;
	readonly duration: number;
	readonly action?: ToastAction;
	readonly onClose?: (reason: ToastCloseReason) => void;
}
export interface ToastClosed {
	readonly id: ToastId;
	readonly reason: ToastCloseReason;
	readonly onClose: ((reason: ToastCloseReason) => void) | undefined;
}
export interface ToastStep<D> {
	readonly state: ToastState<D>;
	readonly closed: readonly ToastClosed[];
}
export interface ToastExit {
	readonly timeout: number;
	readonly held: (id: ToastId) => boolean;
}

export function emptyToasts<D>(): ToastState<D> {
	return { entries: [], paused: false, serial: 0 };
}
export function findToast<D>(state: ToastState<D>, id: ToastId): ToastEntry<D> | undefined {
	return state.entries.find((entry) => entry.id === id);
}
function left(phase: OpenPhase, now: number): number {
	return phase.since === undefined ? phase.remaining : phase.remaining - (now - phase.since);
}
function opened<D>(entry: ToastEntry<D>, paused: boolean, now: number): ToastEntry<D> {
	return { ...entry, phase: { kind: "Open", remaining: entry.duration, since: paused ? undefined : now } };
}
function closing<D>(entry: ToastEntry<D>, reason: ToastCloseReason, now: number): ToastEntry<D> {
	return { ...entry, phase: { kind: "Closing", reason, at: now } };
}
function report<D>(entry: ToastEntry<D>, reason: ToastCloseReason): ToastClosed {
	return { id: entry.id, reason, onClose: entry.onClose };
}
function openCount<D>(entries: readonly ToastEntry<D>[]): number {
	return entries.filter((entry) => entry.phase.kind === "Open").size();
}
function closeAt<D>(
	entries: ToastEntry<D>[],
	index: number,
	reason: ToastCloseReason,
	now: number,
	closed: ToastClosed[],
): void {
	const entry = entries[index];
	closed.push(report(entry, reason));
	if (entry.phase.kind === "Queued") entries.remove(index);
	else entries[index] = closing(entry, reason, now);
}
// `spare` is the toast whose arrival or revival caused the overflow, so it never makes room for itself.
function admit<D>(
	entries: ToastEntry<D>[],
	closed: ToastClosed[],
	policy: ToastPolicy,
	paused: boolean,
	now: number,
	spare?: ToastId,
): void {
	if (policy.overflow === "Queue") {
		let open = openCount(entries);
		for (let index = 0; index < entries.size() && open < policy.limit; index++) {
			if (entries[index].phase.kind !== "Queued") continue;
			entries[index] = opened(entries[index], paused, now);
			open++;
		}
		return;
	}
	while (openCount(entries) > policy.limit) {
		const candidates = entries.filter((entry) => entry.phase.kind === "Open" && entry.id !== spare);
		const victim = candidates.find((entry) => entry.duration < math.huge) ?? candidates[0];
		closeAt(entries, entries.indexOf(victim), "Overflow", now, closed);
	}
}

export function add<D>(state: ToastState<D>, input: ToastInput<D>, policy: ToastPolicy, now: number): ToastStep<D> {
	const entries = [...state.entries];
	const closed: ToastClosed[] = [];
	const index = entries.findIndex((entry) => entry.id === input.id);
	let serial = state.serial;
	if (index >= 0) {
		const existing = entries[index];
		const replaced: ToastEntry<D> = {
			...existing,
			revision: existing.revision + 1,
			data: input.data,
			action: input.action,
			onClose: input.onClose,
			duration: input.duration,
		};
		entries[index] = existing.phase.kind === "Queued" ? replaced : opened(replaced, state.paused, now);
	} else {
		serial++;
		const entry: ToastEntry<D> = {
			id: input.id,
			serial,
			revision: 0,
			data: input.data,
			action: input.action,
			onClose: input.onClose,
			duration: input.duration,
			phase: { kind: "Queued" },
		};
		entries.push(policy.overflow === "Queue" ? entry : opened(entry, state.paused, now));
	}
	admit(entries, closed, policy, state.paused, now, input.id);
	return { state: { ...state, entries, serial }, closed };
}

export function update<D>(state: ToastState<D>, id: ToastId, data: D): ToastState<D> | undefined {
	const index = state.entries.findIndex((entry) => entry.id === id && entry.phase.kind !== "Closing");
	if (index < 0) return undefined;
	const entries = [...state.entries];
	entries[index] = { ...entries[index], data, revision: entries[index].revision + 1 };
	return { ...state, entries };
}

export function close<D>(
	state: ToastState<D>,
	id: ToastId,
	reason: ToastCloseReason,
	policy: ToastPolicy,
	now: number,
): ToastStep<D> {
	const index = state.entries.findIndex((entry) => entry.id === id);
	if (index < 0 || state.entries[index].phase.kind === "Closing") return { state, closed: [] };
	const entries = [...state.entries];
	const closed: ToastClosed[] = [];
	closeAt(entries, index, reason, now, closed);
	admit(entries, closed, policy, state.paused, now);
	return { state: { ...state, entries }, closed };
}

export function closeAll<D>(state: ToastState<D>, reason: ToastCloseReason, now: number): ToastStep<D> {
	const entries: ToastEntry<D>[] = [];
	const closed: ToastClosed[] = [];
	for (const entry of state.entries) {
		if (entry.phase.kind !== "Closing") closed.push(report(entry, reason));
		if (entry.phase.kind === "Open") entries.push(closing(entry, reason, now));
		else if (entry.phase.kind === "Closing") entries.push(entry);
	}
	return closed.isEmpty() ? { state, closed } : { state: { ...state, entries }, closed };
}

export function setPaused<D>(state: ToastState<D>, paused: boolean, now: number): ToastState<D> {
	if (state.paused === paused) return state;
	const entries = state.entries.map((entry): ToastEntry<D> => {
		const phase = entry.phase;
		if (phase.kind !== "Open") return entry;
		return {
			...entry,
			phase: paused
				? { kind: "Open", remaining: left(phase, now), since: undefined }
				: { kind: "Open", remaining: phase.remaining, since: now },
		};
	});
	return { ...state, paused, entries };
}

// Only toasts that were already closing when the tick began can leave, so a toast that times out now stays mounted
// for at least one frame of its exit.
export function tick<D>(state: ToastState<D>, now: number, policy: ToastPolicy, exit: ToastExit): ToastStep<D> {
	const entries: ToastEntry<D>[] = [];
	const closed: ToastClosed[] = [];
	let changed = false;
	for (const entry of state.entries) {
		const phase = entry.phase;
		if (phase.kind === "Open" && left(phase, now) <= 0) {
			closed.push(report(entry, "Timeout"));
			entries.push(closing(entry, "Timeout", now));
			changed = true;
		} else if (phase.kind === "Closing" && (now - phase.at >= exit.timeout || !exit.held(entry.id))) changed = true;
		else entries.push(entry);
	}
	if (!changed) return { state, closed };
	admit(entries, closed, policy, state.paused, now);
	return { state: { ...state, entries }, closed };
}

export function dropClosing<D>(state: ToastState<D>): ToastState<D> {
	const entries = state.entries.filter((entry) => entry.phase.kind !== "Closing");
	return entries.size() === state.entries.size() ? state : { ...state, entries };
}

export function needsTick<D>(state: ToastState<D>): boolean {
	return state.entries.some(
		(entry) =>
			entry.phase.kind === "Closing" ||
			(entry.phase.kind === "Open" && entry.phase.since !== undefined && entry.phase.remaining < math.huge),
	);
}
