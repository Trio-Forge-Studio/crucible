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
	/**
	 * Whether the product is on sale in the environment the place was built for: false when it is retired or the
	 * environment's overlay sets `for_sale = false`. Reading it throws when the build has no environment.
	 */
	readonly forSale: boolean;
}

/**
 * A developer product `crucible.toml` retires. It is off sale, but a purchase made before can still send a receipt, so
 * a place may grant it once, or not at all. In an environment that never had it, its grant claims nothing.
 */
export interface RetiredProductHandle<K extends string = string> extends ProductHandle<K> {}

/** A game pass `crucible.toml` declares, imported from `crucible:experience` as `Passes.<Key>`. */
export interface PassHandle<K extends string = string> {
	readonly [passKind]: K;
	readonly key: K;
	/** The pass's Roblox ID in the environment the place was built for. Reading it throws when the build has none. */
	readonly id: number;
	/**
	 * Whether the pass is on sale in the environment the place was built for: false when it is retired or the
	 * environment's overlay sets `for_sale = false`. Reading it throws when the build has no environment.
	 */
	readonly forSale: boolean;
}

/** A badge `crucible.toml` declares, imported from `crucible:experience` as `Badges.<Key>`. */
export interface BadgeHandle<K extends string = string> {
	readonly [badgeKind]: K;
	readonly key: K;
	/** The badge's Roblox ID in the environment the place was built for. Reading it throws when the build has none. */
	readonly id: number;
}
