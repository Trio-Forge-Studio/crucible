-- Loads a fresh Crucible reactive core from a built runtime folder and a fresh Vide, both compiled at one
-- optimization level and both seeing the fake Instance.
--
-- With `patch`, the compiled flush gets one edit on load: `runRounds` sends a node marked `binding` to `runBinding`
-- instead of `runOnce`. That is the branch the spike's variant B (lib/binding.luau) needs in a trunk runtime. A
-- runtime with its own binding node loads unpatched. With `vide`, the runtime's import of `@rbxts/vide` gets that
-- Vide, so the bridge and the bench share one graph.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local task = require("@lune/task")
local Engine = require("../../../test/lib/engine")
local fake = require("./instance")

local VIDE = "spike/ui/vendor/vide"
local OPTIMIZATION = 2

local RUN_ONCE = "\t\t\trunOnce(effect)\n"
local RUN_ROUNDS = "local function runRounds()\n"
local RETURN = "return {\n\tuseReactivePort = useReactivePort,"

local function patchFlush(source: string): string
	local function replaceOnce(from: string, to: string)
		local first = string.find(source, from, 1, true)
		assert(first, `flush patch: {from} not found`)
		assert(string.find(source, from, first + 1, true) == nil, `flush patch: {from} found twice`)
		source = string.sub(source, 1, first - 1) .. to .. string.sub(source, first + #from)
	end
	replaceOnce(RUN_ONCE, "\t\t\tif effect.binding then runBinding(effect) else runOnce(effect) end\n")
	replaceOnce(RUN_ROUNDS, "local runBinding\n" .. RUN_ROUNDS)
	replaceOnce(
		RETURN,
		"return {\n\tuseBindingRunner = function(f) runBinding = f end,\n\tuseReactivePort = useReactivePort,"
	)
	return source
end

local function compile(source: string, path: string, env)
	local bytecode = luau.compile(source, { optimizationLevel = OPTIMIZATION })
	return luau.load(bytecode, { debugName = path, environment = env })()
end

type Node = { path: string, Parent: Node }

local VIDE_PACKAGE = table.freeze({ path = "@rbxts/vide/src" })

local function crucible(engine, root: string, patch: boolean, videLib)
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
	local loaded = {}
	local load
	local TS = {}
	function TS.getModule()
		return { src = VIDE_PACKAGE }
	end
	function TS.import(_caller, base: Node, ...: string)
		if base == VIDE_PACKAGE then
			return assert(videLib, "this runtime was loaded without Vide")
		end
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
	function load(path: string)
		if loaded[path] == nil then
			local file = if fs.isFile(path .. ".luau") then path .. ".luau" else path .. "/init.luau"
			local source = fs.readFile(file)
			if patch and string.match(path, "/internal/reactive/flush$") then
				source = patchFlush(source)
			end
			local env = { script = node(path), _G = G, task = task, typeof = fake.typeof }
			loaded[path] = compile(source, path, env)
			if string.match(path, "/internal/log%-output$") then
				loaded[path].installLoggingPort(engine.loggingPort)
			end
		end
		return loaded[path]
	end
	return function(relative: string)
		return load(root .. "/" .. relative)
	end
end

-- Vide reads RunService.Heartbeat and its class defaults at load. Defaults are turned off after load, so all three
-- variants write the same properties.
local function vide()
	local service = {
		Heartbeat = {
			Connect = function()
				return { Disconnect = function() end }
			end,
		},
		GetClasses = function()
			return {}
		end,
	}
	local game = {
		GetService = function()
			return service
		end,
	}
	local Enum = setmetatable({}, {
		__index = function(_, enum)
			return setmetatable({}, {
				__index = function(_, item)
					return `{enum}.{item}`
				end,
			})
		end,
	})
	local Color3 = {
		new = function(r, g, b)
			return { r, g, b }
		end,
		fromRGB = function(r, g, b)
			return { r / 255, g / 255, b / 255 }
		end,
	}
	local loaded = {}
	local load
	function load(name: string)
		if loaded[name] == nil then
			local env = {
				game = game,
				typeof = fake.typeof,
				Instance = fake.Instance,
				Enum = Enum,
				Color3 = Color3,
				task = task,
				UDim2 = {
					new = function(...)
						return { ... }
					end,
				},
				UDim = {
					new = function(...)
						return { ... }
					end,
				},
				Vector2 = {
					new = function(...)
						return { ... }
					end,
				},
				require = function(target: string)
					return load(assert(string.match(target, "^%./(.+)$"), target))
				end,
			}
			loaded[name] = compile(fs.readFile(`{VIDE}/{name}.luau`), name, env)
		end
		return loaded[name]
	end
	local lib = load("lib")
	local flags = load("flags")
	flags.strict = false
	flags.defaults = false
	return lib
end

local runtime = {}

-- A fresh Crucible realm loaded from `options.root`: the reactive API, the internal modules a compiled UI calls,
-- and its flush.
function runtime.crucible(options: { root: string, patch: boolean?, vide: any? })
	local engine = Engine.new()
	local module = crucible(engine, options.root, options.patch == true, options.vide)
	local r = {
		engine = engine,
		api = module("reactive"),
		graph = module("internal/reactive/graph"),
		flushModule = module("internal/reactive/flush"),
		ownership = module("internal/reactive/ownership"),
		context = module("internal/reactive/context"),
		runner = module("internal/runner"),
		diagnostics = module("internal/diagnostics"),
		module = module,
	}
	r.flushModule.useReactivePort(engine.port)
	r.flushModule.releaseBarrier()
	function r.flush()
		engine.flush()
	end
	return r
end

function runtime.vide()
	return vide()
end

runtime.fake = fake
runtime.OPTIMIZATION = OPTIMIZATION

return runtime
