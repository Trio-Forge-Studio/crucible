local spec = require("./lib/load")
local Backend = require("./lib/data-backend")

local function realm()
	return Backend.new():realm()
end

local EXPECTED = {
	LEASE_SECONDS = 90,
	LOCAL_AUTHORITY_SECONDS = 70,
	RENEW_INTERVAL_SECONDS = 20,
	RENEW_JITTER_SECONDS = 2,
	TAKEOVER_GRACE_SECONDS = 10,
	CLOCK_ERROR_ASSUMED_SECONDS = 5,
	LOAD_DEADLINE_SECONDS = 120,
	LOAD_POLL_JITTER = 0.25,
	SAVE_TIMEOUT_DEFAULT_SECONDS = 10,
	SAVE_TIMEOUT_MAX_SECONDS = 30,
	HANDOFF_TIMEOUT_SECONDS = 10,
	CALL_DEADLINE_SECONDS = 30,
	MAX_IN_FLIGHT = 8,
	RESERVED_SESSION_SLOTS = 4,
	ORDINARY_MAX_IN_FLIGHT = 3,
	MAX_PENDING_OPERATIONS = 512,
	MAX_PENDING_PER_KEY = 32,
	BUDGET_RESERVE_FRACTION = 0.2,
	BUDGET_RESERVE_MIN = 12,
	ORDINARY_AGING_SECONDS = 5,
	RETRY_BASE_SECONDS = 0.5,
	RETRY_CAP_SECONDS = 8,
	KEY_WRITE_BYTES_PER_MINUTE = 3145728,
	KEY_READ_BYTES_PER_MINUTE = 20971520,
	MAX_ENVELOPE_BYTES = 1048576,
	ENVELOPE_HEADER_MAX_BYTES = 4096,
	PAYLOAD_MAX_BYTES = 262144,
	RECORD_PAYLOAD_MAX_BYTES = 1000000,
	OPERATIONS_MAX_BYTES = 18432,
	MAX_PURCHASE_IDS = 8192,
	RECEIPTS_MAX_BYTES = 393216,
	INBOX_MAX_MESSAGES = 128,
	INBOX_MAX_BYTES = 131072,
	QUARANTINE_MAX_MESSAGES = 8,
	QUARANTINE_MAX_BYTES = 69632,
	OPERATION_ID_MAX_BYTES = 64,
	SEEN_MAX_MESSAGES = 1024,
	SEEN_MAX_BYTES = 69632,
	FINAL_PHASE_SECONDS = 10,
	DISPATCH_CUTOFF_SECONDS = 1.5,
	SETTLE_MARGIN_SECONDS = 0.5,
	SHUTDOWN_FALLBACK_SECONDS = 2,
	LOAD_BUSY_MESSAGE = "Your save is still open in another server. Please rejoin in a minute.",
	LOAD_FAILED_MESSAGE = "Your data could not load. Please rejoin.",
	SESSION_ENDED_MESSAGE = "Your data session ended. Please rejoin.",
	SERVER_UPDATING_MESSAGE = "This server is updating to a newer version of the game. Please rejoin.",
}

local function owned(r, payload)
	local envelope = r.envelope.emptyEnvelope("profiles")
	envelope.epoch = 1
	envelope.lease = { token = "T", serverId = "s", jobId = "j", expiresAt = r.wall() + 90 }
	local decision
	local result = r.lease.ownedWrite({ value = envelope, userIds = {}, metadata = {} }, {
		userId = 1,
		token = "T",
		epoch = 1,
		op = 1,
		snapshot = { revision = 1, schemaVersion = 0, payload = payload, receipts = {}, acks = {}, marks = {} },
		release = false,
		wall = r.wall,
	}, function(d)
		decision = d
	end)
	return decision, result
end

return {
	["Limits rows hold the contract's values"] = function()
		local limits = realm().limits
		for name, value in EXPECTED do
			spec.eq(limits[name], value, name)
		end
		spec.list(limits.LOAD_POLL_SCHEDULE_SECONDS, { 1, 1, 2, 2, 3, 5 }, "LOAD_POLL_SCHEDULE_SECONDS")
	end,
	["rule 39 section caps sum below the envelope cap and a transform cancels over it"] = function()
		local r = realm()
		spec.eq(r.limits.PROFILE_SECTIONS_BYTES, 929792, "profile sections")
		spec.eq(r.limits.RECORD_SECTIONS_BYTES, 1022528, "record sections")
		assert(r.limits.PROFILE_SECTIONS_BYTES < r.limits.MAX_ENVELOPE_BYTES, "profile sections fit")
		assert(r.limits.RECORD_SECTIONS_BYTES < r.limits.MAX_ENVELOPE_BYTES, "record sections fit")
		local decision, result = owned(r, { blob = string.rep("x", 1048576) })
		spec.eq(decision.kind, "too-large", "an oversized envelope cancels")
		spec.eq(result, nil, "nothing written")
		local fits = owned(r, { blob = string.rep("x", 1000) })
		spec.eq(fits.kind, "written", "a fitting envelope writes")
		spec.eq(r.envelope.jsonBytes({ a = "x\n", b = { 1, 2 } }), #'{"a":"x\\n","b":[1,2]}', "encoded JSON bytes")
	end,
	["rule 40 malformed envelopes are unreadable"] = function()
		local r = realm()
		local function parse(change)
			local envelope = r.envelope.emptyEnvelope("profiles")
			change(envelope)
			return r.envelope.parseEnvelope(envelope, "profiles")
		end
		spec.eq(parse(function() end).kind, "ok", "a valid envelope parses")
		spec.eq(
			parse(function(e)
				e.receipts = { "a", "b" }
				e.seen = { "s" }
			end).kind,
			"ok",
			"a valid envelope with entries parses"
		)
		spec.eq(r.envelope.parseEnvelope(nil, "profiles").kind, "absent", "nil is absent")
		local cases = {
			format = function(e)
				e.format = 2
			end,
			kind = function(e)
				e.kind = "records"
			end,
			counter = function(e)
				e.epoch = -1
			end,
			fraction = function(e)
				e.schemaVersion = 1.5
			end,
			generation = function(e)
				e.inbox = { { id = "i", name = "m", version = 0, generation = -1, attempts = 0 } }
			end,
			cap = function(e)
				for index = 1, 129 do
					e.inbox[index] = { id = `i{index}`, name = "m", version = 0, generation = 0, attempts = 0 }
				end
			end,
			id = function(e)
				e.seen = { string.rep("x", 65) }
			end,
			emptyId = function(e)
				e.seen = { "" }
			end,
			payload = function(e)
				e.payload = "text"
			end,
			holes = function(e)
				e.receipts = { "a", nil, "c" }
			end,
		}
		for name, change in cases do
			spec.eq(parse(change).kind, "unreadable", name)
		end
	end,
	["rule 105 the port classifies engine errors"] = function()
		local port = realm().graph.module("internal/data/port")
		local function check(message, retryable, ambiguous)
			local outcome = port.classify(message)
			spec.eq(outcome.retryable, retryable, `{message} retryable`)
			spec.eq(outcome.ambiguous, ambiguous, `{message} ambiguous`)
		end
		check("301: GetAsync request dropped", true, false)
		check("306: throttled", true, false)
		check("502: API Services rejected request", true, true)
		check("no code at all", true, true)
		check("403: cannot write in Studio", false, false)
		check("300: not a throttle", false, false)
		check("511: attribute too large", true, true)
		spec.eq(
			realm().graph.module("internal/data/port").dataPort().serverId ~= nil,
			true,
			"the loader installed the port"
		)
	end,
}
