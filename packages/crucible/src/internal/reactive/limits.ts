/** @internal Runs one effect may make in one flush before it stops as a feedback loop. */
export const MAX_RUNS_PER_EFFECT = 64;

/** @internal Effect runs in one flush, first runs included, before the rest waits for the next flush. */
export const MAX_EFFECT_RUNS_PER_FLUSH = 65_536;

/** @internal Rounds in one flush before the rest waits for the next flush. */
export const MAX_FLUSH_ROUNDS = 256;

/** @internal Live direct children of one generation lifetime before one warning names the owner. */
export const OWNER_CHILDREN_WARNING = 4_096;

/** @internal Reserved for the UI layer's render commit, `Enum.RenderPriority.Last.Value + 1`. Nothing binds it yet. */
export const RENDER_COMMIT_PRIORITY = 2_001;
