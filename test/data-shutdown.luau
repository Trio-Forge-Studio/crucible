local spec = require("./lib/load")
local Backend = require("./lib/data-backend")

local STORE = "FD.Players"
local USER = 4120
local KEY = tostring(USER)
local ORDINARY = 5

-- A realm whose boot holds DataRuntime's shutdown and one game Subsystem, `game(realm, self)`, in parallel.
local function world(game)
	local backend = Backend.new()
	local holder = {}
	local realm = backend:realm({
		subsystems = {
			{
				id = "DataRuntime",
				methods = {
					onShutdown = function(self)
						holder.realm.events.dataStart = holder.realm.clock()
						holder.realm.shutdown.shutdownData(holder.realm.data, self)
						holder.realm.events.dataEnd = holder.realm.clock()
					end,
				},
			},
			{
				id = "Game",
				methods = {
					onShutdown = function(self)
						if game then
							game(holder.realm, self)
						end
						holder.realm.events.gameEnd = holder.realm.clock()
					end,
				},
			},
		},
	})
	holder.realm = realm
	realm.events = {}
	realm.store = realm.profile(STORE, { coins = 0 })
	return backend, realm
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

local function set(realm, player, coins)
	return realm.store:write(player, function(current)
		local value = table.clone(current)
		value.coins = coins
		return value
	end)
end

local function sleep(realm, seconds)
	local wake = realm.clock() + seconds
	while realm.clock() < wake do
		realm.wait(0)
	end
end

local function stored(backend, key: string?)
	return backend:envelope(STORE, key or KEY)
end

