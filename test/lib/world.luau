-- A fresh compiled lifecycle with recording effects. `scripts[name]` scripts one object's constructor, hooks
-- and effects. Lifecycle runs every hook through the real runner and `start` on a real thread. `defer` goes
-- through the fake engine's port, so `flush` runs deferred cancellations.
local Engine = require("./engine")
local spec = require("./load")

local world = {}

world.Phase = { Unconstructed = 0, Constructed = 1, Awake = 2, Enabled = 3, Disabled = 4, Faulted = 5, Destroyed = 6 }
world.Want = { Enabled = 0, Disabled = 1, Destroyed = 2 }
world.Hook = {
	Construct = 0,
	Awake = 1,
	OnEnable = 2,
	Start = 3,
	OnDisable = 4,
	OnDestroy = 5,
	Frame = 6,
	Settle = 9,
	Cancel = 10,
	Internal = 11,
}
world.Kind = { Subsystem = 0, Behaviour = 1 }

local HOOK_NAMES = {
	[world.Hook.Awake] = "awake",
	[world.Hook.OnEnable] = "onEnable",
	[world.Hook.OnDisable] = "onDisable",
	[world.Hook.OnDestroy] = "onDestroy",
}

function world.new()
	local graph = spec.runtime()
	local engine = Engine.new()
	local w = {
		life = graph.module("internal/lifecycle"),
		runner = graph.module("internal/runner"),
		DenseList = graph.module("internal/dense-list").DenseList,
		engine = engine,
		events = {},
		scripts = {},
		threads = {},
		failures = {},
		diagnosed = {},
		cancelError = nil,
		reconcile = function()
			return {}
		end,
	}
	local function log(s, what)
		table.insert(w.events, `{what}:{s.record.name}`)
	end
	local function scriptOf(s)
		return w.scripts[s.record.name]
	end
	local function effect(what)
		return function(s)
			log(s, what)
			local body = scriptOf(s)[what]
			if body then
				body(s)
			end
		end
	end
	w.life.useEffects({
		construct = function(s)
			log(s, "construct")
			local construct = scriptOf(s).construct
			if construct then
				construct(s)
			end
			return { name = s.record.name }
		end,
		hook = function(_, hook)
			return effect(HOOK_NAMES[hook])
		end,
		start = function(s)
			log(s, "start")
			local script = scriptOf(s)
			if script.start == "suspend" then
				w.threads[s.record.name] = coroutine.running()
				coroutine.yield()
			elseif type(script.start) == "function" then
				script.start(s)
			end
		end,
		join = effect("join"),
		leave = effect("leave"),
		tick = effect("tick"),
		untick = effect("untick"),
		reconcile = function()
			return w.reconcile()
		end,
		ended = function(s)
			log(s, if s.phase == world.Phase.Faulted then "faulted" else "destroyed")
		end,
		defer = engine.port.defer,
		cancel = function(thread)
			for name, suspended in w.threads do
				if suspended == thread then
					table.insert(w.events, `cancel:{name}`)
				end
			end
			if w.cancelError then
				error(w.cancelError)
			end
			engine.port.cancel(thread)
		end,
		failed = function(message)
			table.insert(w.failures, message)
		end,
		diagnose = function(s, hook, err)
			log(s, "diagnose")
			table.insert(w.diagnosed, { hook = hook, error = err })
		end,
	})
	function w.make(name, script, kind)
		w.scripts[name] = script or {}
		return w.life.track({ name = name, kind = kind or world.Kind.Behaviour }, nil)
	end
	function w.boot(...)
		local states = { ... }
		w.life.admit(states)
		return table.unpack(states)
	end
	function w.clear()
		table.clear(w.events)
	end
	function w.count(event)
		local n = 0
		for _, e in w.events do
			if e == event then
				n += 1
			end
		end
		return n
	end
	function w.flush()
		engine.flush()
	end
	return w
end

return world
