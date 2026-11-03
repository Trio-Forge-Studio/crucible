import { cleanup, source } from "@rbxts/vide";
import { untracked, watch, type Readable } from "../reactive";
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
	const value = source(untracked(readable));
	// A root, so a component mounted from inside a Crucible run or callback does not end with it.
	dispose = createRootScope(() => watch(readable, (current) => value(current)));
	return value;
}
