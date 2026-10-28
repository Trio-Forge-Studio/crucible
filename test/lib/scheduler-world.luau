local World = require("./world")
local names = { [0] = "preRender", [1] = "fixedUpdate", [2] = "update", [3] = "lateUpdate" }

return function(renders, rate)
	local w = World.new()
	local callers = {}
	for index, name in names do
		callers[index] = function(object, dt)
			local fn = object[name]
			if fn then
				fn(object, dt)
			end
		end
	end
	w.scheduler = w.graph
		.module("internal/scheduler").Scheduler
		.new(w.engine.port, { renders = renders ~= false, fixedRate = rate or 60 }, callers)
	function w.member(name, methods, priority, extra)
		local script = extra or {}
		script.tick = function(s)
			w.scheduler:add(s)
		end
		script.untick = function(s)
			w.scheduler:remove(s)
		end
		local state = w.make(name, script)
		local mask = 0
		for index, hook in names do
			if methods[hook] then
				mask += 2 ^ index
			end
		end
		state.record.hooks = mask
		state.record.renderPriority = priority
		w.boot(state)
		for hook, fn in methods do
			state.object[hook] = fn
		end
		return state
	end
	return w
end
