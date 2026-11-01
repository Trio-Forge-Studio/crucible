local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")

local function declare(r)
	local n = r.n
	return {
		Ping = r.emit.toServer("Calls.Ping", n.string),
		Slow = r.emit.toServer("Calls.Slow", n.string),
		Echo = r.emit.askServer("Calls.Echo", n.string, n.string, 60),
		Any = r.emit.askServer("Calls.Any", n["nil"], n.dynamic, nil),
		Note = r.emit.toClient("Calls.Note", n.string),
	}
end

local function client(c)
	c.m = declare(c)
end

-- A server whose callbacks are `script.ping`, `script.slow`, `script.echo`, `script.any` and `script.ready`.
local function server(link, script)
	local owner
	link:boot(function(s)
		local m = declare(s)
		link.m = m
		s.service("Owner", function(self)
			owner = self
			for _, name in { "Ping", "Slow", "Echo", "Any" } do
				local key = string.lower(name)
				local callback = script[key]
				if callback then
					self[key] = s.emit.receive(self, key, m[name], callback)
				end
			end
			if script.ready then
				self.ready = s.emit.receive(self, "ready", s.ClientReady, script.ready)
			end
		end)
	end)
	assert(link.server.booted, tostring(link.server.bootError))
	return owner
end

local function factory(value)
	return function()
		return value
	end
end

