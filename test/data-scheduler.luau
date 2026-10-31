local spec = require("./lib/load")
local Backend = require("./lib/data-backend")

local LEASE_CRITICAL, RELEASE, RECEIPT, RENEW, LOAD, ORDINARY = 0, 1, 2, 3, 4, 5

local function world()
	local backend = Backend.new()
	local realm = backend:realm()
	return backend, realm, realm.data.scheduler
end

local function request(realm, key: string, priority: number, options)
	options = options or {}
	return {
		store = options.store or "S",
		key = key,
		priority = priority,
		kind = options.kind or "update",
		dispatchDeadline = realm.clock() + (options.deadline or 60),
		safeUntil = options.safeUntil,
		renewal = options.renewal,
		bytes = options.bytes or 100,
		transform = function()
			return function()
				return { value = { key = key }, userIds = {}, metadata = {} }
			end
		end,
	}
end

local function submit(realm, scheduler, key, priority, options)
	local ticket
	realm.run(function()
		ticket = scheduler:submit(request(realm, key, priority, options))
	end)
	return ticket
end

local function dispatched(backend, key: string): number
	return #backend:callsTo({ key = key })
end

local function order(backend): { string }
	local keys = {}
	for _, call in backend.calls do
		table.insert(keys, call.key)
	end
	return keys
end

