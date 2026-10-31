local spec = require("./lib/load")
local Backend = require("./lib/data-backend")

local STORE = "FD.Players"
local USER = 4120
local KEY = tostring(USER)

local function world(count: number?)
	local backend = Backend.new()
	local realms = {}
	for _ = 1, count or 2 do
		local realm = backend:realm()
		realm.store = realm.profile(STORE, { coins = 0, pendingRewards = 0 })
		table.insert(realms, realm)
	end
	return backend, realms[1], realms[2]
end

local function call(realm, method, ...)
	local args = table.pack(...)
	return realm.run(function()
		return realm.store[method](realm.store, table.unpack(args, 1, args.n))
	end)
end

local function loaded(backend, realm)
	local player = realm.join(USER)
	local handle = call(realm, "load", player, "join")
	backend:advance(0.5)
	assert(handle.value == true, "loaded")
	return player
end

local function payout(realm, player, amount)
	local result
	realm.run(function()
		result = realm.store:write(player, function(current)
			local value = table.clone(current)
			value.pendingRewards = amount
			return value
		end)
	end)
	return result
end

local function stored(backend)
	return backend:envelope(STORE, KEY)
end

local function session(realm, player)
	return realm.store.sessions[player]
end

local function clearPlans(backend)
	for _, plan in backend.plans do
		plan.remaining = 0
	end
end

