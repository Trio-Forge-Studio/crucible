local spec = require("./lib/load")
local Backend = require("./lib/data-backend")

local STORE = "FD.Players"
local USER = 4120
local KEY = tostring(USER)
local NONE = -1

local function world(count: number?)
	local backend = Backend.new()
	local realms = {}
	for _ = 1, count or 1 do
		local realm = backend:realm()
		realm.store = realm.profile(STORE, { coins = 0 })
		table.insert(realms, realm)
	end
	return backend, realms[1], realms[2]
end

local function loaded(backend, realm, userId: number?)
	local player = realm.join(userId or USER)
	local handle = realm.run(function()
		return realm.store:load(player, "join")
	end)
	backend:advance(0.5)
	assert(handle.value == true, "loaded")
	return player
end

local function set(realm, player, coins, stage)
	local result
	realm.run(function()
		result = realm.store:write(player, function(current)
			local value = table.clone(current)
			value.coins = coins
			return value
		end, stage)
	end)
	return result
end

local function save(realm, player, timeout)
	return realm.run(function()
		return realm.store:save(player, timeout)
	end)
end

local function session(realm, player)
	return realm.store.sessions[player]
end

local function stored(backend)
	return backend:envelope(STORE, KEY)
end

local function recover(backend)
	for _, plan in backend.plans do
		plan.remaining = 0
	end
end

local function lastCommitted(backend, realm)
	local last
	for _, call in backend:callsTo({ server = realm.id }) do
		if call.committed then
			last = call
		end
	end
	return last
end

local function attempt(realm, fn)
	local outcome = {}
	realm.run(function()
		outcome.ok, outcome.err = pcall(fn)
	end)
	return outcome
end

