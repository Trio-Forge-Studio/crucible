declare const productKind: unique symbol;
declare const passKind: unique symbol;
declare const badgeKind: unique symbol;

/**
 * A developer product `crucible.toml` declares, imported from `crucible:experience` as `Products.<Key>`. `K` is its key
 * in `crucible.toml`.
 */
export interface ProductHandle<K extends string = string> {
	readonly [productKind]: K;
	readonly key: K;
	/** The product's Roblox ID in the environment the place was built for. Reading it throws when the build has none. */
	readonly id: number;
}

/** A game pass `crucible.toml` declares, imported from `crucible:experience` as `Passes.<Key>`. */
export interface PassHandle<K extends string = string> {
	readonly [passKind]: K;
	readonly key: K;
	/** The pass's Roblox ID in the environment the place was built for. Reading it throws when the build has none. */
	readonly id: number;
}

/** A badge `crucible.toml` declares, imported from `crucible:experience` as `Badges.<Key>`. */
export interface BadgeHandle<K extends string = string> {
	readonly [badgeKind]: K;
	readonly key: K;
	/** The badge's Roblox ID in the environment the place was built for. Reading it throws when the build has none. */
	readonly id: number;
}
