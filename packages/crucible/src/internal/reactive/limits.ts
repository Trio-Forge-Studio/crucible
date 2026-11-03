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

/** @internal Replicated declarations one realm admits. */
export const MAX_REPLICATED_DECLARATIONS = 256;

/** @internal Table nesting one replicated value may use, at compile time and at admission. */
export const SYNC_VALUE_DEPTH = 28;

/** @internal Bytes in one string of a replicated value. */
export const SYNC_STRING_BYTES = 49_152;

/** @internal Bytes in one whole sync frame packet. One value can never exceed it. */
export const SYNC_FRAME_BYTES = 196_608;

/** @internal Collection entries in one sync frame. */
export const SYNC_FRAME_COLLECTION_ENTRIES = 12_000;

/** @internal Codec value visits in one sync frame. Also bounds one structural comparison. */
export const SYNC_FRAME_VALUE_VISITS = 48_000;

/** @internal Admission visits for one value: value nodes plus every table key touched. */
export const SYNC_ADMISSION_VISITS = 96_000;

/** @internal Visits one peer examination may spend, the snapshot included. */
export const SYNC_PEER_VISITS = 262_144;

/** @internal Frame bytes one Heartbeat sends to all peers, at least one frame. */
export const SYNC_HEARTBEAT_BYTES = 1_048_576;

/** @internal Peers one Heartbeat examines, at least one. */
export const SYNC_HEARTBEAT_PEERS = 64;

/** @internal A peer starts only while the Heartbeat has spent fewer visits than this. A threshold, not a ceiling. */
export const SYNC_HEARTBEAT_VISITS = 65_536;

/** @internal The last revision a peer's frames may carry. */
export const SYNC_MAX_REVISION = 4_294_967_295;
