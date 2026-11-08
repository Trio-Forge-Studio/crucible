-- A fresh runtime and its own copy of Charm, as the runtime imports it. `patch(graph)` runs before the runtime loads,
-- so it can replace a module's export. `globals` override Lune's for every module, such as a `typeof` that knows fake
-- engine values. `strict` turns on Charm's strict mode, which Studio turns on, before anything is created.
local Engine = require("./engine")
local spec = require("./load")

return function(patch: ((any) -> ())?, globals: { [string]: any }?, strict: boolean?)
	local engine = Engine.new()
	local graph = spec.runtime(globals, engine.loggingPort)
	if patch then
		patch(graph)
	end
	local r = { engine = engine, graph = graph }
	r.charm = graph.package("@rbxts/charm/src")
	r.charm.flags.strict = strict == true
	r.api = graph.module("")
	r.ownership = graph.module("internal/reactive/ownership")
	r.ownership.useDescribe(engine.port.describe)
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
