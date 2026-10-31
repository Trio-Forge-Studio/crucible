local spec = require("./lib/load")
local Engine = require("./lib/engine")
local platform = require("./lib/boot-platform")

local function world(studio)
	local e = Engine.new()
	local clocks = 0
	e.loggingPort.isStudio = function()
		return studio ~= false
	end
	e.loggingPort.now = function()
		clocks += 1
		return clocks
	end
	local g = spec.runtime(nil, e.loggingPort)
	local api = g.module("logging")
	return {
		e = e,
		g = g,
		api = api,
		log = api.Logging:getLogger("Delivery.Van"),
		clocks = function()
			return clocks
		end,
	}
end

return {
	["L1 disabled calls do no record render native or sink work"] = function()
		local w = world(false)
		local touched = 0
		w.log:debug(
			"{x}",
			{ x = setmetatable({}, {
				__tostring = function()
					touched += 1
					return "x"
				end,
			}) }
		)
		spec.eq(touched, 0, "Info filters Debug before normalization")
		spec.eq(w.clocks(), 0, "Info filters Debug before record creation")
		spec.eq(#w.e.gameLogs, 0, "Info filters Debug before output")
		w.api.Logging.defaultSink:setMinimum(4)
		touched = 0
		local context =
			{ x = setmetatable({}, {
				__tostring = function()
					touched += 1
					return "x"
				end,
			}) }
		for _, method in { "debug", "info", "warn", "error" } do
			w.log[method](w.log, "{x}", context)
		end
		spec.eq(touched, 0, "no normalization at any level")
		spec.eq(w.clocks(), 0, "no record clock at any level")
		spec.eq(#w.e.gameLogs, 0, "no output at any level")
		w.api.Logging.defaultSink:setMinimum(2)
		w.log:info("{x}", context)
		spec.eq(touched, 0, "Warning filters Info before normalization")
		spec.eq(w.clocks(), 0, "Warning filters Info before record creation")
		spec.eq(#w.e.gameLogs, 0, "Warning filters Info before output")
	end,
	["L2 aggregate tracks all writers including default and Studio live defaults"] = function()
		for _, studio in { true, false } do
			local w = world(studio)
			spec.eq(w.log:isEnabled(0), studio, "environment default")
			local high = w.api.Logging:addSink({ write = function() end }, 2)
			spec.eq(w.log:isEnabled(0), studio, "a higher sink keeps the lowest minimum")
			high:disconnect()
			local sub = w.api.Logging:addSink({ write = function() end }, 0)
			assert(w.log:isEnabled(0), "custom minimum")
			sub:setMinimum(3)
			spec.eq(w.log:isEnabled(0), studio, "default included")
			w.api.Logging.defaultSink:setMinimum(4)
			assert(not w.log:isEnabled(2), "minimum updated")
			assert(w.log:isEnabled(3), "error accepted")
			sub:disconnect()
			assert(not w.log:isEnabled(3), "disconnect recomputes")
		end
	end,
	["L3 subscriptions are identity idempotent and reconnect is fresh"] = function()
		local w = world()
		local delivered = 0
		local sink = {
			write = function()
				delivered += 1
			end,
		}
		local a = w.api.Logging:addSink(sink, 2)
		spec.eq(w.api.Logging:addSink(sink, 0), a, "same active subscription")
		a:disconnect()
		assert(not pcall(a.setMinimum, a, 99), "disconnected minimum still validates")
		a:disconnect()
		local b = w.api.Logging:addSink(sink, 0)
		assert(a ~= b, "fresh generation")
		a:disconnect()
		assert(w.log:isEnabled(0), "old disconnect cannot remove new")
		spec.eq(w.api.Logging:addSink(sink), b, "stale disconnect preserves live registration")
		w.log:info("once")
		spec.eq(delivered, 1, "one live delivery")
		local order = {}
		for index = 1, 24 do
			w.api.Logging:addSink({
				write = function()
					table.insert(order, index)
				end,
			})
		end
		w.log:info("ordered fanout")
		for index = 1, 24 do
			spec.eq(order[index], index, "registration order")
		end
	end,
	["L4 throwing yielding sinks disconnect once with traceback and fanout continues"] = function()
		for _, mode in { "throw", "yield" } do
			local w = world()
			local calls, later = 0, 0
			w.api.Logging:addSink({
				write = function()
					calls += 1
					if mode == "throw" then
						error("sentinel sink")
					else
						coroutine.yield()
					end
				end,
			})
			w.api.Logging:addSink({
				write = function()
					later += 1
				end,
			})
			w.log:info("first")
			w.log:info("second")
			spec.eq(calls, 1, "disconnected")
			spec.eq(later, 2, "caller and other sinks continue")
			spec.eq(#w.e.logs, 1, "one diagnostic")
			assert(w.e.logs[1].message:gsub("\\", "/"):match("\n[^\n]*test/logging:%d+"), "sink traceback frame")
		end
	end,
	["L5 whole emission guard survives cross logger remove and readd"] = function()
		local w = world()
		local other = w.api.Logging:getLogger("Other")
		local calls = 0
		local sub
		local sink = {}
		sink.write = function()
			calls += 1
			if calls > 3 then
				error("recursion")
			end
			sub:disconnect()
			sub = w.api.Logging:addSink(sink)
			other:info("nested")
		end
		sub = w.api.Logging:addSink(sink)
		w.log:info("outer")
		spec.eq(calls, 1, "no recursion")
		spec.eq(#w.e.gameLogs, 1, "no nested output")
	end,
	["L6 frozen records normalize flat context and isolate later sinks"] = function()
		local w = world()
		local instance = require("@lune/roblox").Instance.new("Folder")
		instance.Name = "Normalized"
		local parent = require("@lune/roblox").Instance.new("Folder")
		parent.Name = "Parent"
		instance.Parent = parent
		local p = platform()
		spec.eq(
			p.graph.module("internal/log-output").createRobloxLoggingPort().describeInstance(instance),
			"Parent.Normalized",
			"real adapter full name"
		)
		local original = { number = 42, flag = false, instance = instance, nested = {}, fn = function() end }
		local retained
		w.api.Logging:addSink({
			write = function(record)
				assert(table.isfrozen(record), "record frozen")
				assert(table.isfrozen(record.context), "context frozen")
				record.context.number = 99
			end,
		})
		w.api.Logging:addSink({
			write = function(record)
				retained = record
			end,
		})
		w.log:info("{number}", original)
		original.number = 7
		assert(table.isfrozen(retained), "retained record frozen")
		assert(table.isfrozen(retained.context), "retained context frozen")
		spec.eq(retained.context.number, 42, "snapshot")
		spec.eq(retained.context.instance, "Parent.Normalized", "instance full name")
		spec.eq(type(retained.context.nested), "string", "table accepted")
		spec.eq(type(retained.context.fn), "string", "function accepted")
		spec.eq(retained.context.flag, false, "boolean preserved")
		spec.eq(#w.e.logs, 1, "mutating sink disconnected")
	end,
	["L7 renderer handles holes escapes missing keys and newlines"] = function()
		local w = world()
		w.log:info("{n} {{n}} {missing} }}\n{flag} {text}", { n = 12, flag = false, text = "{n}" })
		spec.eq(w.e.gameLogs[1].message, "12 {n} {missing} }\nfalse {n}", "render once")
	end,
	["L8 game error is one spawned error with source and caller traceback"] = function()
		local p = platform()
		local api = p.graph.module("logging")
		local callerLine = debug.info(1, "l") + 1
		api.Logging:getLogger("Game.Error"):error("failed {id}", { id = 7 })
		spec.eq(#p.errors, 1, "one spawned error")
		spec.has(p.errors[1], "[Game.Error] failed 7", "source rendered")
		assert(p.errors[1]:sub(1, 1) == "[", "game error is not a sink failure")
		local firstFrame = p.errors[1]:match("\n+([^\n]+)")
		spec.has(firstFrame:gsub("\\", "/"), "test/logging:" .. callerLine, "exact game call site")
		api.Logging:getLogger("Game.Error"):error("later")
		spec.eq(#p.errors, 2, "default sink still connected")
		spec.has(p.errors[2], "[Game.Error] later", "later game output")
	end,
	["L9 private diagnostics bypass game sinks and disconnected default"] = function()
		local w = world()
		local received = 0
		w.api.Logging.defaultSink:disconnect()
		w.api.Logging:addSink({
			write = function()
				received += 1
			end,
		})
		local diagnostic = w.g.module("internal/diagnostics").runtimeDiagnostic
		diagnostic("warning", "unchanged warning")
		diagnostic("error", "unchanged fault\noriginal traceback")
		spec.eq(received, 0, "no game fanout")
		spec.eq(#w.e.logs, 2, "mandatory output")
		spec.eq(w.e.logs[2].message, "unchanged fault\noriginal traceback", "verbatim")
	end,
	["L10 native adapter bounds loggers escapes context and latches fallback"] = function()
		local p = platform()
		local created, writes, printed = {}, {}, {}
		local fail = false
		p.services.LogService = {
			GetLogger = function(_, name)
				table.insert(created, name)
				local function writer(method)
					return function(_, text, context)
						if fail then
							error("native unavailable")
						end
						local displayed = if context then text:gsub("{{", "{"):gsub("}}", "}") else text
						table.insert(
							writes,
							{ logger = name, method = method, text = text, context = context, displayed = displayed }
						)
					end
				end
				return { Output = writer("Output"), Info = writer("Info"), Warn = writer("Warn") }
			end,
		}
		local g = spec.runtime({
			game = p.game,
			task = p.task,
			print = function(text)
				table.insert(printed, text)
			end,
		})
		local port = g.module("internal/log-output").createRobloxLoggingPort()
		local levels = g.module("logging").LogLevel
		spec.list(
			{ levels.Debug, levels.Info, levels.Warning, levels.Error, levels.Off },
			{ 0, 1, 2, 3, 4 },
			"native level mapping contract"
		)
		local context = { field = 7 }
		for level = 0, 2 do
			port.writeGame({ level = level, source = "Native", message = "{literal}", context = context })
		end
		port.writeInternal("warning", "braces {literal}")
		spec.list(created, { "Game", "Crucible" }, "two native loggers")
		for index = 1, 3 do
			spec.eq(writes[index].text, "[Native] {{literal}}", "brace escaping")
			spec.eq(writes[index].context, context, "structured context")
		end
		for index, expected in { "Game.Output", "Game.Info", "Game.Warn", "Crucible.Warn" } do
			spec.eq(`{writes[index].logger}.{writes[index].method}`, expected, "native route " .. index)
		end
		spec.eq(writes[4].text, "[Crucible] braces {{literal}}", "warning escaping")
		assert(type(writes[4].context) == "table", "warning context supplied")
		spec.eq(writes[4].displayed, "[Crucible] braces {literal}", "warning displayed verbatim")
		fail = true
		port.writeGame({ level = 1, source = "Native", message = "failed", context = context })
		fail = false
		port.writeGame({ level = 1, source = "Native", message = "latched", context = context })
		spec.eq(#writes, 4, "native never retried")
		spec.eq(#printed, 2, "failed and future records fall back")
	end,
	["L11 names reject empty segments controls and excessive length"] = function()
		local w = world()
		for _, name in { "", ".A", "A.", "A..B", "A\nB", string.rep("a", 257) } do
			assert(not pcall(w.api.Logging.getLogger, w.api.Logging, name), "reject " .. name)
		end
		assert(w.api.Logging:getLogger("Delivery.Van"), "valid name")
	end,
	["L12 module load logging precedes startup and fixture evaluation"] = function()
		local w = world()
		w.log:info("before boot")
		spec.eq(w.e.gameLogs[1].message, "before boot", "module log")
		local h = require("./lib/place")()
		h.load(h.place:GetService("ServerScriptService").TS.main)
		spec.eq(#h.engine.gameLogs, 4, "sample levels before boot")
		h.place:Destroy()
	end,
	["L12 failed rendering drops the record and recovers"] = function()
		local w = world()
		local received = 0
		w.api.Logging:addSink({
			write = function()
				received += 1
			end,
		})
		w.log:info(
			"{value}",
			{ value = setmetatable({}, {
				__tostring = function()
					error("bad tostring")
				end,
			}) }
		)
		spec.eq(received, 0, "no sink delivery")
		spec.eq(#w.e.gameLogs, 0, "no default output")
		spec.eq(#w.e.logs, 1, "one diagnostic")
		spec.has(w.e.logs[1].message, "Logging failed", "rendering diagnostic")
		w.log:info("recovered")
		spec.eq(received, 1, "later records delivered")
	end,
	["L13 independent runtimes do not share logging state"] = function()
		local server, client = world(), world()
		local received = 0
		server.api.Logging:addSink({
			write = function()
				received += 1
			end,
		})
		client.log:info("client record")
		spec.eq(received, 0, "server sink does not receive client record")
		spec.eq(#server.e.gameLogs, 0, "server output isolated")
		spec.eq(#client.e.gameLogs, 1, "client output receives record")
		server.log:info("server record")
		spec.eq(received, 1, "server sink receives server record")
	end,
}
