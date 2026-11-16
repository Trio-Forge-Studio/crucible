-- The built sample game's LegacyImport on the shipped harness: its compiled Subsystem, store and migrations run on
-- the runtime the place was built with, over the harness Backend's DataStore, with a legacy DataStoreService double.
local harness = require("../packages/crucible/lune")

local PLACE = "build/sample-game.rbxl"
local STORE = "Sample.Players"
local USER = 4120
local KEY = `player_{USER}`

-- `Sample.Legacy` holding `records` by key. A held read parks until `release`, as a slow GetAsync yields.
local function legacyService(records)
	local service = { reads = 0, held = false, waiting = {} }
	function service.release()
		service.held = false
		local waiting = service.waiting
		service.waiting = {}
		for _, thread in waiting do
			local ok, err = coroutine.resume(thread)
			assert(ok, tostring(err))
		end
	end
	function service.GetDataStore(_, name: string)
		assert(name == "Sample.Legacy", `unexpected DataStore {name}`)
		return {
			GetAsync = function(_, key: string)
				service.reads += 1
				if service.held then
					table.insert(service.waiting, coroutine.running())
					coroutine.yield()
				end
				return records[key], nil
			end,
		}
	end
	return service
end

-- One server: the built place on `realm`'s clock and DataStore, booted with only LegacyImport.
local function server(realm, legacy)
	local engine = harness.engine.new()
	engine.port.clock = realm.clock
	engine.port.wait = realm.wait
	engine.port.spawn = function(callback)
		realm.spawn(callback)
	end
	local graph = harness.loadGame({
		place = PLACE,
		engine = engine,
		dataPort = realm.port,
		services = { DataStoreService = legacy },
	})
	graph.import(graph.place:GetService("ServerScriptService").TS.legacy)
	graph.import(graph.crucible.server).boot({ load = {} })
	graph.PlayerData = graph.import(graph.place:GetService("ReplicatedStorage").TS.profile).PlayerData
	return graph
end

local function coins(graph, player)
	return graph.PlayerData(player).coins()
end

local function envelope(backend)
	return backend:envelope(STORE, tostring(USER))
end

return {
	["sample legacy import: a legacy save is imported once, through the sample's migration"] = function()
		local backend = harness.Backend.new()
		local realm = backend:realm()
		local legacy = legacyService({ [KEY] = { save = { gold = 40 }, owner = "old-server", heartbeat = 0 } })
		local graph = server(realm, legacy)
		backend:advance(1)
		local player = realm.join(USER)
		backend:advance(2)
		assert(graph.PlayerData:loaded(player), "writable after the import")
		assert(coins(graph, player) == 40, `fromGold turned 40 gold into coins, got {coins(graph, player)}`)
		local stored = envelope(backend)
		assert(stored.imported == true, "the store holds the import's mark")
		assert(stored.payload.coins == 40 and stored.schemaVersion == 1, "with the migrated record, in the same write")
		realm.leave(player)
		backend:advance(3)
		assert(envelope(backend).lease == nil, "released")
		local again = realm.join(USER)
		backend:advance(2)
		assert(legacy.reads == 1, `the legacy save is read once, read {legacy.reads} times`)
		assert(coins(graph, again) == 40, "the rejoin loads the saved profile")
		graph.place:Destroy()
	end,

	["sample legacy import: a server that crashes mid-import leaves nothing, and the next server imports again"] = function()
		local backend = harness.Backend.new()
		local first = backend:realm()
		local legacy = legacyService({ [KEY] = { save = { gold = 7 } } })
		legacy.held = true
		local a = server(first, legacy)
		backend:advance(1)
		first.join(USER)
		backend:advance(2)
		assert(#legacy.waiting == 1, "the import is reading")
		first.crash()
		backend:advance(1)
		local stored = envelope(backend)
		assert(stored.payload == nil and stored.imported == nil, "nothing imported was written")
		assert(stored.lease ~= nil, "the crashed server's lease is left to expire")
		legacy.held = false
		local second = backend:realm()
		local b = server(second, legacy)
		backend:advance(1)
		local player = second.join(USER)
		backend:advance(110)
		assert(b.PlayerData:loaded(player), "the next server takes the profile once the lease expires")
		assert(legacy.reads == 2, `and imports again, read {legacy.reads} times`)
		assert(coins(b, player) == 7, "the import loads")
		assert(envelope(backend).imported == true, "and is marked")
		a.place:Destroy()
		b.place:Destroy()
	end,

	["sample legacy import: a finished import with the legacy save still present does not run again"] = function()
		local backend = harness.Backend.new()
		local realm = backend:realm()
		local legacy = legacyService({ [KEY] = { save = { gold = 40 } } })
		backend:seed(STORE, tostring(USER), {
			format = 1,
			kind = "profiles",
			schemaVersion = 1,
			payload = { coins = 3 },
			imported = true,
			epoch = 2,
			receipts = {},
			inbox = {},
			quarantine = {},
			seen = {},
			seenFloor = 0,
			operations = {},
			operationFloor = 0,
		})
		local graph = server(realm, legacy)
		backend:advance(1)
		local player = realm.join(USER)
		backend:advance(2)
		assert(graph.PlayerData:loaded(player), "writable")
		assert(legacy.reads == 0, "the legacy save was never read")
		assert(coins(graph, player) == 3, "the profile is what Crucible saved")
		graph.place:Destroy()
	end,

	["sample legacy import: authority lost mid-import writes nothing the import read"] = function()
		local backend = harness.Backend.new()
		local realm = backend:realm()
		local legacy = legacyService({ [KEY] = { save = { gold = 90 } } })
		legacy.held = true
		local graph = server(realm, legacy)
		backend:advance(1)
		local player = realm.join(USER)
		backend:advance(75)
		assert(realm.kicked(player) == "Your data session ended. Please rejoin.", "past the safe bound, the load ends")
		legacy.release()
		backend:advance(5)
		assert(not graph.PlayerData:loaded(player), "the late import never made the profile writable")
		local stored = envelope(backend)
		assert(stored.payload == nil, "nothing the import read was written")
		assert(stored.imported == nil, "and no mark")
		assert(stored.lease == nil, "the load removed only its own lease")
		graph.place:Destroy()
	end,
}
