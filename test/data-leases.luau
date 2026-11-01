local spec = require("./lib/load")
local Backend = require("./lib/data-backend")

local STORE = "FD.Players"
local USER = 4120
local KEY = tostring(USER)
local SEALED, ENDED = 2, 3
local LOAD, ORDINARY = 4, 5
local PENDING_PER_KEY = 32

local function world(count: number?, options)
	local backend = Backend.new()
	local realms = {}
	for index = 1, count or 2 do
		local realm = backend:realm(options and options[index])
		realm.store = realm.profile(STORE, { coins = 0 })
		table.insert(realms, realm)
	end
	return backend, realms[1], realms[2]
end

local function load(realm, player)
	return realm.run(function()
		return realm.store:load(player, "join")
	end)
end

local function write(realm, player, coins)
	local result
	realm.run(function()
		result = realm.store:write(player, function(current)
			local value = table.clone(current)
			value.coins = coins
			return value
		end)
	end)
	return result
end

local function save(realm, player)
	return realm.run(function()
		return realm.store:save(player)
	end)
end

local function session(realm, player)
	return realm.store.sessions[player]
end

local function stored(backend)
	return backend:envelope(STORE, KEY)
end

local function fill(realm, seconds: number)
	local scheduler = realm.data.scheduler
	realm.run(function()
		for _ = 1, PENDING_PER_KEY do
			scheduler:submit({
				store = STORE,
				key = KEY,
				priority = ORDINARY,
				kind = "update",
				dispatchDeadline = realm.clock() + seconds,
				bytes = 100,
				transform = function()
					return function()
						return nil
					end
				end,
			})
		end
	end)
end