return {
	["rule 33 budget at zero holds urgent work until refilled"] = function()
		local backend, realm, scheduler = world()
		realm.budget.update = 0
		submit(realm, scheduler, "renew", RENEW)
		backend:advance(2)
		spec.eq(dispatched(backend, "renew"), 0, "no dispatch at a zero budget")
		realm.budget.update = 10
		backend:advance(0.6)
		spec.eq(dispatched(backend, "renew"), 1, "dispatched once refilled")
	end,
	["rule 33 B3 ordinary saturation waits above the reserve while a renewal dispatches"] = function()
		local backend, realm, scheduler = world()
		spec.eq(scheduler:reserve(), 12, "reserve floor with no players")
		for _ = 1, 30 do
			realm.join(100 + realm.population)
		end
		spec.eq(scheduler:reserve(), 252, "20 % of 60 + 40 x 30")
		realm.budget.update = 252
		realm.budget.read = 1000
		for index = 1, 5 do
			submit(realm, scheduler, `ordinary{index}`, ORDINARY)
		end
		submit(realm, scheduler, "renew", RENEW, { renewal = true, safeUntil = realm.clock() + 60 })
		backend:advance(1)
		spec.eq(dispatched(backend, "renew"), 1, "the renewal dispatched")
		spec.eq(dispatched(backend, "ordinary1"), 0, "ordinary waits at the reserve")
		realm.budget.update = 253
		backend:advance(1)
		spec.eq(dispatched(backend, "ordinary1"), 1, "ordinary dispatches above the reserve")
		realm.budget.update = 1000
		realm.budget.read = 252
		submit(realm, scheduler, "ordinary-read-budget", ORDINARY)
		backend:advance(1)
		spec.eq(dispatched(backend, "ordinary-read-budget"), 0, "an ordinary update also needs read budget above it")
	end,
	["rule 32 K5 eight stalled ordinary requests leave renewals and releases their slots"] = function()
		local backend, realm, scheduler = world()
		backend:fail("stall", { kind = "read" }, 8, { seconds = 80 })
		for index = 1, 8 do
			submit(realm, scheduler, `slow{index}`, ORDINARY, { kind = "read", deadline = 120 })
		end
		backend:advance(1)
		spec.eq(#backend:callsTo({ kind = "read" }), 3, "Ordinary holds at most three slots")
		backend:fail("stall", { key = "load1" }, 1, { seconds = 80 })
		submit(realm, scheduler, "load1", LOAD, { deadline = 120 })
		submit(realm, scheduler, "load2", LOAD, { deadline = 120 })
		backend:advance(1)
		spec.eq(dispatched(backend, "load1"), 1, "a load keeps the fourth shared slot")
		spec.eq(dispatched(backend, "load2"), 0, "Load and Ordinary share four slots")
		for index = 1, 3 do
			submit(realm, scheduler, `renew{index}`, RENEW, { renewal = true, safeUntil = realm.clock() + 60 })
		end
		submit(realm, scheduler, "release", RELEASE, { safeUntil = realm.clock() + 60 })
		backend:advance(1)
		for index = 1, 3 do
			spec.eq(dispatched(backend, `renew{index}`), 1, `renewal {index} dispatched`)
		end
		spec.eq(dispatched(backend, "release"), 1, "the release dispatched")
		backend:advance(2)
		submit(realm, scheduler, "release2", RELEASE, { safeUntil = realm.clock() + 60 })
		backend:advance(1)
		spec.eq(dispatched(backend, "release2"), 1, "owned writes keep dispatching while ordinary stalls")
	end,
	["rule 35 an unresolved request holds its lane past its caller's deadline"] = function()
		local backend, realm, scheduler = world()
		local gate = backend:gate("response", { key = "K" })
		local first = submit(realm, scheduler, "K", ORDINARY, { deadline = 2 })
		backend:advance(0.1)
		local waited = realm.run(function()
			return first:wait(realm.clock() + 2) or "pending"
		end)
		backend:advance(3)
		spec.eq(waited.value, "pending", "the caller settled at its deadline")
		spec.eq(first.dispatched and first.inFlight, true, "the request is still in flight")
		local second = submit(realm, scheduler, "K", RENEW)
		backend:advance(5)
		spec.eq(dispatched(backend, "K"), 1, "the later request never overtook it")
		gate.release()
		backend:advance(1)
		spec.eq(dispatched(backend, "K"), 2, "dispatched once the first returned")
		spec.eq(second.settled.kind, "done", "settled")
	end,
	["rule 35 caller timeout versus uncertain"] = function()
		local backend, realm, scheduler = world()
		realm.budget.update = 0
		local never = submit(realm, scheduler, "never", RENEW, { deadline = 2 })
		backend:advance(3)
		spec.eq(never.settled.kind, "timeout", "never dispatched: timeout")
		spec.eq(dispatched(backend, "never"), 0, "nothing was sent")
		realm.budget.update = 1000
		backend:fail("transient", { key = "maybe" }, 99)
		local maybe = submit(realm, scheduler, "maybe", RENEW, { deadline = 3 })
		backend:advance(4)
		spec.eq(maybe.settled.kind, "uncertain", "dispatched and ambiguous: uncertain")
		backend:fail("throttle", { key = "sent" }, 99)
		local throttled = submit(realm, scheduler, "sent", RENEW, { deadline = 3 })
		backend:advance(4)
		spec.eq(throttled.settled.kind, "timeout", "a throttle proves nothing was sent")
	end,
	["rule 32 pending caps refuse new callers with overloaded"] = function()
		local backend, realm, scheduler = world()
		realm.budget.update = 0
		for _ = 1, 32 do
			spec.eq(submit(realm, scheduler, "K", ORDINARY).settled, nil, "queued")
		end
		spec.eq(submit(realm, scheduler, "K", ORDINARY).settled.kind, "overloaded", "per-key cap")
		for index = 1, 480 do
			submit(realm, scheduler, `k{index % 20}`, LOAD)
		end
		spec.eq(submit(realm, scheduler, "other", ORDINARY).settled.kind, "overloaded", "server-wide cap")
		spec.eq(submit(realm, scheduler, "save", RENEW).settled, nil, "session saves do not count")
		backend:advance(0)
	end,
	["rule 34 dispatch order and aging"] = function()
		local backend, realm, scheduler = world()
		realm.budget.update = 0
		submit(realm, scheduler, "ordinary", ORDINARY)
		submit(realm, scheduler, "load", LOAD)
		submit(realm, scheduler, "renew", RENEW, { renewal = true, safeUntil = realm.clock() + 60 })
		submit(realm, scheduler, "receipt", RECEIPT)
		submit(realm, scheduler, "release-late", RELEASE, { safeUntil = realm.clock() + 50 })
		submit(realm, scheduler, "release-near", RELEASE, { safeUntil = realm.clock() + 40 })
		submit(realm, scheduler, "critical", RENEW, { renewal = true, safeUntil = realm.clock() + 20 })
		realm.budget.update = 1000
		backend:advance(1)
		spec.list(
			order(backend),
			{ "critical", "release-near", "release-late", "receipt", "renew", "load", "ordinary" },
			"LeaseCritical, Release by nearest bound, Receipt, Renew, Load, Ordinary"
		)
		local aged = Backend.new()
		local r2 = aged:realm()
		local s2 = r2.data.scheduler
		r2.budget.update = 0
		submit(r2, s2, "ordinary", ORDINARY, { deadline = 60 })
		aged:advance(6)
		submit(r2, s2, "load", LOAD)
		submit(r2, s2, "renew", RENEW, { renewal = true, safeUntil = r2.clock() + 60 })
		submit(r2, s2, "receipt", RECEIPT)
		r2.budget.update = 1000
		aged:advance(1)
		spec.list(order(aged), { "receipt", "ordinary", "renew", "load" }, "aged Ordinary ahead of Renew and Load only")
		spec.eq(LEASE_CRITICAL, 0, "priority values")
	end,
	["rule 36 retries with jitter and diagnoses a non-retryable error once per store and code"] = function()
		local backend, realm, scheduler = world()
		backend:fail("transient", { key = "flaky" }, 2)
		local flaky = submit(realm, scheduler, "flaky", RENEW)
		backend:advance(3)
		spec.eq(flaky.settled.outcome.kind, "ok", "retried to success")
		spec.eq(dispatched(backend, "flaky"), 3, "two retries")
		local calls = backend:callsTo({ key = "flaky" })
		assert(calls[2].at - calls[1].at <= 0.1 + 0.5 + 1e-6, "first retry within base backoff")
		backend:fail("reject", { key = "bad" }, 2, { code = 403 })
		local first = submit(realm, scheduler, "bad", RENEW)
		backend:advance(1)
		local second = submit(realm, scheduler, "bad", RENEW)
		backend:advance(1)
		spec.eq(first.settled.outcome.code, 403, "settled at once with the error")
		spec.eq(second.settled.outcome.code, 403, "settled at once with the error")
		spec.eq(dispatched(backend, "bad"), 2, "never retried")
		spec.eq(realm.engine.logged("storage error 403"), 1, "diagnosed once per store and code")
	end,
	["rule 36 per-key bytes stay within the per-minute pace"] = function()
		local backend, realm, scheduler = world()
		submit(realm, scheduler, "big", RENEW, { bytes = 2000000, deadline = 120 })
		backend:advance(1)
		submit(realm, scheduler, "big", RENEW, { bytes = 2000000, deadline = 120 })
		backend:advance(30)
		spec.eq(dispatched(backend, "big"), 1, "the second write waits for the window")
		submit(realm, scheduler, "small", RENEW, { bytes = 2000000 })
		backend:advance(1)
		spec.eq(dispatched(backend, "small"), 1, "other keys are paced separately")
		backend:advance(31)
		spec.eq(dispatched(backend, "big"), 2, "dispatched once the minute passed")
	end,
	["rule 54 nothing dispatches at or after the cutoff"] = function()
		local backend, realm, scheduler = world()
		realm.budget.update = 0
		local queued = submit(realm, scheduler, "queued", RELEASE)
		realm.run(function()
			scheduler:setDispatchCutoff(realm.clock() + 1)
		end)
		backend:advance(1.1)
		spec.eq(queued.settled.kind, "timeout", "queued work settles timeout at the cutoff")
		spec.eq(submit(realm, scheduler, "late", RELEASE).settled.kind, "closing", "new work is refused")
		realm.budget.update = 1000
		backend:advance(1)
		spec.eq(#backend.calls, 0, "nothing dispatched")
	end,
}
