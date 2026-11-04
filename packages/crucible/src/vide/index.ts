import { cleanup, source } from "@rbxts/vide";
import { untracked, type Readable } from "../reactive";
import { createWatchFrom } from "../internal/reactive/flush";
import { createRootScope } from "../internal/reactive/ownership";

const NO_OWNER = "useReadable() needs a Vide owner: call it inside a component or root()";

/**
 * A Vide source that follows `readable`. It holds the value read now, untracked, and takes each change on the
 * reactive flush, through a `watch` that ends with the current Vide owner. Outside a Vide component or `root` it
 * throws.
 */
export function useReadable<T>(readable: Readable<T>): () => T {
	let dispose: (() => void) | undefined;
	// Vide's cleanup is the only public test for an owner, and registering first leaves nothing to undo.
	const [owned] = pcall(cleanup, () => dispose?.());
	if (!owned) error(NO_OWNER, 2);
	const initial = untracked(readable);
	const value = source(initial);
	let latest = initial;
	const write = (): undefined => {
		value(latest);
		return undefined;
	};
	// A root, so a component mounted from inside a Crucible run or callback does not end with it.
	dispose = createRootScope(() =>
		createWatchFrom(readable, initial, (current) => {
			latest = current;
			// Vide runs its effects inside the write. Across gsub's C call a yield in one throws where it yields, so
			// Vide's own error path pops its scope stack. Without it the runner cancels the yielded worker mid-effect.
			"x".gsub("x", write);
		}),
	);
	return value;
}
