import {
	effect,
	endBatch,
	getActiveSub,
	ReactiveFlags,
	setActiveSub,
	signal,
	startBatch,
	type ReactiveNode,
} from "@rbxts/charm";

/** @internal The Charm release the runtime's specs run against, and its exact peer version. */
export const CHARM_VERSION = "0.11.1";

let checked = false;

function refuse(what: string): never {
	error(
		`Crucible needs @rbxts/charm ${CHARM_VERSION}, and the installed Charm ${what}. Install the version that @trioforge/crucible's peerDependencies names.`,
		0,
	);
}

/** Charm holds a write's effects until the outermost batch or the running flush ends, and exposes neither. */
function unflushed(): never {
	error(
		`Crucible checks @rbxts/charm as it first loads, and Charm ran no effect for the check's write. Charm waits inside a batch or an effect, so require @trioforge/crucible outside both. Otherwise install @rbxts/charm ${CHARM_VERSION}, the version that @trioforge/crucible's peerDependencies names.`,
		0,
	);
}

function holds(node: ReactiveNode, dep: ReactiveNode): boolean {
	let link = node.deps;
	while (link !== undefined) {
		if (link.dep === dep) return true;
		link = link.nextDep;
	}
	return false;
}

/**
 * A nested effect whose node forgets its parent link stops at once, and the parent's next run drops the link. Its
 * write is the check's first, so it also finds a batch or a flush that holds the parent's run.
 */
function checkHalt() {
	const [read, write] = signal(0);
	let parent: ReactiveNode | undefined;
	let child: ReactiveNode | undefined;
	let stopChild: (() => void) | undefined;
	let runs = 0;
	const stopParent = effect(() => {
		runs += 1;
		read();
		if (parent !== undefined) return;
		parent = getActiveSub();
		stopChild = effect(() => {
			child = getActiveSub();
			read();
		});
	});
	const [ok, err] = pcall(() => {
		if (parent === undefined || child === undefined || stopChild === undefined) refuse("does not run a new effect");
		if (child.subs?.sub !== parent || !holds(parent, child) || child.deps === undefined)
			refuse("links a nested effect differently");
		child.subs = undefined;
		child.subsTail = undefined;
		stopChild();
		if (child.deps !== undefined || child.flags !== ReactiveFlags.None || !holds(parent, child))
			refuse("stops a nested effect differently");
		write(1);
		if (runs === 1) unflushed();
		if (holds(parent, child) || parent.deps === undefined) refuse("drops a stopped nested effect differently");
	});
	// The parent's stop unlinks a child that still holds its link, so the child never reaches the wrong-parent unlink.
	stopParent();
	stopChild?.();
	if (!ok) error(err, 0);
	if (parent!.deps !== undefined) refuse("stops an effect differently");
}

/** An effect that a flush skips because an earlier one threw still runs, or keeps a Dirty or Pending flag. */
function checkSkip() {
	const [read, write] = signal(0);
	let threw = false;
	const stopThrower = effect(() => {
		if (read() === 0) return;
		threw = true;
		error("Crucible's Charm check", 0);
	});
	let node: ReactiveNode | undefined;
	let runs = 0;
	const stopWatcher = effect(() => {
		node = getActiveSub();
		read();
		runs += 1;
	});
	startBatch();
	write(1);
	pcall(endBatch);
	const kept = runs === 2 || bit32.btest(node!.flags, ReactiveFlags.Dirty | ReactiveFlags.Pending);
	stopThrower();
	stopWatcher();
	if (!threw) unflushed();
	if (!kept) refuse("forgets a write that a throwing effect kept from another effect");
}

/**
 * @internal
 * Fails the runtime's load unless Charm behaves as ownership and ServerSync assume: they read and edit its nodes,
 * which no Charm API covers. Untracked, and passes once per realm. A failed check stops its effects and runs again on
 * the next call.
 */
export function checkCharm() {
	if (checked) return;
	if (!typeIs(ReactiveFlags.None, "number") || !typeIs(ReactiveFlags.Dirty, "number")) refuse("has no ReactiveFlags");
	const active = setActiveSub(undefined);
	const [ok, err] = pcall(() => {
		checkHalt();
		checkSkip();
	});
	setActiveSub(active);
	if (!ok) error(err, 0);
	checked = true;
}
