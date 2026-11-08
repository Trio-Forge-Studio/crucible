-- The built sample place, run over the two-realm fakes. The profile world supplies the link, the shared data backend
-- and the server's data port; this adds a fake ConfigService and loads the sample's own compiled scripts from
-- build/sample-game.rbxl. The runtime comes from packages/crucible/out, the package the place was built from: the
-- place's RuntimeLib is replaced by a TS.import that resolves the sample's runtime imports into each realm's graph, so
-- the sample's compiled modules and the runtime share the same module state. The sample's imports of Charm resolve
-- to the copy the runtime loads, as `TS.getModule` makes them share one in the place.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local ConfigBackend = require("./config-backend")
local ProfileWorld = require("./profile-world")

-- Defines Instance:WaitForChild for Lune once per process.
require("./place")

local World = setmetatable({}, { __index = ProfileWorld })
World.__index = World

local RUNTIME = "out"

local function runtimeRelative(node): string?
	local names = {}
	local current = node
	while current ~= nil do
		if current.Name == RUNTIME and current.Parent and current.Parent.Name == "crucible" then
			return table.concat(names, "/")
		end
		table.insert(names, 1, current.Name)
		current = current.Parent
	end
	return nil
end

-- The path of a node under the place's node_modules, such as "@rbxts/charm/src", or nil outside it.
local function packageRelative(node): string?
	local names = {}
	local current = node
	while current ~= nil do
		if current.Name == "node_modules" then
			return table.concat(names, "/")
		end
		table.insert(names, 1, current.Name)
		current = current.Parent
	end
	return nil
end

local function resolve(base, names: { string })
	local node = base
	for _, name in names do
		node = assert(node:FindFirstChild(name), `{node:GetFullName()} has no child {name}`)
	end
	return node
end

local function signal()
	local listeners = {}
	return {
		listeners = listeners,
		Connect = function(_, listener)
			table.insert(listeners, listener)
			return {
				Disconnect = function()
					table.remove(listeners, table.find(listeners, listener))
				end,
			}
		end,
	}
end

