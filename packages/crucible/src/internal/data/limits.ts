/** @internal Lease and load (rules 19 to 24). */
export const LEASE_SECONDS = 90;
/** @internal */
export const LOCAL_AUTHORITY_SECONDS = 70;
/** @internal */
export const RENEW_INTERVAL_SECONDS = 20;
/** @internal */
export const RENEW_JITTER_SECONDS = 2;
/** @internal */
export const TAKEOVER_GRACE_SECONDS = 10;
/** @internal Assumed, not measured (rule 25). */
export const CLOCK_ERROR_ASSUMED_SECONDS = 5;
/** @internal Expiry 90 + grace 10 + clock error 5 = 105, plus one 5 s poll and request time. */
export const LOAD_DEADLINE_SECONDS = 120;
/** @internal The last entry repeats. */
export const LOAD_POLL_SCHEDULE_SECONDS = [1, 1, 2, 2, 3, 5];
/** @internal Each poll is multiplied by uniform [1 - this, 1 + this]. */
export const LOAD_POLL_JITTER = 0.25;
/** @internal */
export const LOAD_POLL_MIN_SECONDS = 0.25;
/** @internal A renewal whose safe bound is nearer than this dispatches as LeaseCritical (rule 34). */
export const LEASE_CRITICAL_SECONDS = 30;

/** @internal Saves and barriers (rules 37, 45). */
export const SAVE_TIMEOUT_DEFAULT_SECONDS = 10;
/** @internal */
export const SAVE_TIMEOUT_MAX_SECONDS = 30;
/** @internal */
export const HANDOFF_TIMEOUT_SECONDS = 10;
/** @internal How long a granting callback waits for its id to be durable (rule 63). */
export const RECEIPT_SAVE_TIMEOUT_SECONDS = 30;
/** @internal */
export const CALL_DEADLINE_SECONDS = 30;

/** @internal Scheduler (rules 32 to 36). */
export const MAX_IN_FLIGHT = 8;
/** @internal */
export const RESERVED_SESSION_SLOTS = 4;
/** @internal */
export const ORDINARY_MAX_IN_FLIGHT = 3;
/** @internal */
export const MAX_PENDING_OPERATIONS = 512;
/** @internal */
export const MAX_PENDING_PER_KEY = 32;
/** @internal */
export const BUDGET_RESERVE_FRACTION = 0.2;
/** @internal */
export const BUDGET_RESERVE_MIN = 12;
/** @internal */
export const ORDINARY_AGING_SECONDS = 5;
/** @internal */
export const RETRY_BASE_SECONDS = 0.5;
/** @internal */
export const RETRY_CAP_SECONDS = 8;
/** @internal */
export const KEY_WRITE_BYTES_PER_MINUTE = 3_145_728;
/** @internal */
export const KEY_READ_BYTES_PER_MINUTE = 20_971_520;
/** @internal The engine publishes no budget-change event, so blocked work rechecks at this period. */
export const BUDGET_RECHECK_SECONDS = 0.5;

/** @internal Sizes in encoded JSON bytes (rule 39). */
export const MAX_ENVELOPE_BYTES = 1_048_576;
/** @internal */
export const ENVELOPE_HEADER_MAX_BYTES = 4_096;
/** @internal */
export const PAYLOAD_MAX_BYTES = 262_144;
/** @internal */
export const RECORD_PAYLOAD_MAX_BYTES = 1_000_000;
/** @internal */
export const OPERATIONS_MAX_BYTES = 18_432;
/** @internal */
export const MAX_PURCHASE_IDS = 8_192;
/** @internal */
export const RECEIPTS_MAX_BYTES = 393_216;
/** @internal */
export const PURCHASE_ID_MAX_BYTES = 64;
/** @internal */
export const INBOX_MAX_MESSAGES = 128;
/** @internal */
export const INBOX_MAX_BYTES = 131_072;
/** @internal */
export const QUARANTINE_MAX_MESSAGES = 8;
/** @internal */
export const QUARANTINE_MAX_BYTES = 69_632;
/** @internal */
export const MESSAGE_PAYLOAD_MAX_BYTES = 8_192;
/** @internal Seven days (rule 69). */
export const MESSAGE_ID_MAX_AGE_SECONDS = 604_800;
/** @internal */
export const ID_FUTURE_SKEW_SECONDS = 5;
/** @internal The 30 s call deadline plus 5 s of clock error, with margin (rule 69). */
export const ID_EVICT_MIN_AGE_SECONDS = 60;
/** @internal */
export const OPERATION_ID_MAX_BYTES = 64;
/** @internal */
export const SEEN_MAX_MESSAGES = 1_024;
/** @internal */
export const SEEN_MAX_BYTES = 69_632;
/** @internal The wait after attempt n is entry n - 1; the last repeats (rule 73). */
export const INBOX_RETRY_SECONDS = [5, 15, 60, 60];
/** @internal Attempts before an item is quarantined with reason "attempts" (rule 73). */
export const INBOX_MAX_ATTEMPTS = 5;
/** @internal */
export const OPERATION_MARKERS_PER_KEY = 256;

/** @internal Shutdown against core's sweep (rules 50 to 56). */
export const FINAL_PHASE_SECONDS = 10;
/** @internal */
export const DISPATCH_CUTOFF_SECONDS = 1.5;
/** @internal */
export const SETTLE_MARGIN_SECONDS = 0.5;
/** @internal */
export const SHUTDOWN_FALLBACK_SECONDS = 2;

/** @internal Kick messages (rules 20, 26, 47, 77). */
export const LOAD_BUSY_MESSAGE = "Your save is still open in another server. Please rejoin in a minute.";
/** @internal */
export const LOAD_FAILED_MESSAGE = "Your data could not load. Please rejoin.";
/** @internal */
export const SESSION_ENDED_MESSAGE = "Your data session ended. Please rejoin.";
/** @internal */
export const SERVER_UPDATING_MESSAGE = "This server is updating to a newer version of the game. Please rejoin.";

/** @internal Rule 39: both sums stay below MAX_ENVELOPE_BYTES. */
export const PROFILE_SECTIONS_BYTES =
	ENVELOPE_HEADER_MAX_BYTES +
	PAYLOAD_MAX_BYTES +
	RECEIPTS_MAX_BYTES +
	INBOX_MAX_BYTES +
	QUARANTINE_MAX_BYTES +
	SEEN_MAX_BYTES;
/** @internal */
export const RECORD_SECTIONS_BYTES = ENVELOPE_HEADER_MAX_BYTES + RECORD_PAYLOAD_MAX_BYTES + OPERATIONS_MAX_BYTES;
