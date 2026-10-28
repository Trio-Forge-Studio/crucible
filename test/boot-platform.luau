local spec = require("./lib/load")
local platform = require("./lib/boot-platform")

return {
	["server boot loads module registrations before sealing and constructs their dependency graph"] = function()
		local p = platform()
		p.services.ServerScriptService = p.node(
			"ServerScriptService",
			nil,
			{ p.node("Consumer", "ModuleScript"), p.node("Provider", "ModuleScript") }
		)
		local base = p.graph.module("subsystem").Subsystem
		local seen = {}
		p.onRequire = function(module)
			local class = setmetatable({}, { __index = base })
			class.__index = class
			function class.new(provider)
				local self = setmetatable({}, class)
				base.constructor(self)
				if module.Name == "Consumer" then
					assert(provider.ready, "provider constructed first")
				end
				self.ready = true
				table.insert(seen, module.Name)
				return self
			end
			p.graph.module("emit").defineClass(class, {
				id = module.Name,
				kind = "subsystem",
				deps = if module.Name == "Consumer" then { "Provider" } else {},
				provides = {},
			})
		end
		p.graph.module("server").boot({ load = { "crucible-path:ServerScriptService/*" } })
		spec.list(seen, { "Provider", "Consumer" }, "required before dependency seal")
		spec.eq(#p.closed, 1, "shutdown wired")
	end,
	["module walk resolves crucible-path stars globstars braces literals deduplicates and requires in full-name order"] = function()
		local p = platform()
		p.services.ServerScriptService = p.node("ServerScriptService", nil, {
			p.node("TS", nil, {
				p.node("core", nil, {
					p.node("A", nil, {
						p.node("server", nil, {
							p.node("ZuluService", "ModuleScript"),
							p.node("AlphaBehaviour", "ModuleScript"),
							p.node("IgnoredService"),
							p.node(
								"other",
								nil,
								{ p.node("BetaService", "ModuleScript"), p.node("NoMatch", "ModuleScript") }
							),
						}),
					}),
				}),
			}),
		})
		local boot = p.graph.module("internal/boot")
		boot.load({
			"crucible-path:ServerScriptService/TS/core/*/server/**/*{Service,Behaviour}",
			"crucible-path:ServerScriptService/TS/core/A/server/ZuluService",
		})
		spec.list(p.required, {
			"ServerScriptService.TS.core.A.server.AlphaBehaviour",
			"ServerScriptService.TS.core.A.server.ZuluService",
			"ServerScriptService.TS.core.A.server.other.BetaService",
		}, "sorted unique modules")
		local ok, err = pcall(boot.parsePattern, "src/server/*.ts")
		spec.eq(ok, false, "untransformed rejects")
		spec.has(err, "transformer", "actionable")
	end,
	["side entries enforce the side and client BOOTED_ATTRIBUTE is set only on successful boot"] = function()
		for _, client in { false, true } do
			local p = platform(client)
			local entry = p.graph.module(if client then "client" else "server")
			spec.eq(type(entry.boot), "function", "export")
			local ok = pcall(entry.boot, { load = { "untransformed" } })
			spec.eq(ok, false, "failed load")
			spec.eq(next(p.attributes), nil, "not booted")
			entry.boot({ load = {} })
			if client then
				spec.eq(entry.BOOTED_ATTRIBUTE, "CrucibleBooted", "constant")
				spec.eq(p.attributes.CrucibleBooted, true, "local player ready")
			end
			spec.eq(#p.closed, if client then 0 else 1, "server-only BindToClose")
			local wrong, err = pcall(p.graph.module, if client then "server" else "client")
			spec.eq(wrong, false, "wrong side rejects")
			spec.has(err, "loaded on", "reason")
		end
	end,
	["Roblox full port maps six frame events render names task cancellation BindToClose and Error output"] = function()
		local p = platform()
		local port = p.graph.module("internal/roblox-port").robloxPort()
		for index, name in
			{ "PreRender", "PreSimulation", "PostSimulation", "RenderStepped", "PreAnimation", "Heartbeat" }
		do
			local dt
			local stop = port.onFrame(index - 1, function(value)
				dt = value
			end)
			p.events[name](0.031)
			spec.eq(dt, 0.031, name)
			stop()
			spec.eq(p.events[name], nil, "disconnect " .. name)
		end
		local a = port.bindRenderStep("crucible:7", 7, function() end)
		local b = port.bindRenderStep("crucible:19", 19, function() end)
		spec.eq(p.bound["crucible:7"].priority, 7, "priority A")
		spec.eq(p.bound["crucible:19"].priority, 19, "priority B")
		a()
		assert(p.bound["crucible:19"], "other priority survives")
		b()
		spec.eq(next(p.bound), nil, "unbound")
		local called = 0
		port.defer(function()
			called += 1
		end)
		spec.eq(called, 0, "deferred")
		p.tasks[1]()
		spec.eq(called, 1, "ran")
		port.spawn(function()
			called += 1
		end)
		spec.eq(called, 2, "spawn")
		local thread = coroutine.create(function() end)
		port.cancel(thread)
		spec.eq(p.cancelled, thread, "cancel")
		port.wait()
		spec.eq(p.waited, true, "wait")
		assert(type(port.clock()) == "number", "monotonic clock")
		local close = function() end
		port.bindToClose(close)
		spec.eq(p.closed[1], close, "BindToClose")
		port.log(0, "warning sentinel")
		port.log(1, "error sentinel")
		spec.eq(#p.warnings, 1, "only warning uses warn")
		spec.has(p.errors[1], "error sentinel", "Error output")
	end,
}