-- One realm's view of the place: its services, its module loader and the prints its scripts made.
local function attach(self, realm, runtimeLib)
	realm.messages = {}
	realm.imported = {}
	realm.place = self.place
	realm.sample = {}
	realm.inputs = signal()
	realm.userInput = { InputBegan = realm.inputs }
	local place = self.place
	local playerGui
	if realm.player then
		playerGui = roblox.Instance.new("Folder")
		playerGui.Name = "PlayerGui"
		playerGui.Parent = realm.player
	end
	local services = {
		RunService = {
			IsServer = function()
				return realm.side == "server"
			end,
			IsClient = function()
				return realm.side == "client"
			end,
		},
		Players = { LocalPlayer = realm.player },
		UserInputService = realm.userInput,
	}
	local game = {
		GetService = function(_, name: string)
			return services[name] or place:GetService(name)
		end,
	}
	local env
	local load
	local function import(_, base, ...)
		local node = resolve(base, { ... })
		local relative = runtimeRelative(node)
		if relative ~= nil then
			realm.imported[relative] = true
			return realm.graph.module(relative)
		end
		-- The runtime finds Charm with TS.getModule, so in the place the sample and the runtime share one copy.
		local package = packageRelative(node)
		if package ~= nil and string.sub(package, 1, #"@rbxts/charm") == "@rbxts/charm" then
			return realm.graph.package(package)
		end
		return load(node)
	end
	local TS = { import = import }
	local function sampleRequire(module)
		if module == runtimeLib then
			return TS
		end
		return load(module)
	end
	function load(module)
		if realm.sample[module] == nil then
			local chunk = luau.load(module.Source, {
				debugName = module:GetFullName(),
				environment = {
					game = game,
					script = module,
					require = sampleRequire,
					task = env.task,
					typeof = roblox.typeof,
					Vector3 = roblox.Vector3,
					Color3 = roblox.Color3,
					CFrame = roblox.CFrame,
					UDim2 = roblox.UDim2,
					Instance = roblox.Instance,
					Enum = roblox.Enum,
					print = function(...)
						local values = {}
						for _, value in { ... } do
							table.insert(values, tostring(value))
						end
						table.insert(realm.messages, table.concat(values, " "))
					end,
				},
			})
			realm.sample[module] = { value = chunk() }
		end
		return realm.sample[module].value
	end
	env = {
		game = game,
		task = self.link.task,
		typeof = roblox.typeof,
		Vector3 = roblox.Vector3,
		Color3 = roblox.Color3,
		CFrame = roblox.CFrame,
		Enum = roblox.Enum,
		require = sampleRequire,
	}
	return env, load
end

-- Replaces a realm's runtime graph with one whose boot reads the place's scripts.
local function rebuild(self, realm, dataPort, configPort)
	local spec = require("./load")
	local env, load = attach(self, realm, self.place:GetService("ReplicatedStorage").rbxts_include.RuntimeLib)
	realm.graph = spec.runtime(env, realm.engine.loggingPort, configPort, realm.port, dataPort)
	realm.graph.module("internal/roblox-port").robloxPort = function()
		return realm.engine.port
	end
	realm.load = load
	-- Lune's fs.readFile yields, so every module a booting realm needs is loaded before its script runs on a thread.
	realm.graph.module("")
	realm.graph.module("emit")
	realm.graph.module(realm.side)
end

-- Options: backend shares a data backend with another world.
function World.new(options)
	options = options or {}
	local configs = ConfigBackend.new()
	local place = roblox.deserializePlace(fs.readFile("build/sample-game.rbxl"))
	local world = ProfileWorld.new({
		backend = options.backend,
		declare = function()
			return {}
		end,
		configs = function()
			return configs.port
		end,
		patch = function(w)
			function w.link.boot()
				return true
			end
			function w.link.start(_, client)
				client.booted = true
			end
		end,
	})
	world.configs = configs
	world.place = place
	world.prints = {}
	setmetatable(world, World)
	local server = world.server
	local dataPort = server.graph.module("internal/data/port").dataPort()
	rebuild(world, server, dataPort, configs.port)
	world.wire = {
		envelope = server.graph.module("internal/network/envelope"),
		codec = server.graph.module("internal/network/codec"),
		frame = server.graph.module("internal/sync/frame"),
	}
	-- Shutdown waits on the link's timeline, so a spec runs the close callback on a thread and advances time.
	server.engine.port.wait = function(seconds: number?)
		local thread = coroutine.running()
		world.link:delay(seconds or 1 / 60, function()
			world.link:resume(thread)
		end)
		coroutine.yield()
	end
	return world
end

-- Moves the link's timeline and the fake ConfigService's together, since the config runtime waits on the latter.
function World:advance(seconds: number)
	self.configs:advance(0)
	ProfileWorld.advance(self, seconds)
	self.configs:advance(seconds)
	self.link:flush()
end

-- Runs the place's server script.
function World:bootServer()
	local main = self.place:GetService("ServerScriptService").TS.main
	local server = self.server
	server.load(main)
	self.link:flush()
	self.link:assertClean()
	server.main = main
	return main
end

-- A client realm for player `userId`, booted from the place's client script.
function World:connect(name: string, userId: number)
	local client = ProfileWorld.player(self, name, userId)
	rebuild(self, client, nil, nil)
	local main = self.place:GetService("StarterPlayer").StarterPlayerScripts.TS.main
	client.main = main
	self.link:resume(coroutine.create(function()
		client.load(main)
	end))
	self.link:flush()
	return client
end

-- Presses `key` in the client's UserInputService. `processed` is the engine's flag for input a UI element already took.
function World:press(client, key, processed: boolean?)
	for _, listener in table.clone(client.inputs.listeners) do
		listener({ KeyCode = key }, processed == true)
	end
	self.link:flush()
end

-- The text of the client's HUD label.
function World:hud(client): string?
	local gui = client.player:FindFirstChild("PlayerGui")
	local screen = gui and gui:FindFirstChild("CoinHud")
	local label = screen and screen:FindFirstChild("Coins")
	return label and label.Text
end

return World
