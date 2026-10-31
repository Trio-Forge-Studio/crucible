-- A compiled lifecycle driven by scripts, with the reactive core attached the way boot attaches it: the enable
-- interval ends through `LifeEffects.disabled`, and the flush defers through the fake engine. `scripts[id]` holds
-- an object's hooks; `lookup(hook)` runs when lifecycle asks for a hook caller, so a spec can make it throw.
local Engine = require("./engine")
local spec = require("./load")

local HOOKS = { [1] = "awake", [2] = "onEnable", [4] = "onDisable", [5] = "onDestroy" }

return function()
	local engine = Engine.new()
	local graph = spec.runtime(nil, engine.loggingPort)
	local w = { engine = engine, graph = graph, events = {}, scripts = {}, failures = {}, threads = {} }
	w.api = graph.module("")
	w.life = graph.module("internal/lifecycle")
	w.Want = { Enabled = 0, Disabled = 1, Destroyed = 2 }
	w.Hook = { Frame = 6 }
	local reactive = graph.module("internal/reactive/flush")
	reactive.useReactivePort(engine.port)
	reactive.releaseBarrier()
	local function log(s, what)
		table.insert(w.events, `{what}:{s.record.id}`)
	end
	w.life.useEffects({
		construct = function(s)
			return { id = s.record.id }
		end,
		hook = function(s, hook)
			local script = w.scripts[s.record.id]
			if script.lookup then
				script.lookup(hook)
			end
			local name = HOOKS[hook]
			if name == nil then
				return nil
			end
			return function(state)
				log(state, name)
				local body = script[name]
				if body then
					body(state)
				end
			end
		end,
		start = function(s)
			local body = w.scripts[s.record.id].start
			if body then
				body(s)
			end
		end,
		join = function() end,
		leave = function() end,
		tick = function() end,
		untick = function() end,
		reconcile = function()
			return {}
		end,
		ended = function(s)
			log(s, "ended")
		end,
		defer = engine.port.defer,
		cancel = engine.port.cancel,
		failed = function(message)
			table.insert(w.failures, message)
		end,
		diagnose = function() end,
		disabled = graph.module("internal/reactive/ownership").endInterval,
	})
	function w.make(id, script)
		w.scripts[id] = script or {}
		return w.life.track({ id = id, kind = 1 })
	end
	function w.boot(...)
		w.life.admit({ ... })
	end
	function w.log(entry)
		table.insert(w.events, entry)
	end
	function w.since(mark)
		local out = {}
		for i = mark + 1, #w.events do
			table.insert(out, w.events[i])
		end
		return out
	end
	function w.flush()
		engine.flush()
	end
	return w
end