return {
	["rule 111 E-D1 bootClock is the boot port's clock and shutdownProgress reads the sweep"] = function()
		local seen = {}
		local backend, realm = world(function(r, self)
			seen.atStart = r.boot.shutdownProgress(self)
			sleep(r, 3)
		end)
		spec.eq(realm.boot.bootClock(), realm.clock(), "bootClock is the realm's scripted clock")
		spec.eq(#realm.closers, 1, "rule 56: data binds no BindToClose of its own")
		backend:advance(10)
		local start = realm.clock()
		realm.close()
		backend:advance(0.01)
		local progress = seen.atStart
		spec.eq(progress.startedAt, start, "startedAt on the boot clock")
		spec.eq(progress.deadline, start + 28, "deadline is start + SHUTDOWN_TIMEOUT_SECONDS")
		spec.eq(progress.forceStartAt, start + 25, "forceStartAt is start + SHUTDOWN_DEADLINE_SECONDS")
		spec.eq(progress.pending, 1, "the other hook is pending, the caller's own excluded")
		backend:advance(40)
		spec.eq(realm.events.dataEnd < start + 4, true, "data's final phase followed the game hook's finish")
	end,
	["rule 111 shutdownProgress is undefined before the sweep and counts hooks not yet started"] = function()
		local backend = Backend.new()
		local holder = {}
		local realm = backend:realm({
			subsystems = {
				{
					id = "Store",
					methods = {
						onShutdown = function(self)
							holder.progress = holder.realm.boot.shutdownProgress(self)
						end,
					},
				},
				{
					id = "Saver",
					deps = { "Store" },
					methods = {
						onShutdown = function()
							holder.realm.wait(2)
						end,
					},
				},
			},
		})
		holder.realm = realm
		backend:advance(1)
		spec.eq(realm.boot.shutdownProgress({}), nil, "undefined before the sweep")
		realm.close()
		backend:advance(0.01)
		spec.eq(holder.progress, nil, "the dependency waits for its dependent")
		backend:advance(3)
		spec.eq(holder.progress.pending, 0, "the dependent finished before the dependency started")
		spec.eq(holder.progress.notStarted, 0, "nothing left unstarted")
	end,
	["rule 111 a second ignite in one realm sees no stale sweep"] = function()
		local backend, realm = world()
		local probe = realm.graph.module("subsystem").Subsystem
		realm.close()
		backend:advance(30)
		assert(realm.boot.shutdownProgress(probe) ~= nil, "the sweep is visible after it ran")
		local ok = pcall(
			realm.boot.ignite,
			realm.engine.port,
			{ name = "server", renders = false, booted = function() end },
			{}
		)
		spec.eq(ok, false, "a second boot is refused by the registry")
		spec.eq(realm.boot.shutdownProgress(probe), nil, "but its ignite reset the sweep first")
	end,
	["rule 111 a second ignite takes its own port's clock"] = function()
		local _, realm = world()
		assert(realm.boot.bootClock() ~= 12345, "the first clock is the realm's")
		local port = table.clone(realm.engine.port)
		port.clock = function()
			return 12345
		end
		pcall(realm.boot.ignite, port, { name = "server", renders = false, booted = function() end }, {})
		spec.eq(realm.boot.bootClock(), 12345, "bootClock follows the port of the latest ignite")
	end,
	["rule 52 early hook: the early flush keeps the lease and the final phase follows the last hook"] = function()
		local accepted
		local backend, realm = world(function(r)
			sleep(r, 1)
			accepted = set(r, r.player, 7)
			sleep(r, 0.5)
		end)
		realm.player = loaded(backend, realm)
		backend:advance(23)
		set(realm, realm.player, 3)
		realm.close()
		backend:advance(0.5)
		local flushed = stored(backend)
		spec.eq(flushed.payload.coins, 3, "dirty session flushed at hook start")
		assert(flushed.lease ~= nil, "the early flush keeps the lease")
		backend:advance(3)
		spec.eq(accepted, true, "a game hook's profile write is accepted in the open phase")
		local final = stored(backend)
		spec.eq(final.lease, nil, "released in the final phase")
		spec.eq(final.payload.coins, 7, "the open-phase write was saved")
		assert(realm.events.dataEnd - realm.events.dataStart < 3, "the hook returned once released")
	end,
	["rule 26 K15 an early flush in flight at the safe bound is not sent after it, only the release"] = function()
		local backend, realm = world(function(r)
			while true do
				r.wait(1)
			end
		end)
		local player = loaded(backend, realm)
		backend:advance(0.2)
		local issued
		for _, call in backend:callsTo({ server = realm.id }) do
			if call.committed then
				issued = call.at
			end
		end
		realm.port.random = function()
			return 0
		end
		backend:fail("throttle", { server = realm.id }, 100000)
		local bound = issued + realm.limits.LOCAL_AUTHORITY_SECONDS
		local old = realm.store.sessions[player]
		backend:advance(issued + 50 - backend.now)
		set(realm, player, 3)
		realm.close()
		backend:advance(issued + 55 - backend.now)
		assert(old.writing ~= nil and old.want ~= nil, "a renewal is retrying, the flush waits behind it")
		for _, plan in backend.plans do
			plan.remaining = 0
		end
		backend:fail("reject", { server = realm.id }, 1)
		backend:fail("throttle", { server = realm.id }, 100000)
		backend:advance(1)
		backend:advance(issued + 69 - backend.now)
		assert(old.writing ~= nil, "the flush is in flight")
		assert(old.sealedBy == "shutdown", "sealed by the final phase")
		backend:advance(bound + 2 - backend.now)
		for _, plan in backend.plans do
			plan.remaining = 0
		end
		backend:advance(3)
		local count = 0
		for _, call in backend:callsTo({ server = realm.id }) do
			if call.committed and call.at > bound then
				count += 1
			end
		end
		spec.eq(count, 1, "one commit after the safe bound")
		spec.eq(old.released, "committed", "and it is the release")
		spec.eq(stored(backend).lease, nil, "the lease is released")
		spec.eq(stored(backend).payload.coins, 3, "with the sealed revision")
	end,
	["rule 53 stalled hook: the final phase starts at deadline minus 10"] = function()
		local backend, realm = world(function(r)
			while true do
				r.wait(1)
			end
		end)
		local player = loaded(backend, realm)
		realm.close()
		local start = realm.clock()
		backend:advance(17.5)
		spec.eq(realm.store:writable(player), true, "still writable before 18")
		backend:advance(1)
		spec.eq(realm.store:writable(player), false, "sealed at 18")
		backend:advance(1)
		spec.eq(stored(backend).lease, nil, "released")
		assert(realm.events.dataEnd - start < 26.5, "the hook returned before the cutoff")
		backend:advance(10)
		spec.eq(realm.events.gameEnd, nil, "the stalled hook never finished")
	end,
	["rule 53 a forced-late hook's profile write is refused and diagnosed once"] = function()
		local results = {}
		local backend, realm = world(function(r)
			sleep(r, 19)
			table.insert(results, set(r, r.player, 5))
			table.insert(results, set(r, r.player, 6))
		end)
		realm.player = loaded(backend, realm)
		realm.close()
		backend:advance(25)
		spec.eq(#results, 2, "two late writes")
		spec.eq(results[1], false, "a late write returns false")
		spec.eq(results[2], false, "a second late write returns false")
		spec.eq(realm.engine.logged("refused a write after the shutdown final phase"), 1, "diagnosed once per store")
		spec.eq(stored(backend).payload.coins, 0, "nothing late was stored")
	end,
	["rule 52 a CommandLog-style flush dispatches in the open phase and generic work closes at the cutoff"] = function()
		local early, late
		local backend, realm = world(function(r)
			sleep(r, 1)
			early = r.data.scheduler:submit({
				store = "FD.CommandLog",
				key = "log",
				priority = ORDINARY,
				kind = "update",
				dispatchDeadline = r.clock() + 30,
				bytes = 100,
				transform = function()
					return function()
						return { value = { entries = 1 }, userIds = {}, metadata = {} }
					end
				end,
			})
			early:wait()
			sleep(r, 26)
			late = r.data.scheduler:submit({
				store = "FD.CommandLog",
				key = "log",
				priority = ORDINARY,
				kind = "update",
				dispatchDeadline = r.clock() + 30,
				bytes = 100,
			})
		end)
		loaded(backend, realm)
		realm.close()
		backend:advance(30)
		spec.eq(early.settled.kind, "done", "accepted in the open phase")
		spec.eq(backend:envelope("FD.CommandLog", "log").entries, 1, "committed")
		spec.eq(late.settled.kind, "closing", "refused after the cutoff")
	end,
	["rule 54 waiters settle honestly at settleAt and nothing dispatches after the cutoff"] = function()
		local saved
		local backend, realm = world(function(r)
			while true do
				r.wait(1)
			end
		end)
		local player = loaded(backend, realm)
		backend:advance(23)
		set(realm, player, 4)
		local gate = backend:gate("response", { server = realm.id })
		saved = realm.run(function()
			return realm.store:save(player, 30)
		end)
		realm.close()
		local start = realm.clock()
		backend:advance(27.4)
		spec.eq(saved.done, false, "still waiting before settleAt")
		backend:advance(0.2)
		spec.eq(saved.value, "uncertain", "an unresolved attempt settles uncertain at settleAt")
		local returned = realm.events.dataEnd - start
		assert(returned >= 27.5 and returned <= 27.5 + Backend.FRAME + 1e-6, "the hook returned at settleAt")
		local after = 0
		for _, call in backend:callsTo({ server = realm.id }) do
			if call.at + realm.mono >= start + 26.5 then
				after += 1
			end
		end
		spec.eq(after, 0, "nothing dispatched at or after the cutoff")
		gate.release()
	end,
	["rule 51 loads close at hook start and loading sessions are abandoned"] = function()
		local backend, realm = world()
		backend:gate("response", { key = "77" })
		local pending = realm.join(77)
		local loading = realm.run(function()
			return realm.store:load(pending, "join")
		end)
		backend:advance(0.2)
		realm.close()
		backend:advance(5)
		spec.eq(loading.value, false, "the load was abandoned")
		spec.eq(realm.store:writable(pending), false, "never writable")
		local late = realm.join(78)
		local lateLoad = realm.run(function()
			return realm.store:load(late, "join")
		end)
		backend:advance(1)
		spec.eq(lateLoad.value, false, "a load after hook start closes at once")
		spec.eq(#backend:callsTo({ key = "78" }), 0, "and sends nothing")
	end,
	["rule 50 with no visible sweep the window is SHUTDOWN_FALLBACK_SECONDS"] = function()
		local backend, realm = world()
		local player = loaded(backend, realm)
		local start = realm.clock()
		local finished = realm.run(function()
			realm.shutdown.shutdownData(realm.data)
			return realm.clock()
		end)
		backend:advance(3)
		spec.eq(realm.store:writable(player), false, "final phase at once")
		assert(finished.value - start <= 1.5, "returned by the fallback settle time")
		spec.eq(stored(backend).lease, nil, "released")
	end,
	["rule 107 retirement seals, releases and kicks once the release settles"] = function()
		local backend, realm = world()
		local player = loaded(backend, realm)
		set(realm, player, 9)
		local gate = backend:gate("response", { server = realm.id })
		realm.run(function()
			realm.data:retire()
		end)
		spec.eq(realm.store:writable(player), false, "sealed at once")
		backend:advance(1)
		spec.eq(realm.kicked(player), nil, "not kicked before the release settles")
		gate.release()
		backend:advance(1)
		spec.eq(realm.kicked(player), realm.limits.SESSION_ENDED_MESSAGE, "kicked once released")
		spec.eq(stored(backend).payload.coins, 9, "the best-effort release saved")
		spec.eq(stored(backend).lease, nil, "lease released")
	end,
}