return {
	["53 75 a throwing callback faults nothing, later packets still start, and failures are per player"] = function()
		local link = Link.new()
		local seen = {}
		local owner = server(link, {
			ping = function(player, text)
				table.insert(seen, `{player.Name} {text}`)
				if text == "boom" then
					error("handler threw")
				end
			end,
		})
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		for _, text in { "boom", "boom", "after" } do
			ann.Network.send(ann.m.Ping, text)
		end
		bea.Network.send(bea.m.Ping, "boom")
		link:flush()
		spec.list(seen, { "Ann boom", "Ann boom", "Ann after", "Bea boom" }, "later packets start")
		spec.eq(link.server.logged("Owner.ping for Ann failed:"), 1, "once per player per field")
		spec.eq(link.server.logged("Owner.ping for Bea failed:"), 1, "Ann's failure does not hide Bea's")
		spec.eq(link.server.logged("handler threw"), 2, "with the error")
		assert(link.server.life.isLive(link.server.state(owner)), "the owner did not fault")
		spec.eq(#link.kicks, 0, "the peer did not fail")
		link:assertClean()
	end,
	["53 K5 a factory-made callback is checked when it returns"] = function()
		local link = Link.new()
		local Promise = spec.Promise()
		server(link, {
			ping = factory(5),
			echo = function(_, text)
				if text == "promise" then
					return Promise.resolve("x")
				elseif text == "number" then
					return 7
				end
				return nil
			end,
			any = function() end,
		})
		local ann = link:join("Ann", client)
		ann.Network.send(ann.m.Ping, "x")
		local promised = link:run(function()
			return ann.Network.ask(ann.m.Echo, "promise")
		end)
		local wrong = link:run(function()
			return ann.Network.ask(ann.m.Echo, "number")
		end)
		local nothing = link:run(function()
			return ann.Network.ask(ann.m.Echo, "nothing")
		end)
		local any = link:run(function()
			return ann.Network.ask(ann.m.Any)
		end)
		link:flush()
		spec.eq(
			link.server.logged("Owner.ping for Ann failed: returned a number; event callbacks return nothing"),
			1,
			"event return"
		)
		spec.eq(promised.value.reason, "handler-error", "a Promise is a handler error")
		spec.eq(
			link.server.logged("Owner.echo for Ann failed: returned a Promise; request callbacks return the response"),
			1,
			"reported"
		)
		spec.eq(wrong.value.reason, "handler-error", "a value its response codec rejects")
		spec.eq(nothing.value.reason, "handler-error", "nothing is not a string")
		spec.eq(any.value.ok, true, "K2: nothing replies nil to an unknown response")
		spec.eq(any.value.value, nil, "nil")
		local other = Link.new()
		server(other, {})
		local bea = other:join("Bea", function(c)
			c.m = declare(c)
			c.service("Listener", function(self)
				self.note = c.emit.receive(self, "note", c.m.Note, factory(true))
			end)
		end)
		other.server.Network.send(bea.player, other.m.Note, "x")
		other.server.Network.send(bea.player, other.m.Note, "y")
		other:flush()
		spec.eq(
			bea.logged("Listener.note failed: returned a boolean; event callbacks return nothing"),
			1,
			"client side, once per field per session"
		)
		link:assertClean()
		other:assertClean()
	end,
	["54 an event at MAX_ACTIVE_HANDLERS fails the peer and a request there gets overloaded"] = function()
		local link = Link.new()
		server(link, {
			slow = function()
				coroutine.yield()
			end,
			echo = function(_, text)
				return text
			end,
		})
		local ann = link:join("Ann", client)
		for i = 1, 64 do
			ann.Network.send(ann.m.Slow, tostring(i))
		end
		link:flush()
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Echo, "x")
		end)
		link:flush()
		spec.eq(asked.value.reason, "overloaded", "a request at the ceiling")
		spec.eq(#link.kicks, 0, "requests never fail the peer")
		ann.Network.send(ann.m.Slow, "65")
		link:flush()
		spec.eq(link.kicks[1].message, "Connection problem, please rejoin.", "an event at the ceiling fails and kicks")
		local fail = link:sent(function(e)
			return e.to == "Ann" and e.opcode == 5
		end)
		spec.eq(h.hex(fail[1].packet), "05 01", "FAIL overloaded")
		link:assertClean()
	end,
	["55 a callback past its lifetime is diagnosed, stops counting, is never cancelled and still replies"] = function()
		local link = Link.new()
		local held, finished = {}, 0
		server(link, {
			slow = function()
				table.insert(held, coroutine.running())
				coroutine.yield()
				finished += 1
			end,
			echo = function(_, text)
				table.insert(held, coroutine.running())
				coroutine.yield()
				return text
			end,
		})
		local ann = link:join("Ann", client)
		for i = 1, 63 do
			ann.Network.send(ann.m.Slow, tostring(i))
		end
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Echo, "late")
		end)
		link:advance(29.9)
		spec.eq(link.server.logged("still running after 30 seconds"), 0, "within its lifetime")
		link:advance(0.1)
		spec.eq(
			link.server.logged("Owner.slow for Ann failed: still running after 30 seconds"),
			1,
			"diagnosed once per field"
		)
		spec.eq(link.server.logged("Owner.echo for Ann failed: still running after 30 seconds"), 1, "the request too")
		ann.Network.send(ann.m.Slow, "64")
		ann.Network.send(ann.m.Slow, "65")
		link:flush()
		spec.eq(#link.kicks, 0, "overrun callbacks no longer count toward the ceiling")
		for _, thread in held do
			link:resume(thread)
		end
		link:flush()
		spec.eq(finished, 65, "every event callback finished")
		spec.eq(asked.value.value, "late", "the request past its lifetime still replies")
		link:assertClean()
	end,
	["76 ending a peer cancels its lifetime timers and no callback"] = function()
		local link = Link.new()
		local held
		server(link, {
			slow = function()
				held = coroutine.running()
				coroutine.yield()
			end,
		})
		local ann = link:join("Ann", client)
		ann.Network.send(ann.m.Slow, "x")
		link:flush()
		link:leave(ann)
		spec.eq(link:pending(), 0, "the lifetime timer was cancelled")
		link:advance(60)
		spec.eq(link.server.logged("still running"), 0, "no overrun after the end")
		spec.eq(coroutine.status(held), "suspended", "the callback was not cancelled")
		link:resume(held)
		spec.eq(#link.server.diagnostics(), 0, "it finished cleanly")
		link:assertClean()
	end,
	["75 76 R4 failures after the peer ended use the session key, once per field, never recreating the peer's set"] = function()
		local link = Link.new()
		local held = {}
		server(link, {
			ping = function(player)
				held[player.Name] = coroutine.running()
				coroutine.yield()
				error("late throw")
			end,
			slow = function(player)
				held["slow " .. player.Name] = coroutine.running()
				coroutine.yield()
				error("overran then threw")
			end,
			ready = function(player)
				return function()
					error(`cleanup threw for {player.Name}`)
				end
			end,
		})
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		local cid = link:join("Cid", client)
		cid.Network.send(cid.m.Slow, "x")
		link:advance(30)
		spec.eq(
			link.server.logged("Owner.slow for Cid failed: still running after 30 seconds"),
			1,
			"overrun while live"
		)
		ann.Network.send(ann.m.Ping, "x")
		bea.Network.send(bea.m.Ping, "x")
		link:flush()
		link:leave(ann)
		link:resume(held.Ann)
		spec.eq(link.server.logged("Owner.ping failed:"), 1, "after the end, under the session key")
		link:leave(bea)
		link:resume(held.Bea)
		spec.eq(link.server.logged("late throw"), 1, "a second player's late throw from that field is not reported")
		spec.eq(link.server.logged("Owner.ping for"), 0, "the ended peers' sets were not recreated")
		link:leave(cid)
		link:resume(held["slow Cid"])
		spec.eq(link.server.logged("overran then threw"), 0, "one callback run reports at most once")
		link:flush()
		spec.eq(link.server.logged("Owner.ready failed: "), 1, "a deferred cleanup after the end uses the session key")
		spec.eq(link.server.logged("cleanup threw for"), 1, "once for the field")
		link:assertClean()
	end,
}
