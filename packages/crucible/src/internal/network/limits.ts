export const PROTOCOL_VERSION = 1;
export const REMOTE_NAME = "CrucibleNetwork";
export const PROTOCOL_ATTRIBUTE = "Protocol";
export const CATALOG_ATTRIBUTE = "Catalog";
export const KICK_MESSAGE = "Connection problem, please rejoin.";

export const MESSAGE_NAME_MAX_BYTES = 64;
export const MAX_MESSAGES = 1024;

export const CLIENT_DISCOVERY_TIMEOUT_SECONDS = 30;

export const MAX_QUEUED_PACKETS = 256;
export const MAX_QUEUED_BYTES = 1_048_576;
export const QUEUED_SLOT_CHARGE_BYTES = 64;

export const MAX_PACKET_BYTES = 262_144;
export const MAX_INSTANCE_SLOTS = 256;
export const MAX_DEPTH = 32;
export const MAX_VALUE_VISITS = 65_536;
export const MAX_COLLECTION_ENTRIES = 16_384;
export const MAX_STRING_BYTES = 65_536;
export const VARUINT_MAX_BYTES = 5;

export const MAX_UNION_BRANCHES = 256;
export const MAX_SCHEMA_NODES = 4096;
export const CFRAME_TOLERANCE = 0.001;

export const MAX_ACTIVE_HANDLERS = 64;
export const MAX_PENDING_ASKS = 64;
export const HANDLER_LIFETIME_SECONDS = 30;
export const DEFAULT_ASK_TIMEOUT_SECONDS = 10;
export const MAX_ASK_TIMEOUT_SECONDS = 120;
