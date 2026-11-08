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

/**
 * @internal
 * Visits a quiet Heartbeat's scan makes before it reads another peer, one per peer and one per projection, at least
 * one peer. A threshold, not a ceiling: a scan makes fewer than this plus one peer's visits.
 */
export const SYNC_HEARTBEAT_SCAN = 1_024;

/** @internal The last revision a peer's frames may carry. */
export const SYNC_MAX_REVISION = 4_294_967_295;
