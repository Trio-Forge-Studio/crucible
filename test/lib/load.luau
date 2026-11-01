-- Loads roblox-ts package output in Lune. Each module gets a `script` node and a `_G` that maps every script to
-- a TS table whose `import` resolves sibling paths, which is all the compiled package uses.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local task = require("@lune/task")
local networkPort = require("./network-port")

type Node = { path: string, Parent: Node }

local ROOT = "packages/crucible/out"

local nodes: { [string]: Node } = {}

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
-- source is read here because Lune's readFile yields, and `TS.Promise` resolves inside a metamethod.
local PROMISE_SOURCE = fs.readFile("node_modules/roblox-ts/include/Promise.lua")
local promise
local function Promise()
	if promise == nil then
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

-- One isolated module graph. Module-level state (the reconciler, the worker pool) is per graph, so each test
-- that builds a runtime starts clean. Each graph gets its own network port: the given one, or the default fake.
local function graph(defaults, loggingPort, configPort, network)
	network = network or networkPort()
	local loaded: { [string]: any } = {}
	local load: (string, { [string]: any }?) -> any
	local TS = setmetatable({}, {
		__index = function(_, key)
			return if key == "Promise" then Promise() else nil
		end,
	})
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
			local chunk = luau.load(fs.readFile(fileOf(path)), { debugName = path, environment = env })
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
		end
		return loaded[path]
	end
	return {
		network = network,
		module = function(relative: string, globals: { [string]: any }?): any
			return load(if relative == "" then ROOT else ROOT .. "/" .. relative, globals)
		end,
	}
end

local spec = {}

local shared = graph()

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
function spec.runtime(defaults, loggingPort, configPort, network)
	return graph(defaults, loggingPort, configPort, network)
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
