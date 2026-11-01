local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")

local CALLBACK = 8

local function declare(r)
	return {
		Ping = r.emit.toServer("Own.Ping", r.n.string),
		Who = r.emit.toServer("Own.Who", r.n.instance({ "Player" })),
		Ask = r.emit.askServer("Own.Ask", r.n.string, r.n.string, nil),
		Bare = r.emit.toServer("Own.Bare", r.n["nil"]),
	}
end

local function client(c)
	c.m = declare(c)
end

return {
	["52 a callback thread that finished holds no owner"] = function()
		local link = Link.new()
		local threads = {}
		link:boot(function(s)
			local m = declare(s)
			s.service("Owner", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function(_, text)
					threads[text] = coroutine.running()
					if text == "held" then
						coroutine.yield()
					end
				end)
			end)
		end)
		local runner = link.server.graph.module("internal/runner")
		local ann = link:join("Ann", client)
		ann.Network.send(ann.m.Ping, "done")
		ann.Network.send(ann.m.Ping, "held")
		link:flush()
		assert(runner.ownerOf(threads.held) ~= nil, "a running callback is owned")
		spec.eq(runner.ownerOf(threads.done), nil, "a finished callback releases its owner")
		link:resume(threads.held)
		spec.eq(runner.ownerOf(threads.held), nil, "so does one that finishes after yielding")
		link:assertClean()
	end,
	["52 current() inside event and request callbacks is the owner; observe binds to it and stops after its fault"] = function()
		local link = Link.new()
		local seen = {}
		local owner, held, set
		link:boot(function(s)
			local m = declare(s)
			set = s.emit.resolveAll("Things")
			s.service("Owner", function(self)
				owner = self
				self.ping = s.emit.receive(self, "ping", m.Ping, function(_, text)
					local state = s.state(owner)
					table.insert(seen, `{text} {s.life.current() == state}`)
					s.task.spawn(function()
						table.insert(seen, `spawned {s.life.current() == nil}`)
					end)
					if text == "hold" then
						set:observe(function(item)
							table.insert(seen, "first " .. item.name)
						end)
						held = coroutine.running()
						coroutine.yield()
						set:observe(function(item)
							table.insert(seen, "late " .. item.name)
						end)
						table.insert(seen, `after fault {s.life.current() == state}`)
					end
				end)
				self.ask = s.emit.receive(self, "ask", m.Ask, function(_, text)
					return `{text} {s.life.current() == s.state(owner)}`
				end)
			end)
		end)
		link.server.task = link.task
		local ann = link:join("Ann", client)
		ann.Network.send(ann.m.Ping, "event")
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Ask, "request")
		end)
		ann.Network.send(ann.m.Ping, "hold")
		link:flush()
		spec.eq(asked.value.value, "request true", "current() in a request callback")
		set:join({ name = "one" })
		link.server.life.fault(link.server.state(owner), CALLBACK, "owner faulted")
		set:join({ name = "two" })
		link:resume(held)
		set:join({ name = "three" })
		spec.list(seen, {
			"event true",
			"spawned true",
			"hold true",
			"spawned true",
			"first one",
			"after fault true",
		}, "owned until it returns, observe disconnected by the fault, nothing subscribed after it")
		link:assertClean()
	end,
	["29 45 retirement replies unavailable to an in-flight request and leaves events running"] = function()
		local link = Link.new()
		local threads, done = {}, {}
		local owner
		link:boot(function(s)
			local m = declare(s)
			s.service("Owner", function(self)
				owner = self
				self.ping = s.emit.receive(self, "ping", m.Ping, function(_, text)
					threads.event = coroutine.running()
					coroutine.yield()
					table.insert(done, "event " .. text)
				end)
				self.ask = s.emit.receive(self, "ask", m.Ask, function()
					threads.request = coroutine.running()
					coroutine.yield()
					table.insert(done, "request")
					return "late value"
				end)
			end)
		end)
		local ann = link:join("Ann", client)
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Ask, "x")
		end)
		ann.Network.send(ann.m.Ping, "running")
		link:flush()
		assert(threads.event and threads.request, "both callbacks started and yielded")
		link.server.life.fault(link.server.state(owner), CALLBACK, "owner faulted")
		link:flush()
		spec.eq(asked.done, true, "the ask settled at retirement")
		spec.eq(asked.value.ok, false, "failed")
		spec.eq(asked.value.reason, "unavailable", "reason")
		link:resume(threads.event)
		link:resume(threads.request)
		link:flush()
		spec.list(done, { "event running", "request" }, "neither callback was cancelled")
		local replies = link:sent(function(e)
			return e.to == "Ann" and (e.opcode == 2 or e.opcode == 3)
		end)
		spec.eq(#replies, 1, "a request replies at most once")
		spec.eq(h.hex(replies[1].packet), `03 {string.format("%02x", ann.m.Ask.index)} 01 00`, "ERROR unavailable")
		ann.Network.send(ann.m.Ping, "after")
		link:flush()
		spec.eq(link.server.logged("Crucible dropped Own.Ping: it has no live receiver"), 1, "retired receiver is gone")
		link:assertClean()
	end,
	["48 52 handler starts follow call order and a yielding callback never delays later starts"] = function()
		local link = Link.new()
		local started, finished, held = {}, {}, nil
		link:boot(function(s)
			local m = declare(s)
			s.service("Owner", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function(_, text)
					table.insert(started, text)
					if text == "1" then
						held = coroutine.running()
						coroutine.yield()
					end
					table.insert(finished, text)
				end)
			end)
		end)
		local ann = link:join("Ann", client)
		for i = 1, 5 do
			ann.Network.send(ann.m.Ping, tostring(i))
		end
		link:flush()
		spec.list(started, { "1", "2", "3", "4", "5" }, "starts in call order")
		spec.list(finished, { "2", "3", "4", "5" }, "the yielding first callback delayed none")
		link:resume(held)
		spec.list(finished, { "2", "3", "4", "5", "1" }, "completion order is free")
		link:assertClean()
	end,
	["30 72 a server callback's player is the engine sender, never a payload value"] = function()
		local link = Link.new()
		local seen = {}
		link:boot(function(s)
			local m = declare(s)
			s.service("Owner", function(self)
				self.who = s.emit.receive(self, "who", m.Who, function(player, named)
					table.insert(seen, `{player.Name} named {named.Name}`)
				end)
			end)
		end)
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		ann.Network.send(ann.m.Who, bea.player)
		link:flush()
		spec.list(seen, { "Ann named Bea" }, "sender from the engine")
		link:assertClean()
	end,
	["52 core rule 7 get binds a pending pair at an event call site and stays pending inside ClientReady"] = function()
		local link = Link.new()
		local results = {}
		local Door
		local first, second
		link:boot(function(s)
			local m = declare(s)
			Door = s.behaviour("Door")
			s.service("Doors", function(self)
				self.bare = s.emit.receive(self, "bare", m.Bare, function()
					table.insert(results, `event {Door:get(first) ~= nil}`)
				end)
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function()
					table.insert(results, `ready {Door:get(second) ~= nil}`)
				end)
			end)
		end)
		local engine = link.server.engine
		local ann = link:join("Ann", client, { boot = false })
		link:inject(ann, h.bytes("04"), {})
		link:flush()
		local bea = link:join("Bea", client, { boot = false })
		first = engine.instance("first", "Model")
		engine.addTag(first, "Door")
		local ix = string.format("%02x", link.server.catalog.definitionAt(0).index)
		spec.eq(link.server.catalog.definitionAt(0).name, "Own.Ask", "catalog order")
		local bare
		for i = 0, link.server.catalog.catalogSize() - 1 do
			if link.server.catalog.definitionAt(i).name == "Own.Bare" then
				bare = string.format("%02x", i)
			end
		end
		assert(ix and bare, "indices")
		link.server.remote.receive(ann.player, h.bytes(`00 {bare} 00`), {})
		second = engine.instance("second", "Model")
		engine.addTag(second, "Door")
		link.server.remote.receive(bea.player, h.bytes("04"), {})
		spec.list(results, { "event true", "ready false" }, "outside a drain get reconciles; inside within it waits")
		spec.eq(link.server.logged("Crucible get pending at"), 1, "warned once")
		link:assertClean()
	end,
}
