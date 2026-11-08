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

function holds(node: ReactiveNode, dep: ReactiveNode): boolean {
	let link = node.deps;
	while (link !== undefined) {
		if (link.dep === dep) return true;
		link = link.nextDep;
	}
	return false;
}

/**
 * A nested effect whose node forgets its parent link stops at once, and the parent's next run drops the link. When
 * the runtime loads inside a batch or a flush, that run waits, so its part is skipped.
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
	if (parent === undefined || child === undefined || stopChild === undefined) refuse("does not run a new effect");
	if (child.subs?.sub !== parent || !holds(parent, child) || child.deps === undefined)
		refuse("links a nested effect differently");
	child.subs = undefined;
	child.subsTail = undefined;
	stopChild();
	if (child.deps !== undefined || child.flags !== ReactiveFlags.None || !holds(parent, child))
		refuse("stops a nested effect differently");
	write(1);
	if (runs === 2 && (holds(parent, child) || parent.deps === undefined))
		refuse("drops a stopped nested effect differently");
	stopParent();
	if (parent.deps !== undefined) refuse("stops an effect differently");
}

/** An effect that a flush skips because an earlier one threw still runs, or keeps a Dirty or Pending flag. */
function checkSkip() {
	const [read, write] = signal(0);
	const stopThrower = effect(() => {
		if (read() !== 0) error("Crucible's Charm check", 0);
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
	if (!kept) refuse("forgets a write that a throwing effect kept from another effect");
}

/**
 * @internal
 * Fails the runtime's load unless Charm behaves as ownership and ServerSync assume: they read and edit its nodes,
 * which no Charm API covers. Runs once per realm, untracked.
 */
export function checkCharm() {
	if (checked) return;
	checked = true;
	if (!typeIs(ReactiveFlags.None, "number") || !typeIs(ReactiveFlags.Dirty, "number")) refuse("has no ReactiveFlags");
	const active = setActiveSub(undefined);
	checkHalt();
	checkSkip();
	setActiveSub(active);
}
