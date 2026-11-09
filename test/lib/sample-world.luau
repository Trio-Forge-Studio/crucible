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
local Ownable = require("./ownable")
local ProfileWorld = require("./profile-world")
local UiLoad = require("./ui-load")

-- Gives Lune's Instances the events the ui entry connects.
require("./ui-instances")

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

-- Lune has no RBXScriptConnection, so a fake service hands out connections that the runtime's `own` disconnects.
local function signal()
	local listeners = {}
	return {
		listeners = listeners,
		Connect = function(_, listener)
			table.insert(listeners, listener)
			return Ownable.connection(function()
				local index = table.find(listeners, listener)
				if index then
					table.remove(listeners, index)
				end
			end)
		end,
	}
end

-- The classes the sample's JSX names. Vide maps each lowercase tag to its class through ReflectionService.
local UI_CLASSES = { "Frame", "ScreenGui", "TextButton", "TextLabel", "UIListLayout" }

-- A GuiService that holds the selection and the Roblox menu flag, without moving either itself.
local function guiService()
	local changed = {}
	return {
		SelectedObject = nil,
		MenuIsOpen = false,
		GetPropertyChangedSignal = function(_, name: string)
			changed[name] = changed[name] or signal()
			return changed[name]
		end,
		Select = function() end,
	}
end

-- One realm's view of the place: its services, its module loader and the prints its scripts made.
local function attach(self, realm, runtimeLib)
	realm.messages = {}
	realm.imported = {}
	realm.place = self.place
	realm.sample = {}
	realm.inputs = signal()
	realm.userInput = {
		InputBegan = realm.inputs,
		InputEnded = signal(),
		LastInputTypeChanged = signal(),
		TextBoxFocused = signal(),
		TextBoxFocusReleased = signal(),
		GamepadDisconnected = signal(),
		IsGamepadButtonDown = function()
			return false
		end,
		GetFocusedTextBox = function()
			return nil
		end,
	}
	local place = self.place
	if realm.player then
		roblox.Instance.new("PlayerGui").Parent = realm.player
	end
	local services = {
		RunService = {
			IsServer = function()
				return realm.side == "server"
			end,
			IsClient = function()
				return realm.side == "client"
			end,
			IsStudio = function()
				return false
			end,
			Heartbeat = signal(),
			PreRender = signal(),
		},
		Players = { LocalPlayer = realm.player },
		UserInputService = realm.userInput,
		GuiService = guiService(),
		TextService = {},
		ReflectionService = {
			GetClasses = function()
				local classes = {}
				for _, name in UI_CLASSES do
					table.insert(classes, { Name = name })
				end
				return classes
			end,
		},
	}
	local game = {
		GetService = function(_, name: string)
			return services[name] or place:GetService(name)
		end,
	}
	local env
	local load
	-- The runtime's ui entry, with the place's Vide that the sample renders with and the runtime's Charm.
	local uiRequire
	local function ui(relative: string)
		if uiRequire == nil then
			local vide = resolve(place:GetService("ReplicatedStorage"), {
				"rbxts_include",
				"node_modules",
				"@rbxts",
				"vide",
				"src",
			})
			uiRequire = UiLoad({
				packages = { ["@rbxts/vide"] = load(vide), ["@rbxts/charm"] = realm.graph.package("@rbxts/charm/src") },
				services = services,
				globals = {
					Instance = roblox.Instance,
					Enum = roblox.Enum,
					Color3 = roblox.Color3,
					Font = roblox.Font,
					UDim = roblox.UDim,
					UDim2 = roblox.UDim2,
					Vector2 = roblox.Vector2,
					task = env.task,
				},
			})
		end
		return uiRequire(table.unpack(string.split(relative, "/"), 2))
	end
	local function import(_, base, ...)
		local node = resolve(base, { ... })
		local relative = runtimeRelative(node)
		if relative ~= nil then
			realm.imported[relative] = true
			if relative == "ui" or string.sub(relative, 1, 3) == "ui/" then
				return ui(relative)
			end
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
					-- Vide requires its siblings by `./name`.
					require = function(target)
						if type(target) == "string" then
							local name = assert(string.match(target, "^%./(.+)$"), `unsupported require {target}`)
							target = assert(module.Parent:FindFirstChild(name), `{module:GetFullName()} has no {name}`)
						end
						return sampleRequire(target)
					end,
					task = env.task,
					typeof = roblox.typeof,
					Vector2 = roblox.Vector2,
					Vector3 = roblox.Vector3,
					Color3 = roblox.Color3,
					CFrame = roblox.CFrame,
					UDim = roblox.UDim,
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
		typeof = Ownable.typeof,
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
