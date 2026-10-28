export interface All<T extends object> {
	size(): number;
	has(item: T): boolean;

	first(): T | undefined;

	forEach(visit: (item: T) => void): void;

	observe(added: (item: T) => void, removed?: (item: T) => void): () => void;
}
