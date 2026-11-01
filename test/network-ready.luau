local Link = require("./lib/network-link")
local spec = require("./lib/load")

local CALLBACK = 8

local function declare(r)
	return { Note = r.emit.toClient("Ready.Note", r.n.string) }
end

local function listener(got)
	return function(c)
		local m = declare(c)
		c.service("Listener", function(self)
			self.note = c.emit.receive(self, "note", m.Note, function(text)
				table.insert(got, text)
			end)
		end)
	end
end

-- Server Subsystems `ids`, each constructed after the one before, each with a ClientReady field running `ready[id]`.
local function servers(link, ids, ready, extra)
	local objects = {}
	local m
	link:boot(function(s)
		m = declare(s)
		for index, id in ids do
			s.service(id, function(self)
				objects[id] = self
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function(player)
					return ready[id](player, self)
				end)
			end, nil, { deps = if index > 1 then { ids[index - 1] } else {} })
		end
		if extra then
			extra(s, m)
		end
	end)
	assert(link.server.booted, tostring(link.server.bootError))
	return objects, m
end

return {
	["33 46 the queue drains before ClientReady, which runs once per peer in owner construction order"] = function()
		local link = Link.new()
		local order, got = {}, {}
		local m
		_, m = servers(link, { "A", "B", "C" }, {
			A = function(player)
				table.insert(order, "A " .. player.Name)
				link.server.Network.send(player, m.Note, "from A")
			end,
			B = function(player)
				table.insert(order, "B " .. player.Name)
			end,
			C = function(player)
				table.insert(order, "C " .. player.Name)
				link.server.Network.send(player, m.Note, "from C")
			end,
		})
		local ann = link:join("Ann", listener(got), { boot = false })
		link.server.Network.send(ann.player, m.Note, "queued")
		link:start(ann)
		link:join("Bea", listener({}))
		spec.list(order, { "A Ann", "B Ann", "C Ann", "A Bea", "B Bea", "C Bea" }, "construction order per peer")
		spec.list(got, { "queued", "from A", "from C" }, "drained packets first, then what callbacks send")
		link:assertClean()
	end,
	["34 a ClientReady yield throw or bad return is a callback failure that faults nothing and later callbacks run"] = function()
		local link = Link.new()
		local resumed, last = false, {}
		local objects = servers(link, { "Yields", "Throws", "Returns", "Last" }, {
			Yields = function()
				coroutine.yield()
				resumed = true
			end,
			Throws = function()
				error("ready threw")
			end,
			Returns = function()
				return 5
			end,
			Last = function(player)
				table.insert(last, player.Name)
			end,
		})
		link:join("Ann")
		link:advance(60)
		spec.eq(resumed, false, "the runner cancelled the yielding callback")
		spec.list(last, { "Ann" }, "later callbacks still run")
		spec.eq(link.server.logged("Yields.ready for Ann failed: yielded"), 1, "yield reported")
		spec.eq(link.server.logged("Throws.ready for Ann failed:"), 1, "throw reported")
		spec.eq(link.server.logged("ready threw"), 1, "with its error")
		spec.eq(link.server.logged("Returns.ready for Ann failed: returned a number"), 1, "bad return reported")
		for id, object in objects do
			assert(link.server.life.isLive(link.server.state(object)), id .. " not faulted")
		end
		spec.eq(#link.kicks, 0, "the peer did not fail")
		link:join("Bea")
		spec.list(last, { "Ann", "Bea" }, "the next peer runs every callback again")
		spec.eq(link.server.logged("Throws.ready for Bea failed:"), 1, "reported per player")
		link:assertClean()
	end,
	["25 35 a conditional cleanup keeps a function for one player and nothing for another"] = function()
		local link = Link.new()
		local cleaned = {}
		servers(link, { "Cond" }, {
			Cond = function(player)
				return if player.Name == "Bea"
					then function()
						table.insert(cleaned, player.Name)
					end
					else nil
			end,
		})
		local ann = link:join("Ann")
		local bea = link:join("Bea")
		link:leave(ann)
		link:leave(bea)
		spec.list(cleaned, {}, "never inline at the end")
		link:flush()
		spec.list(cleaned, { "Bea" }, "only the returned cleanup runs, once")
		link:advance(60)
		spec.list(cleaned, { "Bea" }, "at most once")
		spec.eq(#link.server.diagnostics(), 0, "undefined is a valid return")
		link:assertClean()
	end,
	["35 76 cleanups run deferred on leave and failure in reverse ClientReady order, and on owner fault per peer"] = function()
		local link = Link.new()
		local ran = {}
		local function cleanup(id)
			return function(player)
				return function()
					table.insert(ran, `{id} {player.Name}`)
				end
			end
		end
		local objects = servers(link, { "A", "B" }, { A = cleanup("A"), B = cleanup("B") })
		local ann = link:join("Ann")
		local bea = link:join("Bea")
		local cid = link:join("Cid")
		link:leave(ann)
		spec.list(ran, {}, "deferred on leave")
		link:flush()
		spec.list(ran, { "B Ann", "A Ann" }, "leave runs them in reverse ClientReady order")
		table.clear(ran)
		link:inject(bea, buffer.fromstring("\5\0"), {})
		link:flush()
		spec.list(ran, { "B Bea", "A Bea" }, "a client FAIL ends the peer failed and runs them")
		spec.eq(link.kicks[1].message, "Connection problem, please rejoin.", "and kicks")
		table.clear(ran)
		local dee = link:join("Dee")
		link.server.life.fault(link.server.state(objects.A), CALLBACK, "owner faulted")
		spec.list(ran, {}, "deferred on owner fault, never inside its drain")
		link:flush()
		spec.list(ran, { "A Cid", "A Dee" }, "the owner's cleanups for every peer in peer creation order")
		link:leave(cid)
		link:leave(dee)
		link:flush()
		spec.list(ran, { "A Cid", "A Dee", "B Cid", "B Dee" }, "a retired owner's cleanup never runs again")
		link:assertClean()
	end,
	["29 35 36 R3 an owner retiring during its own callback runs the returned cleanup once and never stores it"] = function()
		local link = Link.new()
		local ran, calls = {}, 0
		servers(link, { "Self" }, {
			Self = function(player, object)
				calls += 1
				link.server.life.fault(link.server.state(object), CALLBACK, "retired itself")
				return function()
					table.insert(ran, player.Name)
				end
			end,
		})
		local ann = link:join("Ann")
		spec.eq(calls, 1, "called once")
		link:flush()
		spec.list(ran, { "Ann" }, "the late cleanup runs once, deferred")
		link:leave(ann)
		link:join("Bea")
		link:advance(60)
		spec.list(ran, { "Ann" }, "never stored, so leaving runs nothing more")
		spec.eq(calls, 1, "a retired owner's ClientReady is never called again")
		link:assertClean()
	end,
	["33 35 R3 a callback that ends its peer stops the pass and its cleanup runs after the end's"] = function()
		local link = Link.new()
		local ran, called = {}, {}
		local ann
		servers(link, { "A", "B", "C" }, {
			A = function(player)
				table.insert(called, "A")
				return function()
					table.insert(ran, "A " .. player.Name)
				end
			end,
			B = function(player)
				table.insert(called, "B")
				link:leave(ann)
				return function()
					table.insert(ran, "B " .. player.Name)
				end
			end,
			C = function()
				table.insert(called, "C")
			end,
		})
		ann = link:join("Ann", nil, { boot = false })
		link:inject(ann, buffer.fromstring("\4"), {})
		link:flush()
		spec.list(called, { "A", "B" }, "the pass stops at the ended peer")
		spec.list(ran, { "A Ann", "B Ann" }, "the late cleanup runs once, after what the end deferred")
		link:assertClean()
	end,
	["33 36 R3 a callback that retires a later owner skips that owner in the same pass and later"] = function()
		local link = Link.new()
		local called = {}
		local objects
		objects = servers(link, { "A", "B", "C" }, {
			A = function(player)
				table.insert(called, "A " .. player.Name)
				if player.Name == "Ann" then
					link.server.life.fault(link.server.state(objects.B), CALLBACK, "retired by A")
				end
			end,
			B = function(player)
				table.insert(called, "B " .. player.Name)
			end,
			C = function(player)
				table.insert(called, "C " .. player.Name)
			end,
		})
		link:join("Ann")
		link:join("Bea")
		spec.list(called, { "A Ann", "C Ann", "A Bea", "C Bea" }, "B retired before its turn")
		link:assertClean()
	end,
	["33 36 R3 a middle owner retiring itself in its callback never skips the next owner"] = function()
		local link = Link.new()
		local called = {}
		servers(link, { "A", "B", "C" }, {
			A = function(player)
				table.insert(called, "A " .. player.Name)
			end,
			B = function(player, object)
				table.insert(called, "B " .. player.Name)
				link.server.life.fault(link.server.state(object), CALLBACK, "retired itself")
			end,
			C = function(player)
				table.insert(called, "C " .. player.Name)
			end,
		})
		link:join("Ann")
		link:join("Bea")
		spec.list(called, { "A Ann", "B Ann", "C Ann", "A Bea", "C Bea" }, "C runs after B retires itself")
		link:assertClean()
	end,
	["31 32 ClientReady is a receive target only and one per Subsystem"] = function()
		local link = Link.new()
		link:boot(function(s)
			declare(s)
			s.service("Twice", function(self)
				self.first = s.emit.receive(self, "first", s.ClientReady, function() end)
				self.second = s.emit.receive(self, "second", s.ClientReady, function() end)
			end)
		end)
		spec.eq(link.server.booted, false, "a second ClientReady in one owner fails boot")
		spec.has(link.server.bootError, "Twice.second: Twice.first already receives ClientReady", "names both fields")
		local other = Link.new()
		local ok = other:boot(function(s)
			declare(s)
			s.service("One", function(self)
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function() end)
			end)
			s.service("Two", function(self)
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function() end)
			end)
		end)
		assert(ok, tostring(other.server.bootError))
		local sent, err = pcall(other.server.Network.send, nil, other.server.ClientReady)
		spec.eq(sent, false, "ClientReady cannot be sent")
		spec.has(err, "Network.send needs a message declared with toClient", "not a message")
		spec.eq(other.server.remote.attributes.Catalog, "Ready.Note", "never in the catalog")
		other:assertClean()
	end,
}