return {
	["rule 45 handoff confirmed seals, releases and leaves the destination an ordinary load"] = function()
		local backend, a, b = world()
		local pa = loaded(backend, a)
		payout(a, pa, 57)
		spec.eq(type(a.store.published[pa]), "table", "published while writable")
		local handed = call(a, "handoff", pa)
		spec.eq(a.store:writable(pa), false, "sealed at once")
		spec.eq(a.store.published[pa], nil, "the slot reads undefined at once")
		backend:advance(1)
		spec.eq(handed.value, "ok", "the release committed")
		spec.eq(payout(a, pa, 99), false, "handle writes refused after the handoff")
		local envelope = stored(backend)
		spec.eq(envelope.lease, nil, "no lease")
		spec.eq(envelope.payload.pendingRewards, 57, "the payout is stored")
		local before = #backend.calls
		a.leave(pa)
		a.run(function()
			a.store:leave(pa)
		end)
		backend:advance(1)
		spec.eq(#backend.calls, before, "rule 46: a PlayerRemoving after a handoff writes nothing more")
		local pb = b.join(USER)
		local lb = call(b, "load", pb, "join")
		backend:advance(0.5)
		spec.eq(lb.value, true, "the destination acquires at once")
		spec.eq(session(b, pb).value.pendingRewards, 57, "and reads the payout")
	end,
	["rule 45 an uncertain handoff keeps releasing in the background"] = function()
		local backend, a, b = world()
		local pa = loaded(backend, a)
		payout(a, pa, 57)
		backend:fail("transient", { server = a.id }, 1000)
		local handed = call(a, "handoff", pa)
		backend:advance(10.5)
		spec.eq(handed.value, "uncertain", "uncertain at the 10 s bound")
		local pb = b.join(USER)
		local lb = call(b, "load", pb, "join")
		backend:advance(5)
		spec.eq(lb.done, false, "rule 46: the destination meets the lease and polls")
		clearPlans(backend)
		backend:advance(15)
		spec.eq(stored(backend).payload.pendingRewards, 57, "the background release landed")
		spec.eq(lb.value, true, "the destination took the released lease")
		spec.eq(session(b, pb).value.pendingRewards, 57, "with the payout")
	end,
	["rule 47 resume after a failed teleport reacquires with a fresh token"] = function()
		local backend, a = world(1)
		local pa = loaded(backend, a)
		payout(a, pa, 57)
		local old = session(a, pa)
		call(a, "handoff", pa)
		backend:advance(1)
		local resumed = call(a, "resume", pa)
		backend:advance(1)
		spec.eq(resumed.value, true, "writable again")
		local current = session(a, pa)
		assert(current ~= old and current.token ~= old.token, "a fresh session and token")
		spec.eq(stored(backend).epoch, 2, "a new acquisition")
		spec.eq(current.value.pendingRewards, 57, "the payout survived the trip")
		spec.list(a.store.reasons, { "join", "resume" }, "onLoad ran with reason resume")
		spec.eq(old.phase, 3, "rule 48: the sealed session never became writable again")
		spec.eq(call(a, "resume", pa).value, true, "resume of a writable session is true at once")
	end,
	["rule 47 resume after the destination took over is refused busy"] = function()
		local backend, a, b = world()
		local pa = loaded(backend, a)
		call(a, "handoff", pa)
		backend:advance(1)
		loaded(backend, b)
		local resumed = call(a, "resume", pa)
		backend:advance(125)
		spec.eq(resumed.value, false, "never steals the destination's lease")
		spec.eq(a.kicked(pa), a.limits.LOAD_BUSY_MESSAGE, "kicked busy")
		spec.eq(stored(backend).lease.serverId, b.id, "destination keeps its lease")
	end,
	["rule 47 K6 resume during a release's backoff waits for it to settle"] = function()
		local backend, a = world(1)
		local pa = loaded(backend, a)
		backend:advance(23)
		payout(a, pa, 57)
		local old = session(a, pa)
		backend:fail("transient", { server = a.id }, 1000)
		call(a, "handoff", pa)
		backend:advance(12)
		local resumed = call(a, "resume", pa)
		backend:advance(13)
		spec.eq(resumed.done, false, "resume waits while the release backs off")
		spec.eq(stored(backend).lease.token, old.token, "the sealed lease is still stored")
		clearPlans(backend)
		local settledAt
		for _ = 1, 200 do
			backend:advance(0.1)
			if stored(backend).lease == nil and settledAt == nil then
				settledAt = backend.now
			end
			if resumed.done then
				break
			end
		end
		spec.eq(resumed.value, true, "resumed after the release settled")
		assert(settledAt ~= nil, "the release committed before the reacquisition")
		spec.eq(session(a, pa).value.pendingRewards, 57, "the payout is kept")
		spec.eq(stored(backend).epoch, 2, "acquired after the release")
	end,
	["rule 29 resume after the release gave up carries the sealed revision"] = function()
		local backend, a = world(1)
		local pa = loaded(backend, a)
		backend:advance(23)
		local old = session(a, pa)
		spec.eq(stored(backend).commit.revision, 0, "disk holds revision 0")
		payout(a, pa, 57)
		backend:fail("transient", { server = a.id }, 1000)
		call(a, "handoff", pa)
		backend:advance(75)
		spec.eq(old.released, "given up", "the release gave up at its bound")
		spec.eq(stored(backend).lease.token, old.token, "the sealed token's lease is still stored")
		clearPlans(backend)
		local resumed = call(a, "resume", pa)
		backend:advance(1)
		spec.eq(resumed.value, true, "resume supersedes this server's own settled token")
		local envelope = stored(backend)
		spec.eq(envelope.payload.pendingRewards, 57, "the sealed snapshot was carried")
		spec.eq(envelope.commit.token, old.token, "carried under the sealed token's marker")
		spec.eq(envelope.commit.revision, 1, "at the sealed revision")
		spec.eq(envelope.epoch, 2, "in the same write that took the lease")
		local fresh = session(a, pa)
		spec.eq(fresh.value.pendingRewards, 57, "the new session holds the payout")
		spec.eq(fresh.confirmed, -1, "R1: nothing confirmed for the new token")
		local gate = backend:gate("commit", { server = a.id })
		local saving = call(a, "save", pa)
		backend:advance(2)
		spec.eq(saving.done, false, "R1: the first save waits for the new token's own commit")
		gate.release()
		backend:advance(1)
		spec.eq(saving.value, "ok", "ok after its own commit")
		spec.eq(stored(backend).commit.token, fresh.token, "marker under the new token")
	end,
	["rule 30 a same-server rejoin waits for the old release to settle"] = function()
		local backend, a = world(1)
		local first = loaded(backend, a)
		payout(a, first, 57)
		local gate = backend:gate("transform", { server = a.id })
		a.leave(first)
		a.run(function()
			a.store:leave(first)
		end)
		local second = a.join(USER)
		local rejoined = call(a, "load", second, "join")
		backend:advance(5)
		spec.eq(rejoined.done, false, "waits while the old release is unsettled")
		spec.eq(#backend:callsTo({ server = a.id }) - 1, 1, "only the parked release was sent")
		gate.release()
		backend:advance(1)
		spec.eq(rejoined.value, true, "acquired after the release settled")
		spec.eq(session(a, second).value.pendingRewards, 57, "with the released data")
		spec.eq(stored(backend).epoch, 2, "with epoch + 1")
	end,
}
