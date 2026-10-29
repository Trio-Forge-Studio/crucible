local spec = require("./load")
local Engine = require("./engine")

return function()
	local graph = spec.runtime()
	local w = {
		graph = graph,
		emit = graph.module("emit"),
		registry = graph.module("internal/registry"),
		life = graph.module("internal/lifecycle"),
		claim = graph.module("internal/construction-claim"),
		base = graph.module("crucible-object").CrucibleObject,
		subsystem = graph.module("subsystem").Subsystem,
		hooks = graph.module("internal/hooks"),
		engine = Engine.new(),
	}
	graph.module("internal/implementers").useImplementerPort(w.engine.port)
	w.life.useEffects({
		construct = function(s)
			w.claim.setClaim(s)
			return s.record.klass.new()
		end,
		hook = function(s, hook)
			local name = ({ [1] = "awake", [2] = "onEnable", [4] = "onDisable", [5] = "onDestroy" })[hook]
			if name and s.record.klass[name] then
				return function(state)
					state.object[name](state.object)
				end
			end
			return nil
		end,
		start = function(s)
			w.hooks.hooks().start(s.object, 0)
		end,
		join = function(s)
			for _, set in s.record.memberOf do
				set:join(s.object)
			end
		end,
		leave = function(s)
			for _, set in s.record.memberOf do
				set:leave(s.object)
			end
		end,
		tick = function() end,
		untick = function() end,
		reconcile = function()
			return {}
		end,
		ended = function() end,
		defer = w.engine.port.defer,
		cancel = w.engine.port.cancel,
		failed = function(message)
			error(message)
		end,
		diagnose = function() end,
	})
	function w.define(id, kind, methods, options)
		options = options or {}
		local base = if kind == "subsystem" then w.subsystem else w.base
		local class = setmetatable(methods or {}, { __index = base })
		class.__index = class
		function class.new()
			local self = setmetatable({}, class)
			base.constructor(self)
			if options.init then
				options.init(self)
			end
			return self
		end
		w.emit.defineClass(class, {
			id = id,
			kind = kind,
			deps = options.deps or {},
			provides = options.provides or {},
			tag = options.tag or id,
			instance = { "Model" },
			attributes = options.attributes,
		})
		return class, w.registry.recordOf(class)
	end
	function w.seal()
		return w.registry.seal({ renders = false })
	end
	function w.track(record, attributes)
		local s = w.life.track(record, nil, attributes)
		if record.kind == 0 then
			record.state = s
		end
		return s
	end
	function w.admit(states)
		w.life.admit(states)
		w.claim.clearClaim()
	end
	return w
end
