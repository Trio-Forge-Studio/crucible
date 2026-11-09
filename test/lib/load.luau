local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local networkPort = require("./network-port")

type Node = { path: string, Parent: Node }

local ROOT = "packages/crucible/out"
-- Where `TS.getModule` finds a package, such as @rbxts/charm. Its modules see a `game` with only the services Charm
-- reads, Lune's `Instance`, `Enum` and `Color3`, and a `require` that takes a child node, a `./` path or an
-- `@self/` path.
local PACKAGES = "packages/crucible/node_modules"

local nodes: { [string]: Node } = {}

-- The bytecode optimization level for modules loaded after `spec.optimize` sets it. Unset, Lune's default applies.
local optimization: number? = nil

-- Changes each module's source as it loads, after `spec.rewrite` sets it, such as a changed copy of Charm.
local rewrite: ((path: string, source: string) -> string)? = nil

-- Every Luau file of the runtime and Charm, read before `spec.readAhead(true)` returns. Lune's fs calls yield, and a
-- yield inside a strict Charm batch or effect is an error, so while it is set, modules load from it alone.
local sources: { [string]: string }? = nil

local function isFile(path: string): boolean
	if sources then
		return sources[path] ~= nil
	end
	return fs.isFile(path)
end

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
	if isFile(path .. ".luau") then
		return path .. ".luau"
	elseif isFile(path .. "/init.luau") then
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

-- Charm asks RunService whether it runs in Studio, which turns on its strict and frozen flags. Specs set them.
local PACKAGE_SERVICE = {
	IsStudio = function()
		return false
	end,
}
local PACKAGE_GAME = {
	GetService = function()
		return PACKAGE_SERVICE
	end,
}

-- One isolated module graph. Module-level state (the reconciler, the worker pool) is per graph, so each test
-- that builds a runtime starts clean. Each graph gets its own network port: the given one, or the default fake.
local function graph(defaults, loggingPort, configPort, network, dataPort, root: string?)
	local folder = root or ROOT
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
						local child = string.match(target, "^@self/(.+)$")
						if child then
							return load(`{path}/{child}`)
						end
						local sibling = string.match(target, "^%.%./(.+)$")
						if sibling then
							return load(`{node(path).Parent.Parent.path}/{sibling}`)
						end
						local relative =
							assert(string.match(target, "^%./(.+)$"), `unsupported require {target} from {path}`)
						return load(`{node(path).Parent.path}/{relative}`)
					end
					return load(target.path)
				end
			end
			local file = fileOf(path)
			local source = if sources then sources[file] else fs.readFile(file)
			if rewrite then
				source = rewrite(path, source)
			end
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
			return load(if relative == "" then folder else folder .. "/" .. relative, globals)
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

function spec.rewrite(transform: ((path: string, source: string) -> string)?)
	rewrite = transform
end

local function readTree(folder: string, into: { [string]: string })
	for _, name in fs.readDir(folder) do
		local path = `{folder}/{name}`
		if fs.isDir(path) then
			readTree(path, into)
		elseif string.match(name, "%.luau$") then
			into[path] = fs.readFile(path)
		end
	end
end

-- On, reads the runtime and Charm now and loads every later module from memory, so a spec can load the runtime where
-- a yield is an error. Off, loads from disk again.
function spec.readAhead(on: boolean)
	if not on then
		sources = nil
		return
	end
	local read = {}
	readTree(ROOT, read)
	readTree(`{PACKAGES}/@rbxts/charm/src`, read)
	readTree(`{PACKAGES}/@rbxts/charm-sync`, read)
	sources = read
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

-- A fresh graph of the compiled runtime in `root` instead of packages/crucible/out, such as another commit's build.
function spec.runtimeAt(root: string, defaults, loggingPort)
	return graph(defaults, loggingPort, nil, nil, nil, root)
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