local function attempts(realm)
	local scheduler = realm.data.scheduler
	local submit = scheduler.submit
	local log = {}
	scheduler.submit = function(self, request)
		local ticket = submit(self, request)
		if request.store == STORE and request.key == KEY and request.priority == LOAD then
			table.insert(log, { at = realm.clock(), refused = ticket.settled and ticket.settled.kind == "overloaded" })
			assert(#log <= 2000, "a refused load resubmits without yielding")
		end
		return ticket
	end
	return log
end

local function snapshot(revision: number, coins: number)
	return { revision = revision, schemaVersion = 0, payload = { coins = coins }, receipts = {} }
end

return {
	["rule 24 one winner under simultaneous acquisition"] = function()
		local backend, a, b = world()
		local pa, pb = a.join(USER), b.join(USER)
		local la, lb = load(a, pa), load(b, pb)
		backend:advance(1)
		spec.eq(la.value, true, "first acquisition wins")
		spec.eq(lb.done, false, "second acquisition polls")
		spec.eq(stored(backend).lease.serverId, a.id, "stored lease is the winner's")
		spec.eq(stored(backend).epoch, 1, "one acquisition committed")
		backend:advance(125)
		spec.eq(lb.value, false, "loser never takes a live lease")
		spec.eq(b.kicked(pb), b.limits.LOAD_BUSY_MESSAGE, "loser kicked busy at the deadline")
		spec.eq(a.store:writable(pa), true, "winner kept renewing")
		spec.eq(stored(backend).lease.serverId, a.id, "lease never stolen")
	end,
	["rule 24 takeover at expiry plus grace and rule 22 expiry"] = function()
		local backend, a, b = world()
		local pa = a.join(USER)
		load(a, pa)
		backend:advance(0.5)
		local expiresAt = stored(backend).lease.expiresAt
		spec.eq(expiresAt, a.wallOffset + 0.05 + 90, "expiry is the transform's wall clock plus 90")
		a.crash()
		local pb = b.join(USER)
		local lb = load(b, pb)
		backend:advance(99)
		spec.eq(lb.done, false, "no takeover before expiry plus grace")
		spec.eq(stored(backend).lease.serverId, a.id, "crashed lease still stored")
		for _ = 1, 400 do
			if lb.done then
				break
			end
			backend:advance(0.05)
		end
		spec.eq(lb.value, true, "takeover after expiry plus grace")
		local lease = stored(backend).lease
		spec.eq(lease.serverId, b.id, "new owner")
		assert(lease.expiresAt - 90 > expiresAt + 10, "taken only past expiresAt + TAKEOVER_GRACE_SECONDS")
		local committing
		for _, call in backend:callsTo({ server = b.id }) do
			committing = committing or (call.committed and call)
		end
		spec.eq(
			session(b, pb).safeUntil,
			committing.at + b.mono + 70,
			"authority dates from the taking attempt, never a held poll"
		)
		spec.eq(stored(backend).epoch, 2, "epoch rose once per acquisition")
	end,
	["rule 21 polls follow the schedule and never wait past expiry plus grace"] = function()
		local backend, a, b = world()
		load(a, a.join(USER))
		backend:advance(0.5)
		local expiresAt = stored(backend).lease.expiresAt
		a.crash()
		local start = backend.now
		load(b, b.join(USER))
		backend:advance(119)
		local times = {}
		for _, call in backend:callsTo({ server = b.id }) do
			table.insert(times, call.at)
			if call.committed then
				break
			end
		end
		local schedule = { 1, 1, 2, 2, 3, 5, 5, 5 }
		for index = 2, #schedule + 1 do
			local gap = times[index] - times[index - 1] - 0.1
			local base = schedule[index - 1]
			assert(
				gap >= base * 0.75 - 1e-6 and gap <= base * 1.25 + 1e-6,
				`poll {index - 1} waited {gap}, base {base}`
			)
		end
		assert(times[1] == start, "first attempt at once")
		for _, at in times do
			assert(at + b.wallOffset <= expiresAt + 10 + 0.35, "a poll never waits past expiry plus grace")
		end
		spec.eq(stored(backend).lease.serverId, b.id, "first poll past expiry plus grace takes the lease")
	end,
	["rule 20 a crash then rejoin loads within LOAD_DEADLINE_SECONDS"] = function()
		local backend, a, b = world()
		load(a, a.join(USER))
		backend:advance(25)
		assert(#backend:callsTo({ server = a.id }) >= 2, "crashed right after a renewal")
		a.crash()
		local start = backend.now
		local pb = b.join(USER)
		local lb = load(b, pb)
		backend:advance(a.limits.LOAD_DEADLINE_SECONDS)
		spec.eq(lb.value, true, "loaded before the deadline")
		spec.eq(b.kicked(pb), nil, "never kicked")
		assert(backend.now - start <= 120, "within LOAD_DEADLINE_SECONDS")
	end,
	["rule 27 a stale origin save is fenced"] = function()
		local backend, a, b = world()
		local pa = a.join(USER)
		load(a, pa)
		backend:advance(1)
		local gate = backend:gate("transform", { server = a.id })
		spec.eq(write(a, pa, 5), true, "origin writes locally")
		backend:advance(75)
		spec.eq(session(a, pa).phase >= SEALED, true, "origin sealed at its bound")
		spec.eq(a.kicked(pa), a.limits.SESSION_ENDED_MESSAGE, "origin kicked")
		local pb = b.join(USER)
		local lb = load(b, pb)
		backend:advance(40)
		spec.eq(lb.value, true, "contender took the expired lease")
		spec.eq(write(b, pb, 9), true, "new owner writes")
		local saved = save(b, pb)
		backend:advance(1)
		spec.eq(saved.value, "ok", "new owner saved")
		gate.release()
		backend:advance(5)
		local envelope = stored(backend)
		spec.eq(envelope.payload.coins, 9, "the origin's stale write never landed")
		spec.eq(envelope.lease.serverId, b.id, "the new owner's lease stands")
		local fenced = false
		for _, call in backend:callsTo({ server = a.id }) do
			fenced = fenced
				or (call.parked == nil and call.outcome and call.outcome.kind == "ok" and not call.committed)
		end
		assert(fenced, "the parked origin write ran and cancelled")
	end,
	["rule 30 same-server ABA is fenced by token and epoch"] = function()
		local backend, a = world(1)
		local first = a.join(USER)
		load(a, first)
		backend:advance(1)
		local old = session(a, first)
		a.leave(first)
		a.run(function()
			a.store:leave(first)
		end)
		backend:advance(1)
		spec.eq(old.phase, ENDED, "old session released")
		local second = a.join(USER)
		load(a, second)
		backend:advance(1)
		local current = session(a, second)
		spec.eq(stored(backend).epoch, 2, "same server acquisition increments the epoch")
		assert(current.token ~= old.token, "fresh token")
		local function replay(token, epoch)
			local decision
			local result = a.lease.ownedWrite(backend:stored(STORE, KEY), {
				userId = USER,
				token = token,
				epoch = epoch,
				op = 99,
				snapshot = snapshot(9, 666),
				release = false,
				wall = a.wall,
			}, function(d)
				decision = d
			end)
			return decision.kind, result
		end
		local kind, result = replay(old.token, old.epoch)
		spec.eq(kind, "fenced", "old token fenced")
		spec.eq(result, nil, "nothing written")
		spec.eq(replay(current.token, old.epoch), "fenced", "old epoch fenced")
		spec.eq(replay(old.token, current.epoch), "fenced", "old token fenced at the current epoch")
		spec.eq(replay(current.token, current.epoch), "written", "current token and epoch write")
	end,
	["rule 108 a conflicting commit reruns the transform and a late response arrives late"] = function()
		local backend, a, b = world()
		local pa, pb = a.join(USER), b.join(USER)
		local gate = backend:gate("commit", { server = a.id })
		local la = load(a, pa)
		backend:advance(0.07)
		local lb = load(b, pb)
		backend:advance(0.2)
		spec.eq(lb.value, true, "the second server committed first")
		gate.release()
		backend:advance(0.2)
		local first = backend:callsTo({ server = a.id })[1]
		spec.eq(first.rerun, true, "the parked transform reran on the newer value")
		spec.eq(first.committed, nil, "and found the lease held")
		spec.eq(la.done, false, "so the first server polls")
		spec.eq(stored(backend).lease.serverId, b.id, "one winner")
		backend:fail("late", { server = b.id }, 1, { seconds = 5 })
		local saving = save(b, pb)
		backend:advance(4)
		spec.eq(saving.done, false, "the committed save's response is late")
		backend:advance(2)
		spec.eq(saving.value, "ok", "confirmed when the late response arrives")
		backend:fail("conflict", { server = b.id }, 1)
		write(b, pb, 4)
		local again = save(b, pb)
		backend:advance(1)
		spec.eq(again.value, "ok", "a forced rerun still commits once")
		spec.eq(stored(backend).payload.coins, 4, "with the newest revision")
	end,
	["rule 17 a slot never has two loads at once"] = function()
		local backend, a = world(1)
		local pa = a.join(USER)
		local first, second = load(a, pa), load(a, pa)
		backend:advance(1)
		spec.eq(first.value, true, "first load")
		spec.eq(second.value, true, "the second call waited for the first")
		spec.eq(#backend:callsTo({ server = a.id }), 1, "one acquisition")
	end,
	["rule 19 acquisition committed then errored finds its own commit"] = function()
		local backend, a = world(1)
		backend:fail("committed", { server = a.id }, 1)
		local pa = a.join(USER)
		local la = load(a, pa)
		backend:advance(3)
		spec.eq(la.value, true, "loaded")
		spec.eq(stored(backend).epoch, 1, "the retry did not acquire again")
		local calls = backend:callsTo({ server = a.id })
		spec.eq(calls[1].plan, "committed", "first attempt committed then errored")
		spec.eq(calls[2].committed, nil, "the retry wrote nothing")
	end,
	["rule 19 an ambiguous acquisition loop dates authority from its earliest attempt"] = function()
		local backend, a = world(1)
		backend:fail("committed", { server = a.id }, 1)
		backend:fail("transient", { server = a.id }, 2)
		local pa = a.join(USER)
		local la = load(a, pa)
		backend:advance(10)
		spec.eq(la.value, true, "loaded")
		local calls = backend:callsTo({ server = a.id })
		assert(#calls >= 4 and calls[4].at > calls[1].at, "later attempts")
		spec.eq(session(a, pa).safeUntil, calls[1].at + a.mono + 70, "authority from the first attempt's send")
	end,
	["rule 19 a held answer after an uncommitted ambiguous attempt resets the authority origin"] = function()
		local backend, a, b = world()
		backend:fail("transient", { server = a.id }, 1)
		local pa, pb = a.join(USER), b.join(USER)
		local la = load(a, pa)
		local lb = load(b, pb)
		backend:advance(1)
		local calls = backend:callsTo({ server = a.id })
		spec.eq(calls[1].plan, "transient", "the first attempt was ambiguous")
		spec.eq(calls[1].committed, nil, "and never committed")
		spec.eq(lb.value, true, "the other server acquired meanwhile")
		spec.eq(stored(backend).lease.serverId, b.id, "its lease is live")
		assert(#calls >= 2 and calls[2].committed == nil and calls[2].outcome.kind == "ok", "the next attempt is held")
		b.crash()
		for _ = 1, 2400 do
			if la.done then
				break
			end
			backend:advance(0.05)
		end
		spec.eq(la.value, true, "acquired once the other lease expired")
		spec.eq(stored(backend).lease.serverId, a.id, "the lease is this server's")
		calls = backend:callsTo({ server = a.id })
		local taking
		for _, call in calls do
			taking = taking or (call.committed and call)
		end
		assert(taking.index > calls[2].index, "taken by a poll after the held answer")
		spec.eq(
			session(a, pa).safeUntil,
			taking.at + a.mono + 70,
			"authority dates from the first attempt after the held answer, never the ambiguous one"
		)
	end,
	["rule 20 a non-retryable load error kicks at once without a retry"] = function()
		local backend, a = world(1)
		backend:fail("reject", { server = a.id }, 1, { code = 403 })
		local pa = a.join(USER)
		local start = backend.now
		local la = load(a, pa)
		backend:advance(0.5)
		spec.eq(la.value, false, "the load ended")
		spec.eq(a.kicked(pa), a.limits.LOAD_FAILED_MESSAGE, "K8: kicked failed")
		assert(a.kicks[1].at - start <= 0.1 + 1e-6, "at once, on the error's response")
		backend:advance(130)
		local calls = backend:callsTo({ server = a.id })
		spec.eq(#calls, 1, "exactly one storage call: no retry and no abandonment")
		spec.eq(calls[1].plan, "reject", "the one call was refused")
		spec.eq(#a.kicks, 1, "kicked once")
		spec.eq(stored(backend), nil, "nothing written")
	end,
	["rule 20 a held load whose polls then throttle until the deadline kicks failed"] = function()
		local backend, a, b = world()
		load(a, a.join(USER))
		backend:advance(1)
		local pb = b.join(USER)
		local start = backend.now
		local lb = load(b, pb)
		backend:advance(100)
		spec.eq(lb.done, false, "polling the live lease")
		local held = 0
		for _, call in backend:callsTo({ server = b.id }) do
			held += if call.outcome and call.outcome.kind == "ok" and not call.committed then 1 else 0
		end
		assert(held >= 5, `{held} held answers before the throttling`)
		backend:fail("throttle", { server = b.id }, 100000)
		backend:advance(30)
		spec.eq(lb.value, false, "the load ended")
		spec.eq(b.kicked(pb), b.limits.LOAD_FAILED_MESSAGE, "storage errors at the deadline kick failed, not busy")
		local late = b.kicks[1].at - start - b.limits.LOAD_DEADLINE_SECONDS
		assert(late >= -1e-6 and late <= 0.25, `kicked {late} s after the deadline`)
		spec.eq(stored(backend).lease.serverId, a.id, "the live lease was never taken")
	end,
	["rule 32 a load refused at a pending cap retries no faster than the poll minimum"] = function()
		local backend, a = world(1)
		local log = attempts(a)
		a.budget.update = 0
		fill(a, 30)
		local pa = a.join(USER)
		local la = load(a, pa)
		backend:advance(29)
		spec.eq(la.done, false, "the load keeps trying while refused")
		assert(#log >= 2, "the refused load tried again")
		for index, attempt in log do
			assert(attempt.refused, `attempt {index} was refused at the per-key cap`)
			if index > 1 then
				local gap = attempt.at - log[index - 1].at
				assert(gap >= 0.25 - 1e-6, `attempt {index} followed the last after {gap} s`)
			end
		end
		assert(#log <= 29 / 0.25 + 1, `{#log} attempts in 29 s`)
		backend:advance(1.5)
		a.budget.update = 1000
		backend:advance(2)
		spec.eq(la.value, true, "loaded once the cap cleared")
		spec.eq(a.kicked(pa), nil, "never kicked")
		spec.eq(#backend:callsTo({ server = a.id }), 1, "the refused attempts and the withdrawn callers sent nothing")
	end,
	["rule 20 a load refused at a pending cap ends at the deadline"] = function()
		local backend, a, b = world()
		load(a, a.join(USER))
		backend:advance(1)
		local log = attempts(b)
		local pb = b.join(USER)
		local start = backend.now
		local lb = load(b, pb)
		backend:advance(100)
		spec.eq(lb.done, false, "polling the live lease")
		b.budget.update = 1
		fill(b, 60)
		backend:advance(25)
		local refused = 0
		for _, attempt in log do
			refused += if attempt.refused then 1 else 0
		end
		assert(refused >= 10, `{refused} attempts refused at the per-key cap`)
		spec.eq(lb.value, false, "the load ended")
		spec.eq(b.kicked(pb), b.limits.LOAD_FAILED_MESSAGE, "refused at the cap, not answered by a live lease")
		local late = b.kicks[1].at - start - b.limits.LOAD_DEADLINE_SECONDS
		assert(math.abs(late) <= 1e-6, `kicked {late} s after the deadline`)
	end,
	["rule 20 abandon after an ambiguous acquisition removes only its lease"] = function()
		local backend, a = world(1)
		backend:fail("committed", { server = a.id }, 1)
		local pa = a.join(USER)
		local la = load(a, pa)
		backend:advance(0.1)
		a.leave(pa)
		a.run(function()
			a.store:leave(pa)
		end)
		spec.eq(la.value, false, "PlayerRemoving ends the load while its retry is in flight")
		backend:advance(5)
		local envelope = stored(backend)
		spec.eq(envelope.lease, nil, "the acquisition's own lease removed")
		spec.eq(envelope.epoch, 1, "nothing acquired again")
		spec.eq(envelope.payload, nil, "nothing but the abandonment written")
		spec.eq(a.kicked(pa), nil, "a departed player is not kicked")
	end,
	["rule 23 a late response past safeUntil grants nothing"] = function()
		local backend, a = world(1)
		local pa = a.join(USER)
		load(a, pa)
		backend:advance(1)
		local bound = session(a, pa).safeUntil
		local gate = backend:gate("response", { server = a.id })
		backend:advance(bound - a.clock() + 1)
		spec.eq(a.store:writable(pa), false, "sealed at the bound")
		gate.release()
		backend:advance(1)
		spec.eq(session(a, pa).safeUntil, bound, "the late confirmation did not extend authority")
		spec.eq(write(a, pa, 3), false, "writes stay refused")
		spec.eq(session(a, pa).phase >= SEALED, true, "no resurrection")
	end,
	["rule 25 skew of 5 s either way keeps the 20 s margin"] = function()
		for _, skew in { 5, -5 } do
			local backend, a, b = world(2, { {}, { wall = 1700000000 + skew } })
			local pa = a.join(USER)
			load(a, pa)
			backend:advance(0.5)
			local expiresAt = stored(backend).lease.expiresAt
			local bound = session(a, pa).safeUntil - a.mono
			a.crash()
			local pb = b.join(USER)
			local lb = load(b, pb)
			backend:advance(119)
			spec.eq(lb.value, true, `skew {skew}: loaded within the deadline`)
			local taken
			for _, call in backend:callsTo({ server = b.id }) do
				if call.committed then
					taken = taken or call.at + 0.05
				end
			end
			assert(taken + b.wallOffset > expiresAt + 10, `skew {skew}: taken past expiry plus grace by its own clock`)
			assert(taken - bound >= 20, `skew {skew}: at least 20 s after the owner's bound`)
		end
	end,
	["rule 23 a suspended writer resuming seals and its writes are fenced"] = function()
		local backend, a, b = world()
		local pa = a.join(USER)
		load(a, pa)
		backend:advance(1)
		a.suspended = true
		local pb = b.join(USER)
		local lb = load(b, pb)
		backend:advance(110)
		spec.eq(lb.value, true, "contender took over")
		write(b, pb, 7)
		save(b, pb)
		backend:advance(1)
		a.suspended = false
		backend:advance(0.01)
		spec.eq(write(a, pa, 99), false, "first authority check after resuming fails")
		spec.eq(a.kicked(pa), a.limits.SESSION_ENDED_MESSAGE, "kicked")
		backend:advance(30)
		spec.eq(stored(backend).payload.coins, 7, "nothing of the suspended writer merged")
		spec.eq(stored(backend).lease.serverId, b.id, "lease kept by the new owner")
	end,
	["rule 25 a common wall-clock step fences the old owner and nothing merges"] = function()
		local backend, a, b = world()
		local pa = a.join(USER)
		load(a, pa)
		backend:advance(30)
		a.wallOffset += 200
		b.wallOffset += 200
		local pb = b.join(USER)
		local lb = load(b, pb)
		backend:advance(0.5)
		spec.eq(lb.value, true, "contender takes over at once after the common step")
		spec.eq(write(a, pa, 41), true, "the old owner still accepts writes inside its monotonic bound")
		write(b, pb, 8)
		backend:advance(40)
		spec.eq(a.store:writable(pa), false, "the old owner's next owned write was fenced")
		spec.eq(a.kicked(pa), a.limits.SESSION_ENDED_MESSAGE, "old owner kicked")
		save(b, pb)
		backend:advance(1)
		spec.eq(stored(backend).payload.coins, 8, "the old owner's write was lost, never merged")
		spec.eq(stored(backend).lease.serverId, b.id, "new owner keeps the lease")
	end,
	["rule 26 an owned write that finds its own lease expired seals"] = function()
		local backend, a = world(1)
		local pa = a.join(USER)
		load(a, pa)
		backend:advance(1)
		a.wallOffset += 100
		backend:advance(25)
		spec.eq(a.store:writable(pa), false, "sealed")
		spec.eq(a.kicked(pa), a.limits.SESSION_ENDED_MESSAGE, "kicked")
		spec.eq(stored(backend).lease.serverId, a.id, "an expired lease is never renewed")
	end,
	["rule 40 an unreadable record fails closed at once"] = function()
		local backend, a = world(1)
		local bad = { format = 2, kind = "profiles", schemaVersion = 0, epoch = 4, payload = { coins = 3 } }
		backend:seed(STORE, KEY, bad)
		local pa = a.join(USER)
		local la = load(a, pa)
		backend:advance(1)
		spec.eq(la.value, false, "not loaded")
		spec.eq(a.kicked(pa), a.limits.LOAD_FAILED_MESSAGE, "kicked at once")
		spec.eq(stored(backend).format, 2, "never repaired")
		spec.eq(stored(backend).epoch, 4, "never overwritten")
		spec.eq(#backend:callsTo({ server = a.id }), 1, "no retry")
		spec.eq(a.engine.logged("unreadable"), 1, "diagnosed")
	end,
	["rule 77 a newer stored version is never acquired"] = function()
		local backend, a = world(1)
		local newer = a.envelope.emptyEnvelope("profiles")
		newer.schemaVersion = 3
		newer.payload = { coins = 12 }
		newer.epoch = 6
		backend:seed(STORE, KEY, newer)
		local pa = a.join(USER)
		local la = load(a, pa)
		backend:advance(1)
		spec.eq(la.value, false, "not loaded")
		spec.eq(a.kicked(pa), a.limits.SERVER_UPDATING_MESSAGE, "kicked at once")
		spec.eq(stored(backend).epoch, 6, "epoch untouched")
		spec.eq(stored(backend).lease, nil, "no lease taken")
	end,
}
