export const MAX_CONFIG_DECLARATIONS = 1_000;
export const CONFIG_NAME_MAX_BYTES = 64;
export const CONFIG_VALUE_MAX_BYTES = 100_000;
export const CONFIG_ADMISSION_VISITS = 100_000;
export const CONFIG_REQUEST_CONCURRENCY = 4;
export const CONFIG_FETCH_RETRY_BASE_SECONDS = 1;
export const CONFIG_FETCH_RETRY_CAP_SECONDS = 30;
export const CONFIG_REFRESH_TIMEOUT_SECONDS = 10;

// Rule 97 admits configs by rule 76's walk, whose depth and string bounds are data's PERSIST_DEPTH and
// PERSIST_STRING_BYTES. Data's limits module is not under configs yet, so the values are restated here.
export const CONFIG_VALUE_DEPTH = 28;
export const CONFIG_STRING_MAX_BYTES = 65_536;