return {
	["rule 28 B1 a no-op renewal still extends the lease and records its op"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:advance(23)
		local before = stored(backend)
		spec.eq(session(a, pa).confirmed, 0, "the load's revision is confirmed by the first renewal")
		backend:advance(23)
		local after = stored(backend)
		spec.eq(after.commit.revision, before.commit.revision, "no new revision")
		assert(after.commit.op > before.commit.op, "the renewal committed as its own operation")
		assert(after.lease.expiresAt > before.lease.expiresAt, "a no-op renewal extends the lease")
	end,
	["rule 28 B1 a no-op release still releases"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:advance(23)
		local before = stored(backend)
		a.leave(pa)
		a.run(function()
			a.store:leave(pa)
		end)
		backend:advance(1)
		local after = stored(backend)
		spec.eq(after.lease, nil, "lease removed")
		assert(after.commit.op > before.commit.op, "the release committed")
		spec.eq(session(a, pa), nil, "a departed player's slot is dropped once released")
	end,
	["rule 28 B1 a committed-then-errored renewal confirms what the disk holds"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		set(a, pa, 5)
		backend:fail("committed", { server = a.id }, 1)
		local first = #backend.calls + 1
		save(a, pa, 0)
		backend:advance(3)
		local calls = backend.calls
		spec.eq(calls[first].plan, "committed", "first attempt committed and errored")
		spec.eq(calls[first + 1].committed, nil, "the retry found its commit and wrote nothing")
		spec.eq(session(a, pa).confirmed, 1, "confirmed the committed revision")
		spec.eq(session(a, pa).safeUntil, calls[first].at + a.mono + 70, "authority from the committing attempt's send")
	end,
	["rule 28 K3 an ambiguous commit then a staged receipt then a crash is never acknowledged"] = function()
		local backend, a, b = world(2)
		local pa = loaded(backend, a)
		set(a, pa, 5)
		backend:fail("committed", { server = a.id }, 1)
		save(a, pa, 0)
		backend:advance(0.1)
		local retry = backend:gate("transform", { server = a.id })
		backend:advance(1)
		local sess = session(a, pa)
		set(a, pa, 6, function()
			sess:stage("R")
		end)
		local waiting = save(a, pa, 10)
		local next = backend:gate("transform", { server = a.id })
		retry.release()
		backend:advance(0.5)
		spec.eq(sess.confirmed, 1, "the already confirmed only what it found")
		spec.eq(sess.durable.R, nil, "R is not durable")
		spec.eq(waiting.done, false, "a save of revision 2 keeps waiting")
		assert(#next.parked == 1, "the next operation carries R and has not committed")
		a.crash()
		local envelope = stored(backend)
		spec.eq(envelope.commit.revision, 1, "disk holds revision 1")
		spec.eq(#envelope.receipts, 0, "disk holds no R")
		local pb = b.join(USER)
		local lb = b.run(function()
			return b.store:load(pb, "join")
		end)
		while not lb.done do
			backend:advance(0.05)
		end
		spec.eq(lb.value, true, "rejoined elsewhere")
		local fresh = session(b, pb)
		spec.eq(fresh.durable.R, nil, "R was never acknowledged, so a redelivery grants once")
		spec.eq(fresh.revision, 0, "revision 0 is the loaded record")
		spec.eq(fresh.confirmed, NONE, "nothing confirmed for the new token")
	end,
	["rule 37 save after an already that did not carry its revision waits for the next operation"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		set(a, pa, 5)
		backend:fail("committed", { server = a.id }, 1)
		save(a, pa, 0)
		backend:advance(0.1)
		local retry = backend:gate("transform", { server = a.id })
		backend:advance(1)
		local sess = session(a, pa)
		set(a, pa, 6, function()
			sess:stage("R")
		end)
		local waiting = save(a, pa, 20)
		local next = backend:gate("commit", { server = a.id })
		retry.release()
		backend:advance(0.5)
		spec.eq(session(a, pa).confirmed, 1, "the already confirmed revision 1")
		spec.eq(waiting.done, false, "save of revision 2 not ok after the already")
		next.release()
		backend:advance(0.5)
		spec.eq(waiting.value, "ok", "ok once the next operation committed revision 2")
		spec.eq(stored(backend).commit.revision, 2, "disk holds revision 2")
		spec.eq(sess.durable.R, true, "the staged id is durable once a confirmed envelope holds it")
	end,
	["rule 37 R1 a new session's first save with an update waits for its own commit"] = function()
		for _, update in { true, false } do
			local backend, a, b = world(2)
			local pa = loaded(backend, a)
			for coins = 1, 5 do
				set(a, pa, coins)
			end
			local done = save(a, pa)
			backend:advance(1)
			spec.eq(done.value, "ok", "session A committed revision 5")
			spec.eq(stored(backend).commit.revision, 5, "A's marker holds revision 5")
			a.leave(pa)
			a.run(function()
				a.store:leave(pa)
			end)
			backend:advance(1)
			local pb = loaded(backend, b)
			local sess = session(b, pb)
			spec.eq(sess.confirmed, NONE, "B inherits no confirmation")
			if update then
				set(b, pb, 6)
			end
			local gate = backend:gate("commit", { server = b.id })
			local waiting = save(b, pb)
			backend:advance(2)
			spec.eq(waiting.done, false, `update {update}: save does not return ok before B's write commits`)
			gate.release()
			backend:advance(1)
			spec.eq(waiting.value, "ok", `update {update}: ok once B's own write committed`)
			local commit = stored(backend).commit
			spec.eq(commit.token, sess.token, "the marker is B's")
			spec.eq(commit.revision, if update then 1 else 0, "B's own revision")
			spec.eq(stored(backend).payload.coins, if update then 6 else 5, "B's first write stored the payload")
		end
	end,
	["rule 37 R1 the same after a same-server rejoin"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		set(a, pa, 5)
		save(a, pa)
		backend:advance(1)
		a.leave(pa)
		a.run(function()
			a.store:leave(pa)
		end)
		backend:advance(1)
		local again = loaded(backend, a)
		spec.eq(session(a, again).confirmed, NONE, "nothing confirmed after a same-server rejoin")
		local gate = backend:gate("commit", { server = a.id })
		local waiting = save(a, again)
		backend:advance(2)
		spec.eq(waiting.done, false, "waits for its own commit")
		gate.release()
		backend:advance(1)
		spec.eq(waiting.value, "ok", "ok after its own commit")
		spec.eq(stored(backend).commit.token, session(a, again).token, "marker under the new token")
	end,
	["rule 13 K4 a write from a thread spawned inside a callback throws and the callback's result stands"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		local spawned
		local result = attempt(a, function()
			return a.store:write(pa, function(current)
				a.spawn(function()
					local ok, err = pcall(function()
						a.store:write(pa, function()
							return { coins = 99 }
						end)
					end)
					spawned = { ok = ok, err = err }
					for _, name in { "handoff", "resume" } do
						local okCall, errCall = pcall(function()
							return a.store[name](a.store, pa)
						end)
						spawned[name] = okCall == false
							and string.find(errCall, "inside a data callback", 1, true) ~= nil
					end
				end)
				local value = table.clone(current)
				value.coins = 3
				return value
			end)
		end)
		spec.eq(result.ok, true, "the callback's write succeeded")
		spec.eq(spawned.ok, false, "the spawned write threw")
		spec.has(spawned.err, "FD.Players: 4120's profile is inside a data callback", "busy message")
		spec.eq(spawned.handoff, true, "a handoff of the busy session throws")
		spec.eq(spawned.resume, true, "a resume of the busy session throws")
		spec.eq(session(a, pa).value.coins, 3, "the callback's result stands")
		spec.eq(session(a, pa).revision, 1, "one revision")
	end,
	["rule 13 K4 deferred work after a refused result still runs"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		local ran = false
		local result = attempt(a, function()
			a.store:write(pa, function()
				a.delay(0, function()
					ran = true
				end)
				return "not a record"
			end)
		end)
		spec.eq(result.ok, false, "prepare refused the result at the call")
		spec.eq(session(a, pa).revision, 0, "nothing stored")
		backend:advance(0.1)
		spec.eq(ran, true, "deferred work still ran")
	end,
	["rule 13 K4 a PlayerRemoving inside a callback's spawned thread waits for the callback"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		local order = {}
		attempt(a, function()
			a.store:write(pa, function(current)
				a.spawn(function()
					a.leave(pa)
					a.store:leave(pa)
					table.insert(order, "released")
				end)
				table.insert(order, "callback returned")
				local value = table.clone(current)
				value.coins = 11
				return value
			end)
		end)
		spec.list(order, { "callback returned", "released" }, "the release waited for the callback")
		backend:advance(1)
		local envelope = stored(backend)
		spec.eq(envelope.lease, nil, "released")
		spec.eq(envelope.payload.coins, 11, "the release carried the callback's committed result")
	end,
	["rule 13 a write on a callback's own thread throws, even to another player"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		local other = loaded(backend, a, 77)
		local result = attempt(a, function()
			a.store:write(pa, function(current)
				a.store:write(other, function(v)
					return v
				end)
				return current
			end)
		end)
		spec.eq(result.ok, false, "threw")
		spec.has(result.err, "a write inside a data callback; return the data instead", "own-thread message")
	end,
	["rule 13 a yielding callback is cancelled and thrown at the caller"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		local result = attempt(a, function()
			a.store:write(pa, function()
				coroutine.yield()
				return { coins = 50 }
			end)
		end)
		spec.eq(result.ok, false, "threw at the caller")
		spec.has(result.err, "yielded", "yield reported")
		spec.eq(session(a, pa).value.coins, 0, "nothing stored")
		spec.eq(session(a, pa).busy, false, "the session is idle again")
	end,
	["rule 12 an equal result is accepted with no revision and a pre-load write calls nothing"] = function()
		local backend, a = world()
		local pending = a.join(USER)
		local called = false
		spec.eq(
			a.store:write(pending, function(v)
				called = true
				return v
			end),
			false,
			"no session: false"
		)
		spec.eq(called, false, "change never called")
		local pa = loaded(backend, a)
		local result
		a.run(function()
			result = a.store:write(pa, function(current)
				return current
			end)
		end)
		spec.eq(result, true, "an equal result returns true")
		spec.eq(session(a, pa).revision, 0, "and is no revision")
		spec.eq(set(a, pa, 4), true, "a change returns true")
		spec.eq(session(a, pa).revision, 1, "and is one revision")
	end,
	["rule 14 and rule 110 prepare refuses an oversized payload, naming the store and never the value"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		local secret = "SECRET" .. string.rep("x", 262144)
		local result = attempt(a, function()
			a.store:write(pa, function()
				return { coins = 1, note = secret }
			end)
		end)
		spec.eq(result.ok, false, "refused at the call")
		spec.has(result.err, "FD.Players: 4120's profile is", "names the store and user")
		assert(not string.find(tostring(result.err), "SECRET", 1, true), "never the value")
		spec.eq(session(a, pa).value.coins, 0, "nothing changed")
		spec.eq(session(a, pa).revision, 0, "no revision")
	end,
	["rule 9 writable exactly until safeUntil"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:gate("transform", { server = a.id })
		local bound = session(a, pa).safeUntil
		backend:advance(bound - a.clock() - 0.01)
		spec.eq(a.store:writable(pa), true, "writable just before the bound")
		spec.eq(set(a, pa, 2), true, "writes accepted just before the bound")
		a.suspended = true
		backend:advance(0.02)
		spec.eq(a.store:writable(pa), false, "an authority check past the bound fails before the seal timer runs")
		a.suspended = false
		backend:advance(0.01)
		spec.eq(a.store:writable(pa), false, "not writable at the bound")
		spec.eq(set(a, pa, 3), false, "writes refused at the bound")
	end,
	["rule 10 players lists writable sessions only"] = function()
		local backend, a = world()
		spec.eq(#a.store:players(), 0, "none before a load")
		local pa = loaded(backend, a)
		spec.eq(a.store:players()[1], pa, "the loaded player")
		a.run(function()
			a.store:handoff(pa)
		end)
		spec.eq(#a.store:players(), 0, "a sealed session leaves")
	end,
	["rule 11 await is true once writable and false when the player leaves during the load"] = function()
		local backend, a = world()
		local pa = a.join(USER)
		local waited = a.run(function()
			return a.store:await(pa)
		end)
		a.run(function()
			a.store:load(pa, "join")
		end)
		backend:advance(1)
		spec.eq(waited.value, true, "true once writable")
		local other = a.join(55)
		backend:gate("transform", { key = "55" })
		a.run(function()
			a.store:load(other, "join")
		end)
		local left = a.run(function()
			return a.store:await(other)
		end)
		backend:advance(1)
		spec.eq(left.done, false, "waits while loading")
		a.leave(other)
		a.run(function()
			a.store:leave(other)
		end)
		backend:advance(1)
		spec.eq(left.value, false, "false when the player leaves")
		local idle = a.join(66)
		local never = a.run(function()
			return a.store:await(idle)
		end)
		backend:advance(1)
		a.leave(idle)
		a.run(function()
			a.store:leave(idle)
		end)
		backend:advance(0.1)
		spec.eq(never.value, false, "false when a player with no load leaves")
		backend:seed(STORE, "67", { format = 9 })
		local broken = a.join(67)
		local failing = a.run(function()
			return a.store:await(broken)
		end)
		a.run(function()
			a.store:load(broken, "join")
		end)
		backend:advance(1)
		spec.eq(failing.value, false, "false without waiting further when the load fails")
		a.run(function()
			a.store:handoff(pa)
		end)
		local sealed = a.run(function()
			return a.store:await(pa)
		end)
		spec.eq(sealed.value, false, "false at once for a sealed session")
	end,
	["rule 11 parked waiters continue in park order when woken together"] = function()
		local backend, a = world()
		local waiters = a.graph.module("internal/data/waiters").Waiters.new(a.port)
		local continued = {}
		for index = 1, 16 do
			a.run(function()
				waiters:park()
				table.insert(continued, index)
			end)
		end
		backend:advance(1)
		spec.eq(#continued, 0, "parked until woken")
		a.run(function()
			waiters:wake()
		end)
		spec.list(continued, { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 }, "FIFO by park order")
	end,
	["rule 18 an onLoad change saves at once and a throwing onLoad fails the load with nothing saved"] = function()
		local backend = Backend.new()
		local a = backend:realm()
		local mode = "change"
		a.store = a.data:profile({ name = STORE, version = 0, defaults = { coins = 0 } }, {
			onLoad = function(_, value)
				if mode == "throw" then
					error("claim failed")
				end
				local changed = table.clone(value)
				if mode == "oversized" then
					changed.blob = string.rep("x", 270000)
				end
				changed.coins = 1
				return changed
			end,
			onRelease = function(_, value)
				return value
			end,
			publish = function() end,
			delivered = function() end,
		})
		mode = "oversized"
		local big = a.join(77)
		local refused = a.run(function()
			return a.store:load(big, "join")
		end)
		backend:advance(1)
		spec.eq(refused.value, false, "an onLoad result over the payload cap fails the load")
		spec.eq(backend:envelope(STORE, "77").payload, nil, "nothing of it was saved")
		mode = "change"
		local pa = a.join(USER)
		a.run(function()
			a.store:load(pa, "join")
		end)
		backend:advance(0.4)
		spec.eq(session(a, pa).revision, 1, "the change is one revision")
		spec.eq(stored(backend).payload.coins, 1, "saved at once")
		mode = "throw"
		local other = a.join(88)
		local failed = a.run(function()
			return a.store:load(other, "join")
		end)
		backend:advance(1)
		spec.eq(failed.value, false, "the load failed")
		spec.eq(a.kicked(other), a.limits.LOAD_FAILED_MESSAGE, "kicked")
		local envelope = backend:envelope(STORE, "88")
		spec.eq(envelope.lease, nil, "the acquisition was abandoned")
		spec.eq(envelope.payload, nil, "nothing saved")
		spec.eq(a.engine.logged("onLoad failed"), 2, "each failed load is diagnosed")
	end,
	["rule 16 writes never wait for storage"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:gate("transform", { server = a.id })
		save(a, pa, 0)
		spec.eq(set(a, pa, 9), true, "accepted at once while storage is parked")
		spec.eq(session(a, pa).value.coins, 9, "visible at once")
	end,
	["rule 31 a dirty session saves within one interval and a requested save runs at once"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:advance(23)
		set(a, pa, 4)
		backend:advance(23)
		spec.eq(stored(backend).payload.coins, 4, "saved by the next renewal")
		set(a, pa, 5)
		local before = #backend.calls
		save(a, pa, 0)
		backend:advance(0.2)
		spec.eq(#backend.calls, before + 1, "a requested save dispatches at once")
		spec.eq(stored(backend).payload.coins, 5, "and stores the newest revision")
		local first = #backend.calls + 1
		backend:advance(120)
		local gaps = {}
		for index = first + 1, #backend.calls do
			local gap = backend.calls[index].at - backend.calls[index - 1].at
			table.insert(gaps, gap)
			assert(gap >= 18 and gap <= 22.2, `renewal gap {gap} is not 20 s plus or minus 2 s`)
		end
		assert(#gaps >= 4, "a clean session keeps renewing")
	end,
	["rule 32 a session's saves coalesce into one pending operation"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:advance(1)
		local gate = backend:gate("response", { key = KEY })
		set(a, pa, 1)
		save(a, pa, 0)
		backend:advance(0.2)
		local before = #backend:callsTo({ key = KEY })
		for coins = 2, 4 do
			set(a, pa, coins)
			save(a, pa, 0)
		end
		backend:advance(0.2)
		gate.release()
		backend:advance(1)
		spec.eq(#backend:callsTo({ key = KEY }), before + 1, "three requests behind a flight cost one more call")
		spec.eq(stored(backend).payload.coins, 4, "carrying the newest revision")
	end,
	["rule 44 a renewal for a departed player releases instead"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		a.leave(pa)
		backend:advance(25)
		spec.eq(stored(backend).lease, nil, "the missed PlayerRemoving did not hold the lease")
	end,
	["rule 41 platform data is preserved by every transform"] = function()
		local backend, a = world()
		local seeded = a.envelope.emptyEnvelope("profiles")
		backend:seed(STORE, KEY, seeded, { 7 }, { source = "seed" })
		local pa = loaded(backend, a)
		set(a, pa, 3)
		save(a, pa)
		backend:advance(1)
		a.leave(pa)
		a.run(function()
			a.store:leave(pa)
		end)
		backend:advance(1)
		local entry = backend:stored(STORE, KEY)
		spec.list(entry.userIds, { 7, USER }, "UserIds kept, plus the profile's user")
		spec.eq(entry.metadata.source, "seed", "metadata kept")
	end,
	["rule 49 inspect reads a live session, a stored record, an absent key and an unreadable one"] = function()
		local backend, a, b = world(2)
		local pa = loaded(backend, a)
		set(a, pa, 8)
		local live = a.run(function()
			return a.store:inspect(USER)
		end)
		spec.eq(live.value.value.coins, 8, "a live session's current data")
		save(a, pa)
		backend:advance(1)
		local remote = b.run(function()
			return b.store:inspect(USER)
		end)
		backend:advance(1)
		spec.eq(remote.value.value.coins, 8, "another server reads the stored record")
		spec.eq(stored(backend).lease.serverId, a.id, "inspect never opens anything")
		local absent = b.run(function()
			return b.store:inspect(999)
		end)
		backend:advance(1)
		spec.eq(absent.value.kind, "ok", "absent key ok")
		spec.eq(absent.value.value, nil, "absent key reads undefined")
		backend:seed(STORE, "31", { format = 9 })
		local bad = b.run(function()
			return b.store:inspect(31)
		end)
		backend:advance(1)
		spec.eq(bad.value.kind, "unavailable", "unreadable is unavailable")
	end,
	["rule 49 and rule 35 inspect settles at its call deadline"] = function()
		local backend, a, b = world(2)
		local pa = loaded(backend, a)
		save(a, pa)
		backend:advance(1)
		local gate = backend:gate("response", { kind = "read" })
		local stalled = b.run(function()
			return b.store:inspect(USER)
		end)
		backend:advance(b.limits.CALL_DEADLINE_SECONDS - 1)
		spec.eq(stalled.done, false, "inspect waits for its read")
		backend:advance(1.5)
		spec.eq(stalled.value and stalled.value.kind, "uncertain", "a dispatched read with no answer is uncertain")
		gate.release()
		backend:advance(1)
		b.budget.read = 0
		local queued = b.run(function()
			return b.store:inspect(USER)
		end)
		backend:advance(b.limits.CALL_DEADLINE_SECONDS + 0.5)
		spec.eq(queued.value and queued.value.kind, "timeout", "a read never dispatched is timeout")
	end,
	["rule 43 a throwing or oversized onRelease is skipped, keeping its input, and diagnosed"] = function()
		local backend = Backend.new()
		local a = backend:realm()
		local mode = "throw"
		a.store = a.data:profile({ name = STORE, version = 0, defaults = { coins = 0 } }, {
			onLoad = function(_, value)
				return value
			end,
			onRelease = function(_, value)
				if mode == "throw" then
					error("claim failed")
				end
				local changed = table.clone(value)
				changed.blob = string.rep("x", 270000)
				return changed
			end,
			publish = function() end,
			delivered = function() end,
		})
		for _, case in { { mode = "throw", user = USER }, { mode = "oversized", user = 88 } } do
			mode = case.mode
			local player = loaded(backend, a, case.user)
			set(a, player, 6)
			a.leave(player)
			a.run(function()
				a.store:leave(player)
			end)
			backend:advance(1)
			local envelope = backend:envelope(STORE, tostring(case.user))
			spec.eq(envelope.lease, nil, `{case.mode}: the release still ran`)
			spec.eq(envelope.payload.coins, 6, `{case.mode}: the claim's input was kept`)
			spec.eq(envelope.payload.blob, nil, `{case.mode}: the claim's result was skipped`)
		end
		spec.eq(a.engine.logged("onRelease (leave) was skipped"), 2, "diagnosed once per session")
	end,
	["rule 26 K15 a session sealed at its safe bound still releases and commits with no newer owner"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:fail("throttle", { server = a.id }, 100000)
		set(a, pa, 41)
		local old = session(a, pa)
		local bound = lastCommitted(backend, a).at + a.limits.LOCAL_AUTHORITY_SECONDS
		backend:advance(bound - backend.now + 1)
		spec.eq(old.sealedBy, "authority", "sealed because the safe bound passed")
		spec.eq(a.kicked(pa), a.limits.SESSION_ENDED_MESSAGE, "kicked at the safe bound")
		spec.eq(stored(backend).lease.token, old.token, "the lease is still the sealed session's")
		backend:advance(4)
		recover(backend)
		backend:advance(10)
		local release = lastCommitted(backend, a)
		assert(release.at > bound, `the release was sent {release.at - bound} s after the safe bound`)
		spec.eq(old.released, "committed", "the release committed")
		local envelope = stored(backend)
		spec.eq(envelope.lease, nil, "the lease is released")
		spec.eq(envelope.payload.coins, 41, "with the sealed session's newest revision")
		spec.eq(envelope.commit.token, old.token, "under the sealed session's marker")
	end,
	["rule 26 K15 a session sealed at its safe bound releases against a newer owner and cancels"] = function()
		local backend, a, b = world(2)
		local pa = loaded(backend, a)
		backend:fail("throttle", { server = a.id }, 100000)
		set(a, pa, 41)
		local old = session(a, pa)
		local bound = lastCommitted(backend, a).at + a.limits.LOCAL_AUTHORITY_SECONDS
		-- B's wall clock runs 30 s ahead, so B takes A's lease while A's release may still try (rule 25's step).
		b.wallOffset += 30
		backend:advance(bound - backend.now + 1)
		spec.eq(old.sealedBy, "authority", "sealed because the safe bound passed")
		local pb = b.join(USER)
		local lb = b.run(function()
			return b.store:load(pb, "join")
		end)
		backend:advance(1)
		spec.eq(lb.value, true, "the newer owner acquired")
		local taken = stored(backend)
		recover(backend)
		backend:advance(12)
		local tried = false
		for _, call in backend:callsTo({ server = a.id }) do
			tried = tried or (call.at > bound + 2 and call.outcome ~= nil and call.outcome.kind == "ok")
		end
		assert(tried, "the sealed session's release was attempted after the newer owner acquired")
		spec.eq(old.released, "fenced", "the release cancelled against the newer owner")
		local envelope = stored(backend)
		spec.eq(envelope.lease.token, taken.lease.token, "the newer owner keeps the lease")
		spec.eq(envelope.epoch, taken.epoch, "nothing was written over the newer owner")
		spec.eq(envelope.payload.coins, 0, "the sealed revision was never merged")
	end,
	["rule 26 authority loss is diagnosed once per session"] = function()
		local backend, a = world()
		local pa = loaded(backend, a)
		backend:gate("transform", { server = a.id })
		backend:advance(80)
		set(a, pa, 1)
		set(a, pa, 2)
		spec.eq(a.engine.logged("lost authority"), 1, "one diagnostic")
	end,
}
