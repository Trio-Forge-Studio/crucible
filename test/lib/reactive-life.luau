-- A compiled lifecycle driven by scripts, with the reactive core attached the way boot attaches it: the enable
-- interval ends through `LifeEffects.disabled`, and an effect that fails after a hook is logged in `effectFailures`.
-- `scripts[id]` holds an object's hooks; `lookup(hook)` runs when lifecycle asks for a hook caller, so a spec can make
-- it throw. `globals` override Lune's for every module. `strict` turns on Charm's strict mode before anything runs.
local Engine = require("./engine")
local spec = require("./load")

local HOOKS = { [1] = "awake", [2] = "onEnable", [4] = "onDisable", [5] = "onDestroy" }

return function(globals: { [string]: any }?, strict: boolean?)
	local engine = Engine.new()
	local graph = spec.runtime(globals, engine.loggingPort)
	local w = {
		engine = engine,
		graph = graph,
		events = {},
		scripts = {},
		failures = {},
		effectFailures = {},
		threads = {},
	}
	w.charm = graph.package("@rbxts/charm/src")
	w.charm.flags.strict = strict == true
	w.api = graph.module("")
	w.life = graph.module("internal/lifecycle")
	w.ownership = graph.module("internal/reactive/ownership")
	w.Want = { Enabled = 0, Disabled = 1, Destroyed = 2 }
	w.Hook = { Awake = 1, OnEnable = 2, Frame = 6 }
	w.ownership.useDescribe(engine.port.describe)
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
		effectFailed = function(s, hook, err)
			table.insert(w.effectFailures, { id = if s then s.record.id else nil, hook = hook, error = tostring(err) })
		end,
		disabled = w.ownership.endInterval,
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
