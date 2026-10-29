local spec = require("./load")
local Engine = require("./engine")
return function(beforeBinding)
	local graph = spec.runtime()
	local w = { engine = Engine.new(), events = {}, graph = graph }
	w.emit = graph.module("emit")
	w.registry = graph.module("internal/registry")
	w.life = graph.module("internal/lifecycle")
	if beforeBinding then
		beforeBinding(graph)
	end
	w.binding = graph.module("internal/binding")
	w.claim = graph.module("internal/construction-claim")
	w.behaviour = graph.module("crucible-behaviour").CrucibleBehaviour
	w.hooks = graph.module("internal/hooks")
	w.binding.useEngine(w.engine.port)
	graph.module("internal/implementers").useImplementerPort(w.engine.port)
	w.life.useEffects({
		construct = w.binding.construct,
		hook = function(_s, hook)
			local name = ({ [1] = "awake", [2] = "onEnable", [4] = "onDisable", [5] = "onDestroy" })[hook]
			if name then
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
		reconcile = w.binding.reconcile,
		ended = function(s)
			w.binding.forget(s)
			if s.fault then
				w.engine.port.log(1, tostring(s.fault.error))
			end
		end,
		defer = w.engine.port.defer,
		cancel = w.engine.port.cancel,
		failed = function(message)
			w.engine.port.log(1, message)
		end,
		diagnose = function(_, _, message)
			w.engine.port.log(1, tostring(message))
		end,
	})
	function w.define(id, methods, options)
		options = options or {}
		local base = options.base or w.behaviour
		local class = setmetatable(methods or {}, { __index = base })
		class.__index = class
		function class.new(...)
			local self = setmetatable({}, class)
			w.behaviour.constructor(self)
			if options.init then
				options.init(self, ...)
			end
			return self
		end
		w.emit.defineClass(class, {
			kind = "behaviour",
			id = id,
			deps = options.deps or {},
			provides = options.provides or {},
			tag = options.tag or id,
			instance = options.instance or { "Model" },
			attributes = options.attributes,
		})
		return class, w.registry.recordOf(class)
	end
	function w.tagged(name, tag, attributes)
		local instance = w.engine.instance(name, "Model", attributes)
		w.engine.addTag(instance, tag)
		return instance
	end
	function w.boot()
		local order = w.registry.seal({ renders = false })
		local behaviours = {}
		for _, record in w.registry.records() do
			if record.kind == 1 then
				table.insert(behaviours, record)
			end
		end
		w.stop = w.binding.watch(behaviours)
		local initial = {}
		for _, record in order do
			record.state = w.life.track(record)
			table.insert(initial, record.state)
		end
		w.life.admit(initial)
		w.life.drain()
	end
	return w
end
