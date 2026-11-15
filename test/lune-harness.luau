local harness = require("../packages/crucible/lune")

return {
	["Lune destroyed behaviours are collectable while terminal phases remain readable"] = function()
		local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
		local Member = graph.import(graph.place:GetService("ServerScriptService").TS.systems.member).Member
		local weak = setmetatable({}, { __mode = "v" })
		local created = 0
		Member.awake = function(self)
			self.payload = table.create(10000, 42)
			created += 1
			if created <= 200 then
				weak[created] = self
			end
		end
		Member.start = function() end
		Member.onEnable = function() end
		Member.onDisable = function() end
		Member.onDestroy = function() end
		graph.import(graph.crucible.server).boot({ load = {} })
		local instances = {}
		local function churn()
			for i = 1, 200 do
				local instance = graph.engine.instance("Member", "Part", { Identity = "test" })
				instances[i] = instance
				graph.engine.addTag(instance, "S2Member")
				graph.engine.flush()
				assert(graph.lifecycle(Member, instance) == "enabled", "created behaviour is live")
				graph.engine.removeTag(instance, "S2Member")
				graph.engine.flush()
			end
		end
		churn()
		local live = graph.engine.instance("Live", "Part", { Identity = "live" })
		graph.engine.addTag(live, "S2Member")
		graph.engine.flush()
		assert(graph.lifecycle(Member, live) == "enabled", "next construction replaces the runner cached result")
		assert(created == 201, "probe observed every behaviour object")
		require("@lune/task").wait(0.1)
		for _ = 1, 400 do
			local garbage = {}
			for i = 1, 4000 do
				garbage[i] = { i }
			end
			if next(weak) == nil then
				break
			end
		end
		assert(next(weak) == nil, "destroyed behaviour objects are collectable")
		for _, instance in instances do
			assert(graph.lifecycle(Member, instance) == "destroyed", "history survives object collection")
		end
		graph.place:Destroy()
	end,
	["Lune game modules cannot impersonate runtime lifecycle"] = function()
		local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
		local roblox = require("@lune/roblox")
		local folder = roblox.Instance.new("Folder")
		folder.Name = "internal"
		local module = roblox.Instance.new("ModuleScript")
		module.Name = "lifecycle"
		module.Source = "return 42"
		module.Parent = folder
		assert(graph.load(module) == 42, "game lifecycle module evaluates normally")
		folder:Destroy()
		graph.place:Destroy()
	end,
	["Lune behaviour lifecycle retains terminal states and follows rebinding"] = function()
		for _, fails in { false, true } do
			local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
			local Member = graph.import(graph.place:GetService("ServerScriptService").TS.systems.member).Member
			local instance = graph.engine.instance("Member", "Part", { Identity = "test" })
			graph.engine.addTag(instance, "S2Member")
			assert(graph.lifecycle(Member, instance) == "unconstructed", "not yet admitted")
			if fails then
				Member.awake = function()
					error("behaviour awake failure")
				end
			end
			local ok, message = pcall(graph.import(graph.crucible.server).boot, { load = {} })
			if not fails then
				assert(ok, tostring(message))
			end
			if fails then
				assert(graph.lifecycle(Member, instance) == "faulted", "fault survives unbinding")
			else
				assert(graph.lifecycle(Member, instance) == "enabled", "behaviour booted")
				graph.engine.removeTag(instance, "S2Member")
				graph.engine.flush()
				assert(graph.lifecycle(Member, instance) == "destroyed", "destroy survives unbinding")
				graph.engine.addTag(instance, "S2Member")
				graph.engine.flush()
				assert(graph.lifecycle(Member, instance) == "enabled", "latest generation replaces terminal state")
			end
			graph.place:Destroy()
		end
	end,
	["Lune replacements cannot change after a consumer reads false"] = function()
		local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
		local roblox = require("@lune/roblox")
		local folder = roblox.Instance.new("Folder")
		local replacement = roblox.Instance.new("ModuleScript")
		replacement.Name = "replacement"
		replacement.Parent = folder
		local consumer = roblox.Instance.new("ModuleScript")
		consumer.Parent = folder
		consumer.Source = "return { value = require('./replacement') }"
		graph.provideGame(replacement, false)
		assert(graph.load(consumer).value == false, "consumer captures false")
		assert(not pcall(graph.provideGame, replacement, 42), "consumed replacement rejected")
		assert(graph.load(replacement) == false, "later loads retain false")
		folder:Destroy()
		graph.place:Destroy()
	end,
	["Lune lifecycle reports a fault from a compiled hook"] = function()
		local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
		local systems = graph.place:GetService("ServerScriptService").TS.systems
		graph.import(systems.store)
		local Controller = graph.import(systems.controller).Controller
		Controller.awake = function()
			error("fixture awake failure")
		end
		local ok, message = pcall(graph.import(graph.crucible.server).boot, { load = {} })
		assert(not ok and string.find(tostring(message), "fixture awake failure", 1, true), "boot reports hook failure")
		assert(graph.lifecycle(Controller) == "faulted", "faulted hook has a named lifecycle state")
		graph.place:Destroy()
	end,
	["Lune runtime preserves supplied game under installed node_modules"] = function()
		local game = {
			GetService = function()
				return {
					IsStudio = function()
						return false
					end,
					IsServer = function()
						return true
					end,
					IsClient = function()
						return false
					end,
				}
			end,
		}
		assert(harness.runtime({ game = game }).module("server"), "server entry loads with supplied game")
	end,
	["Lune graph Promise handlers are isolated"] = function()
		local a = harness.runtime().Promise()
		local b = harness.runtime().Promise()
		assert(a ~= b, "Promise implementation belongs to graph")
		local disconnect = a.onUnhandledRejection(function() end)
		disconnect()
	end,
	["Lune scheduler cancels waiting and delayed threads"] = function()
		local backend = harness.Backend.new()
		local realm = backend:realm()
		local calls = 0
		local waiting = realm.spawn(function()
			realm.wait(1)
			calls += 1
		end)
		local delayed = realm.delay(1, function()
			calls += 1
		end)
		realm.cancel(waiting)
		realm.cancel(delayed)
		local resumes = 0
		local resume = backend.resume
		backend.resume = function(self, ...)
			resumes += 1
			return resume(self, ...)
		end
		backend:advance(2)
		assert(resumes == 0, "cancelled timers never attempt a resume")
		assert(calls == 0 and coroutine.status(waiting) == "dead", "cancelled threads never resume")
	end,
	["Lune network faults consume every supported operation once"] = function()
		for _, name in { "create", "publish", "discover", "fireClient", "fireServer" } do
			local port = harness.networkPort()
			port.failNext = name
			local ok, message = pcall(port[name], {}, {}, {})
			assert(not ok and string.find(tostring(message), "injected " .. name, 1, true), name)
			assert(port.failNext == nil and #port.fired == 0, "failed operation consumed before send")
			assert(pcall(port[name], {}, {}, {}), "next operation succeeds")
		end
	end,
	["Lune graph replaces modules before consumers import them"] = function()
		local graph = harness.loadGame({ place = "build/sample-game.rbxl" })
		local module = graph.place:GetService("ReplicatedStorage").TS.profile
		local double = { PlayerData = {} }
		graph.provideGame(module, double)
		assert(graph.import(module) == double and graph.load(module) == double, "imports use replacement")
		local Controller = graph.import(graph.place:GetService("ServerScriptService").TS.systems.controller).Controller
		assert(graph.lifecycle(Controller) == "unconstructed", "not booted")
		graph.import(graph.crucible.testing)
		assert(not pcall(graph.provideGame, graph.crucible.testing, false), "evaluated replacement rejected")
		graph.place:Destroy()
	end,
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
		local player = realm.join(4120)
		local released = false
		local fixture = harness.profileFixture(realm, token, {
			onLoad = function(got, value)
				assert(got == player, "onLoad receives player")
				hookCalls += 1
				return value
			end,
			onRelease = function(got, value, reason)
				assert(got == player and reason == "leave", "onRelease receives player and reason")
				released = true
				return value
			end,
		})
		assert(fixture:state(player) == "absent", "no session before load")
		local loading = realm.run(function()
			return fixture:load(player, "join")
		end)
		backend:advance(1)
		assert(loading.done and loading.value == true, "compiled schema admitted")
		assert(fixture.published[player].coins == 9, "compiled migration from gold")
		assert(hookCalls == 1, "fixture hook")
		assert(fixture:state(player) == "active" and fixture:writable(player), "loaded session active")
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
		local releasing = realm.run(function()
			fixture:release(player)
		end)
		backend:advance(1)
		assert(releasing.done and released and not fixture:writable(player), "player-facing release")
		assert(fixture:state(player) == "ended", "release settled")
		graph.place:Destroy()
	end,
}
