function walk<T extends defined>(items: readonly (T | undefined)[], stop: number, visit: (item: T) => void) {
	for (let i = 0; i < stop; i++) {
		const item = items[i];
		if (item !== undefined) visit(item);
	}
}

/**
 * @internal
 * Ordered list with O(1) append, O(1) removal by tombstone, and allocation-free iteration. Holes are
 * `end - count`. An append always writes at `end`, so no slot is reused until compaction, which runs outside a
 * walk once holes pass half the slots. Outside a walk `end <= 2 * count`, so a walk touches at most twice the
 * members and compaction costs at most twice the removals. Iteration order is insertion order and survives any
 * mutation from inside a visit. Items appended during `forEach` are visited by the next call, not this one.
 */
export class DenseList<T extends defined> {
	private readonly items = new Array<T | undefined>();
	private readonly slots = new Map<T, number>();
	private end = 0;
	private count = 0;
	private depth = 0;

	size(): number {
		return this.count;
	}

	has(item: T): boolean {
		return this.slots.has(item);
	}

	/** Idempotent. Returns false when the item was already present. */
	add(item: T): boolean {
		if (this.slots.has(item)) return false;
		this.items[this.end] = item;
		this.slots.set(item, this.end);
		this.end += 1;
		this.count += 1;
		return true;
	}

	/** Idempotent. Returns false when the item was absent. */
	remove(item: T): boolean {
		const slot = this.slots.get(item);
		if (slot === undefined) return false;
		this.items[slot] = undefined;
		this.slots.delete(item);
		this.count -= 1;
		this.compactIfSparse();
		return true;
	}

	/** Rethrows a visit's error after restoring the walk state. */
	forEach(visit: (item: T) => void) {
		this.depth += 1;
		const [ok, err] = pcall(walk, this.items, this.end, visit);
		this.depth -= 1;
		this.compactIfSparse();
		if (!ok) error(err, 0);
	}

	first(): T | undefined {
		for (let i = 0; i < this.end; i++) {
			const item = this.items[i];
			if (item !== undefined) return item;
		}
		return undefined;
	}

	private compactIfSparse() {
		if (this.depth > 0 || this.end - this.count <= this.end / 2) return;
		let write = 0;
		for (let read = 0; read < this.end; read++) {
			const item = this.items[read];
			if (item === undefined) continue;
			if (read !== write) {
				this.items[write] = item;
				this.items[read] = undefined;
				this.slots.set(item, write);
			}
			write += 1;
		}
		this.end = write;
	}
}
