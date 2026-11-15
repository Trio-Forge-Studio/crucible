local harness = require("../packages/crucible/lune")

return {
	["Lune harness isolates compiled graphs and overrides services"] = function()
		local service = { marker = true }
		local a = harness.loadGame({ place = "build/sample-game.rbxl", services = { Workspace = service } })
		local b = harness.loadGame({ place = "build/sample-game.rbxl" })
		local module = a.place:GetService("ReplicatedStorage").TS.profile
		local first = a.import(module).PlayerData
		assert(first == a.import(module).PlayerData, "one module instance per graph")
		assert(
			first ~= b.import(b.place:GetService("ReplicatedStorage").TS.profile).PlayerData,
			"isolated declarations"
		)
		local probe = require("@lune/roblox").Instance.new("ModuleScript")
		probe.Source = "return game:GetService('Workspace')"
		assert(a.load(probe) == service, "service double reaches compiled environment")
		probe:Destroy()
		local testing = a.import(a.crucible.testing)
		local Controller = a.import(a.place:GetService("ServerScriptService").TS.systems.controller).Controller
		local controller = testing.subsystem(Controller, { ready = true })
		controller:awake()
		assert(a.messages[#a.messages] == "S2 controller awake", "compiled class uses same testing registry")
		a.place:Destroy()
		b.place:Destroy()
	end,
	["Lune harness profile fixture uses compiled defaults migrations and persistence"] = function()
		local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
		local token = graph.import(graph.place:GetService("ReplicatedStorage").TS.profile).PlayerData
		local backend = harness.Backend.new()
		local realm = backend:realm()
		backend:seed("Sample.Players", "4120", {
			format = 1,
			kind = "profiles",
			schemaVersion = 0,
			payload = { gold = 9 },
			epoch = 0,
			receipts = {},
			inbox = {},
			quarantine = {},
			seen = {},
			seenFloor = 0,
			operations = {},
			operationFloor = 0,
		})
		local hookCalls = 0
		local fixture = harness.profileFixture(realm, token, {
			onLoad = function(_, value)
				hookCalls += 1
				return value
			end,
		})
		local player = realm.join(4120)
		local loading = realm.run(function()
			return fixture:load(player, "join")
		end)
		backend:advance(1)
		assert(loading.done and loading.value == true, "compiled schema admitted")
		assert(fixture.published[player].coins == 9, "compiled migration from gold")
		assert(hookCalls == 1, "fixture hook")
		assert(fixture.published[player].pickups == 0, "serverOnly default unwrapped")
		realm.run(function()
			fixture:write(player, function(value)
				local next = table.clone(value)
				next.coins = 7
				return next
			end)
		end)
		local saving = realm.run(function()
			return fixture:save(player, 20)
		end)
		backend:advance(21)
		assert(saving.done, "save completed on fake clock")
		assert(backend:envelope("Sample.Players", "4120").payload.coins == 7, "real persistence path")
		graph.place:Destroy()
	end,
}
