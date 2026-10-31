-- A fresh runtime whose reactive core has the fake engine's port and a released barrier, as boot's commit leaves it.
-- `patch(graph)` runs before the flush module loads, so it can replace an export the flush imports, which is how
-- specs inject an internal error where the flush calls into ownership, the graph or the diagnostic sink.
local Engine = require("./engine")
local spec = require("./load")

return function(patch: ((any) -> ())?)
	local engine = Engine.new()
	local graph = spec.runtime(nil, engine.loggingPort)
	if patch then
		patch(graph)
	end
	local r = { engine = engine, graph = graph }
	r.api = graph.module("")
	r.reactive = graph.module("internal/reactive/flush")
	r.limits = graph.module("internal/reactive/limits")
	r.reactive.useReactivePort(engine.port)
	r.reactive.releaseBarrier()
	function r.flush()
		engine.flush()
	end
	function r.logged(part: string): number
		return engine.logged(part)
	end
	function r.root(setup)
		return r.api.scope(setup)
	end
	return r
end
