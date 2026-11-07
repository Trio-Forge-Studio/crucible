local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local networkPort = require("./network-port")

type Node = { path: string, Parent: Node }

local ROOT = "packages/crucible/out"
-- Where `TS.getModule` finds a package, such as @rbxts/vide for the vide subpath. Its modules see a `game` with
-- only the services Vide reads, Lune's `Instance`, `Enum` and `Color3`, and a `require` that takes a child node or a
-- `./` path.
local PACKAGES = "packages/crucible/node_modules"

local nodes: { [string]: Node } = {}

-- The bytecode optimization level for modules loaded after `spec.optimize` sets it. Unset, Lune's default applies.
local optimization: number? = nil

local function node(path: string): Node
	local existing = nodes[path]
	if existing then
		return existing
	end
	local created = setmetatable({ path = path }, {
		__index = function(self, key)
			if key == "Parent" then
				return node(string.match(self.path, "^(.*)/[^/]+$") or ".")
			end
			if type(key) == "string" and string.sub(self.path, 1, #PACKAGES + 1) == PACKAGES .. "/" then
				return node(`{self.path}/{key}`)
			end
			return nil
		end,
	}) :: any
	nodes[path] = created
	return created
end

local function fileOf(path: string): string
	if fs.isFile(path .. ".luau") then
		return path .. ".luau"
	elseif fs.isFile(path .. "/init.luau") then
		return path .. "/init.luau"
	end
	error(`no compiled module at {path}; run the package build first`)
end

-- roblox-ts emits `TS.Promise` for the Promise global; the RuntimeLib's copy needs only a Heartbeat to load. Its
-- source is read here because Lune's readFile yields, and `TS.Promise` resolves inside a metamethod. Scratch copies
-- without node_modules never reach it.
local PROMISE_PATH = "crates/compiler/roblox-ts/Promise.lua"
local PROMISE_SOURCE = if fs.isFile(PROMISE_PATH) then fs.readFile(PROMISE_PATH) else nil
local promise
local function Promise()
	if promise == nil then
		assert(PROMISE_SOURCE, `TS.Promise needs {PROMISE_PATH}`)
		local runService = {
			Heartbeat = {
				Connect = function()
					return { Disconnect = function() end }
				end,
			},
		}
		promise = luau.load(PROMISE_SOURCE, {
			debugName = "Promise",
			environment = {
				game = {
					GetService = function()
						return runService
					end,
				},
				task = task,
			},
		})()
	end
	return promise
end

-- Vide connects to RunService.Heartbeat and lists ReflectionService's classes when it loads.
local PACKAGE_SERVICE = {
	Heartbeat = {
		Connect = function()
			return { Disconnect = function() end }
		end,
	},
	GetClasses = function()
		return {}
	end,
}
local PACKAGE_GAME = {
	GetService = function()
		return PACKAGE_SERVICE
	end,
}

-- One isolated module graph. Module-level state (the reconciler, the worker pool) is per graph, so each test
-- that builds a runtime starts clean. Each graph gets its own network port: the given one, or the default fake.
local function graph(defaults, loggingPort, configPort, network, dataPort)
	network = network or networkPort()
	local loaded: { [string]: any } = {}
	local load: (string, { [string]: any }?) -> any
	local TS = setmetatable({}, {
		__index = function(_, key)
			return if key == "Promise" then Promise() else nil
		end,
	})
	function TS.getModule(_caller: Node, scope: string, name: string): Node
		return node(`{PACKAGES}/{scope}/{name}`)
	end
	function TS.import(_caller: Node, base: Node, ...: string)
		local path = base.path
		for _, name in { ... } do
			path ..= "/" .. name
		end
		return load(path)
	end
	local G = setmetatable({}, {
		__index = function()
			return TS
		end,
	})
	function load(path: string, globals: { [string]: any }?): any
		if loaded[path] == nil then
			local env: { [string]: any } = { script = node(path), _G = G, task = task }
			for key, value in defaults or {} do
				env[key] = value
			end
			for key, value in globals or {} do
				env[key] = value
			end
			if string.sub(path, 1, #PACKAGES + 1) == PACKAGES .. "/" then
				env.game = PACKAGE_GAME
				env.Instance = roblox.Instance
				env.Enum = roblox.Enum
				env.Color3 = roblox.Color3
				env.require = function(target: Node | string)
					if type(target) == "string" then
						local relative =
							assert(string.match(target, "^%./(.+)$"), `unsupported require {target} from {path}`)
						return load(`{node(path).Parent.path}/{relative}`)
					end
					return load(target.path)
				end
			end
			local source = fs.readFile(fileOf(path))
			if optimization then
				source = luau.compile(source, { optimizationLevel = optimization })
			end
			local chunk = luau.load(source, { debugName = path, environment = env })
			loaded[path] = chunk()
			if string.match(path, "/internal/log%-output$") and loggingPort then
				loaded[path].installLoggingPort(loggingPort)
			end
			if string.match(path, "/internal/configs/port$") and configPort then
				loaded[path].installConfigPort(configPort)
			end
			if string.match(path, "/internal/network/port$") then
				loaded[path].installNetworkPort(network)
				network.installed = true
			end
			if string.match(path, "/internal/data/port$") and dataPort then
				loaded[path].installDataPort(dataPort)
			end
		end
		return loaded[path]
	end
	return {
		network = network,
		module = function(relative: string, globals: { [string]: any }?): any
			return load(if relative == "" then ROOT else ROOT .. "/" .. relative, globals)
		end,
		-- A module of an installed package, the same instance the runtime's imports of it get.
		package = function(relative: string): any
			return load(`{PACKAGES}/{relative}`)
		end,
	}
end

local spec = {}

local shared = graph()

function spec.optimize(level: number)
	optimization = level
end

-- Loads `packages/crucible/out/<relative>` once per process.
function spec.module(relative: string, globals: { [string]: any }?): any
	return shared.module(relative, globals)
end

-- Loads `relative` in a fresh graph, so its module-level state and its imports' do not leak between tests.
-- Globals given here override Lune's for that module only.
function spec.fresh(relative: string, globals: { [string]: any }?): any
	return graph().module(relative, globals)
end

-- A fresh graph to load several modules that must share state, such as lifecycle and its runner.
function spec.runtime(defaults, loggingPort, configPort, network, dataPort)
	return graph(defaults, loggingPort, configPort, network, dataPort)
end

spec.Promise = Promise

function spec.eq(actual: any, expected: any, label: string)
	if actual ~= expected then
		error(`{label}: expected {tostring(expected)}, got {tostring(actual)}`, 2)
	end
end

function spec.list(actual: { any }, expected: { any }, label: string)
	local a, e = table.concat(actual, " "), table.concat(expected, " ")
	if a ~= e then
		error(`{label}:\n         expected {e}\n         got      {a}`, 2)
	end
end

function spec.has(text: any, part: string, label: string)
	if string.find(tostring(text), part, 1, true) == nil then
		error(`{label}: "{part}" not in {tostring(text)}`, 2)
	end
end

return spec
