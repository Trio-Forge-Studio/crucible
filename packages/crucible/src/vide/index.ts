import { cleanup, source, type Source } from "@rbxts/vide";
import { untracked, type Readable } from "../reactive";
import { createRootBinding } from "../internal/reactive/flush";
import { retire, type BindingNode } from "../internal/reactive/ownership";

const NO_OWNER = "useReadable() needs a Vide owner: call it inside a component or root()";

let pendingSource: Source<unknown> | undefined;
let pendingValue: unknown;

function write(): undefined {
	const target = pendingSource!;
	const value = pendingValue;
	pendingSource = undefined;
	pendingValue = undefined;
	target(value);
	return undefined;
}

function deliver(target: Source<unknown>, value: unknown) {
	pendingSource = target;
	pendingValue = value;
	// Vide runs its effects inside the write. Across gsub's C call a yield in one throws where it yields, so Vide's
	// own error path pops its scope stack. Without it the yield would suspend the reactive flush mid-effect.
	"x".gsub("x", write);
}

/**
 * A Vide source that follows `readable`. It holds the value read now, untracked, and takes each change on the
 * reactive flush, through a binding that ends with the current Vide owner. Outside a Vide component or `root` it
 * throws.
 */
export function useReadable<T>(readable: Readable<T>): () => T {
	let binding: BindingNode | undefined;
	// Vide's cleanup is the only public test for an owner, and registering first leaves nothing to undo.
	const [owned] = pcall(cleanup, () => {
		if (binding !== undefined) retire(binding);
	});
	if (!owned) error(NO_OWNER, 2);
	const initial = untracked(readable);
	const value = source(initial);
	// A root, so a component mounted from inside a Crucible run or callback does not end with it.
	binding = createRootBinding(readable, initial, deliver, value);
	return value;
}
