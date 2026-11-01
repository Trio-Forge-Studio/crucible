local Engine = require("./lib/engine")
local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local loadPlace = require("./lib/place")
local spec = require("./lib/load")

local KICK = "Connection problem, please rejoin."

local function declare(r)
	local n = r.n
	return {
		Ping = r.emit.toServer("Peers.Ping", n.string),
		Note = r.emit.toClient("Peers.Note", n.string),
		Echo = r.emit.askServer("Peers.Echo", n.string, n.string, nil),
		Flag = r.emit.toServer("Peers.Flag", n.bool),
		Mark = r.emit.toClient("Peers.Mark", n.bool),
	}
end

local function hex(n)
	return string.format("%02x", n)
end

local function client(c)
	c.m = declare(c)
	c.notes = {}
	c.service("Listener", function(self)
		self.note = c.emit.receive(self, "note", c.m.Note, function(text)
			table.insert(c.notes, text)
		end)
	end)
end

local function to(name, opcode)
	return function(e)
		return e.to == name and (opcode == nil or e.opcode == opcode)
	end
end

return {
	["45 76 49 a request finishing after its peer ended sends nothing and a packet with no peer is reported once"] = function()
		local link = Link.new()
		local held, done = {}, {}
		link:boot(function(s)
			local m = declare(s)
			s.service("Owner", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function(player)
					held.event = coroutine.running()
					coroutine.yield()
					table.insert(done, "event " .. player.Name)
				end)
				self.echo = s.emit.receive(self, "echo", m.Echo, function(_, text)
					held.request = coroutine.running()
					coroutine.yield()
					table.insert(done, "request")
					return text
				end)
			end)
		end)
		local ann = link:join("Ann", client)
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Echo, "x")
		end)
		ann.Network.send(ann.m.Ping, "x")
		link:flush()
		local before = #link:sent(to("Ann"))
		link:leave(ann)
		link:resume(held.request)
		link:resume(held.event)
		link:flush()
		spec.list(done, { "request", "event Ann" }, "ending cancels no callback")
		spec.eq(#link:sent(to("Ann")), before, "the late reply is dropped")
		spec.eq(asked.done, nil, "the departed client never hears back")
		local bea = link:join("Bea", client)
		local cid = link:join("Cid", client)
		link:inject(bea, h.bytes("05 00"), {})
		link:inject(cid, h.bytes("05 00"), {})
		link:inject(bea, h.bytes("04"), {})
		link:inject(cid, h.bytes("04"), {})
		link:flush()
		spec.eq(#link.kicks, 2, "both kicked")
		spec.eq(
			link.server.logged("Crucible dropped a network packet from a player with no peer"),
			1,
			"once per session"
		)
		link:assertClean()
	end,
	["15 74 unknown indices drop under the sentinel and known wrong-kind ones under the message name"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local size = link.server.catalog.catalogSize()
		local function peer(name)
			local c = link:join(name, nil, { boot = false })
			link:inject(c, h.bytes("04"), {})
			link:flush()
			return c
		end
		local ann = peer("Ann")
		for index = size, size + 300 do
			local varuint = if index < 128
				then hex(index)
				else `{hex(index % 128 + 128)} {hex(math.floor(index / 128))}`
			link:inject(ann, h.bytes(`00 {varuint} 00`), {})
		end
		link:flush()
		spec.eq(
			link.server.logged("Crucible dropped an unknown message index from Ann"),
			1,
			"one sentinel for every unknown index"
		)
		spec.eq(#link.server.diagnostics(), 1, "no key derived from an untrusted index")
		local bea = peer("Bea")
		link:inject(bea, h.bytes(`00 {hex(m.Note.index)} 00 00`), {})
		link:inject(bea, h.bytes(`01 {hex(m.Ping.index)} 01 00 00`), {})
		link:inject(bea, h.bytes(`00 {hex(m.Echo.index)} 00 00`), {})
		link:inject(bea, h.bytes(`02 {hex(m.Echo.index)} 01 00 00`), {})
		link:flush()
		spec.eq(
			link.server.logged("Crucible dropped Peers.Note sent as the wrong kind of packet from Bea"),
			1,
			"toClient from a client"
		)
		spec.eq(
			link.server.logged("Crucible dropped Peers.Ping sent as the wrong kind of packet from Bea"),
			1,
			"REQUEST naming an event"
		)
		spec.eq(
			link.server.logged("Crucible dropped Peers.Echo sent as the wrong kind of packet from Bea"),
			1,
			"EVENT naming an ask"
		)
		spec.eq(
			link.server.logged("Crucible dropped a server-to-client packet from Bea"),
			1,
			"OK from a client, under the sentinel"
		)
		local cid = peer("Cid")
		local dee = peer("Dee")
		local eve = peer("Eve")
		link:inject(cid, h.bytes(`00 {hex(m.Ping.index)} 00 00`), {}, "extra")
		link:inject(dee, h.bytes(`00 {hex(m.Ping.index)} 00 00`))
		link:inject(eve, h.bytes(`00 {hex(m.Ping.index)} 00 00`), setmetatable({}, {}))
		link:flush()
		spec.eq(link.server.logged("Crucible dropped a malformed network packet from Cid"), 1, "a third argument")
		spec.eq(link.server.logged("Crucible dropped a malformed network packet from Dee"), 1, "a missing sidecar")
		spec.eq(link.server.logged("Crucible dropped a malformed network packet from Eve"), 1, "a metatabled sidecar")
		spec.eq(#link.kicks, 0, "never a kick")
		link:assertClean()
	end,
	["10 14 a late server declaration throws and a client-only name is unpublished"] = function()
		local link = Link.new()
		link:boot(function(s)
			declare(s)
		end)
		local ok, err = pcall(link.server.emit.toServer, "Doors.Toggle", link.server.n["nil"])
		spec.eq(ok, false, "late declaration")
		spec.has(
			err,
			"Doors.Toggle was declared after the network catalog sealed; declare messages in modules that boot loads.",
			"rule 10 message"
		)
		local ann = link:join("Ann", function(c)
			client(c)
			c.m.Only = c.emit.toServer("Peers.ClientOnly", c.n.string)
			c.m.Heard = c.emit.toClient("Peers.ClientHeard", c.n.string)
			c.service("Hearer", function(self)
				self.heard = c.emit.receive(self, "heard", c.m.Heard, function() end)
			end)
		end)
		spec.eq(
			ann.logged("Crucible Peers.ClientHeard is not in the server's network catalog"),
			1,
			"an unpublished receiver at commit"
		)
		local before = #link:sent()
		spec.eq(ann.Network.send(ann.m.Only, "x"), false, "send returns false")
		spec.eq(ann.Network.send(ann.m.Only, "x"), false, "again")
		spec.eq(#link:sent(), before, "nothing sent")
		spec.eq(ann.logged("Crucible Peers.ClientOnly is not in the server's network catalog"), 1, "once per message")
		link:assertClean()
	end,
	["77 R2 a bad client payload drops without a kick and a server EVENT the client cannot decode kicks"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", client)
		link:inject(ann, h.bytes(`00 {hex(m.Flag.index)} 00 07`), {})
		link:flush()
		spec.eq(
			link.server.logged("Crucible dropped Peers.Flag from Ann: payload: invalid boolean"),
			1,
			"dropped with a diagnostic"
		)
		spec.eq(#link.kicks, 0, "a client-sent payload never kicks")
		link:injectToClient(ann, h.bytes(`00 {hex(m.Mark.index)} 00 07`), {})
		link:flush()
		spec.eq(
			ann.logged("Crucible network session failed (malformed): Peers.Mark payload: invalid boolean"),
			1,
			"client fails"
		)
		local fail = link:sent(function(e)
			return e.from == "Ann" and e.opcode == 5
		end)
		spec.eq(h.hex(fail[1].packet), "05 03", "FAIL malformed")
		spec.eq(link.kicks[1].message, KICK, "the server kicks on it")
		spec.eq(link.server.logged("Crucible network peer Ann sent FAIL (malformed); kicking"), 1, "reported")
		for _, reason in { "00", "01", "02", "c8" } do
			local c = link:join("Fail" .. reason, client)
			link:inject(c, h.bytes(`05 {reason}`), {})
		end
		link:flush()
		spec.eq(#link.kicks, 5, "a client FAIL of any reason kicks")
		spec.eq(link.server.logged("sent FAIL (unknown reason 200)"), 1, "an unknown reason is reported as such")
		link:assertClean()
	end,
	["78 76 the session stays open through onShutdown, then closes with FAIL closing, no kick, and ends every peer"] = function()
		local link = Link.new()
		local m
		local cleaned = {}
		link:boot(function(s)
			m = declare(s)
			s.service("Holder", function(self)
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function(player)
					return function()
						table.insert(cleaned, player.Name)
					end
				end)
				self.ping = s.emit.receive(self, "ping", m.Ping, function()
					coroutine.yield()
				end)
			end)
			s.service("Saver", nil, {
				onShutdown = function()
					for _, c in link.clients do
						s.Network.send(c.player, m.Note, "bye")
					end
				end,
			})
		end)
		local gone = link:join("Gone", client)
		link:leave(gone)
		link:flush()
		spec.list(cleaned, { "Gone" }, "a departed peer's cleanup ran when it left")
		table.clear(cleaned)
		local goneSent = #link:sent(to("Gone"))
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		local waiting = link:join("Waits", nil, { boot = false })
		link.server.Network.send(waiting.player, m.Note, "queued")
		ann.Network.send(ann.m.Ping, "held")
		link:flush()
		spec.eq(link:pending(), 1, "the held callback's lifetime timer")
		spec.eq(#link.server.closers, 1, "BindToClose bound")
		link.server.closers[1]()
		link:flush()
		spec.list(cleaned, { "Ann", "Bea" }, "close ends every ready peer and runs its cleanup once")
		spec.eq(link:pending(), 0, "ending the peers cancelled the lifetime timer")
		spec.eq(#link:sent(to("Gone")), goneSent, "a peer that already ended gets no FAIL closing")
		spec.list(ann.notes, { "bye" }, "sends from onShutdown fire to ready peers")
		spec.list(bea.notes, { "bye" }, "every ready peer")
		for _, c in { ann, bea, waiting } do
			local last = link:sent(to(c.name))
			spec.eq(h.hex(last[#last].packet), "05 02", `{c.name} gets FAIL closing last`)
		end
		spec.eq(#link.kicks, 0, "no kick at shutdown")
		spec.eq(link.server.remote, nil, "remote destroyed")
		spec.eq(link.server.added, nil, "PlayerAdded disconnected")
		spec.eq(link.server.removing, nil, "PlayerRemoving disconnected")
		spec.eq(ann.logged("Crucible network session ended: the server sent FAIL (closing)"), 1, "the client ends")
		spec.eq(link.server.Network.send(ann.player, m.Note, "after"), false, "a closed session sends nothing")
		link:assertClean()
	end,
	["76 77 a failed client session admits no later packet"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", client)
		link:injectToClient(ann, h.bytes(`00 {hex(m.Mark.index)} 00 07`), {})
		link:injectToClient(ann, h.bytes(`00 {hex(m.Note.index)} 00 01 78`), {})
		link:injectToClient(ann, h.bytes(`00 {hex(m.Mark.index)} 00 07`), {})
		link:flush()
		spec.list(ann.notes, {}, "a valid EVENT after the failure runs no callback")
		spec.eq(ann.logged("Crucible network session failed"), 1, "one failure diagnostic")
		spec.eq(#link:sent(function(e)
			return e.from == "Ann" and e.opcode == 5
		end), 1, "one FAIL")
		spec.eq(#link.kicks, 1, "one kick")
		link:assertClean()
	end,
	["49 a second PlayerAdded for a present player keeps its one ready peer"] = function()
		local link = Link.new()
		local m
		local pings = {}
		link:boot(function(s)
			m = declare(s)
			s.service("Pinged", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function(player, text)
					table.insert(pings, player.Name .. " " .. text)
				end)
			end)
		end)
		local ann = link:join("Ann", client)
		link.server.added(ann.player)
		spec.eq(link.server.Network.send(ann.player, m.Note, "still"), true, "sent")
		ann.Network.send(ann.m.Ping, "here")
		link:flush()
		spec.list(ann.notes, { "still" }, "the server still sends to the ready peer")
		spec.list(pings, { "Ann here" }, "and still admits its packets")
		link:assertClean()
	end,
	["20 21 broadcast encodes once to every live peer now; later joiners miss it and an overflow fails one peer"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", client)
		local full = link:join("Full", nil, { boot = false })
		for i = 1, 256 do
			link.server.Network.send(full.player, m.Note, tostring(i))
		end
		local waits = link:join("Waits", client, { boot = false })
		link.server.Network.broadcast(m.Note, "all")
		local late = link:join("Late", client)
		link:start(waits)
		link:flush()
		spec.list(ann.notes, { "all" }, "ready peer")
		spec.list(waits.notes, { "all" }, "waiting peer, after READY")
		spec.list(late.notes, {}, "a later joiner never receives it")
		spec.eq(#link.kicks, 1, "only the full peer failed")
		spec.eq(link.kicks[1].player, "Full", "which one")
		local packets = link:sent(function(e)
			return e.opcode == 0 and e.from == "Server"
		end)
		spec.eq(h.hex(packets[1].packet), h.hex(packets[2].packet), "the same bytes to each peer")
		local unknown = link:instance("Player", "Nobody")
		spec.eq(link.server.Network.send(unknown, m.Note, "x"), false, "no peer for a player who never joined")
		link:assertClean()
	end,
	["56 an event with no live receiver drops with one diagnostic per message on each side"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", function(c)
			c.m = declare(c)
		end)
		for _ = 1, 3 do
			ann.Network.send(ann.m.Ping, "x")
			link.server.Network.send(ann.player, m.Note, "x")
		end
		link:flush()
		spec.eq(link.server.logged("Crucible dropped Peers.Ping: it has no live receiver"), 1, "server, once")
		spec.eq(ann.logged("Crucible dropped Peers.Note: it has no live receiver"), 1, "client, once")
		spec.eq(#link.kicks, 0, "the session continues")
		link:assertClean()
	end,
	["24 26 28 claims are checked on construction, unique per message and frozen at commit"] = function()
		local function boot(setup)
			local link = Link.new()
			link:boot(setup)
			return link
		end
		local twice = boot(function(s)
			local m = declare(s)
			for _, id in { "First", "Second" } do
				s.service(id, function(self)
					self.ping = s.emit.receive(self, "ping", m.Ping, function() end)
				end)
			end
		end)
		spec.has(twice.server.bootError, "Peers.Ping has two receivers: First.ping and Second.ping", "rule 26")
		local wrong = boot(function(s)
			local m = declare(s)
			s.service("Wrong", function(self)
				self.note = s.emit.receive(self, "note", m.Note, function() end)
			end)
		end)
		spec.has(
			wrong.server.bootError,
			"Wrong.note: Peers.Note is server-to-client; receive it in a client Subsystem.",
			"side"
		)
		local loose = boot(function(s)
			local m = declare(s)
			s.emit.receive({}, "loose", m.Ping, function() end)
		end)
		spec.has(
			loose.server.bootError,
			"receive() must initialize a readonly field of the Subsystem being constructed",
			"claim"
		)
		local behaviour = boot(function(s)
			local m = declare(s)
			s.behaviour("Door", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function() end)
			end)
		end)
		local door = behaviour.server.engine.instance("door", "Model")
		behaviour.server.engine.addTag(door, "Door")
		behaviour:flush()
		spec.eq(
			behaviour.server.logged("Door.ping: receive() must initialize a readonly field of a Subsystem"),
			1,
			"owner kind"
		)
		local clientSide = Link.new()
		clientSide:boot(function(s)
			declare(s)
		end)
		local ann = clientSide:join("Ann", function(c)
			local m = declare(c)
			c.service("Bad", function(self)
				self.ping = c.emit.receive(self, "ping", m.Ping, function() end)
			end)
		end)
		spec.has(
			ann.bootError,
			"Bad.ping: Peers.Ping is client-to-server; receive it in a server Subsystem.",
			"client side"
		)
		local calls = 0
		local late
		local frozen = Link.new()
		frozen:join("Ann", nil, { boot = false })
		frozen:boot(function(s)
			local m = declare(s)
			late = s.service("Late", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function()
					calls += 1
				end)
			end)
			s.service("Early", nil, {
				awake = function()
					local player = s.link.clients[1].player
					s.remote.receive(player, h.bytes("04"), {})
					s.remote.receive(player, h.bytes(`00 {hex(m.Ping.index)} 00 00`), {})
				end,
			}, { deps = { "Late" } })
		end)
		spec.eq(frozen.server.bootError, nil, "boot commits")
		local held = frozen.server.registry.recordOf(late).state.object.ping
		spec.eq(type(held), "table", "rule 27: an opaque Receiver")
		spec.eq(next(held), nil, "holding no connection")
		assert(table.isfrozen(held), "frozen")
		spec.eq(pcall(held), false, "not callable")
		local record = frozen.server.registry.recordOf(late)
		local again = frozen.server.life.track(record)
		frozen.server.life.admit({ again })
		spec.eq(frozen.server.logged("Late.ping: receive() after the network commit"), 1, "rule 28")
		spec.eq(calls, 0, "no callback ran")
		frozen:assertClean()
	end,
	["7 untransformed macros throw at module evaluation"] = function()
		local root = spec.runtime(nil, Engine.new().loggingPort).module("")
		for _, call in
			{
				function()
					root.toServer("A.B")
				end,
				function()
					root.toClient("A.B")
				end,
				function()
					root.askServer("A.B")
				end,
				function()
					root.receive(root.ClientReady, function() end)
				end,
			}
		do
			local ok, err = pcall(call)
			spec.eq(ok, false, "throws")
			spec.has(err, "Crucible networking needs the @trioforge/crucible/transformer plugin", "rule 7")
		end
	end,
	["8 the root and emit never reach a session, a port, the envelope or a side entry"] = function()
		local place = loadPlace()
		local root = place.import(place.crucible)
		assert(root.toServer and root.ClientReady, "root exports networking")
		place.import(place.crucible.emit)
		for module in place.loaded do
			local name = module:GetFullName()
			for _, forbidden in
				{
					"network.session",
					"network.server-session",
					"network.client-session",
					"network.port",
					"network.roblox-network-port",
					"network.envelope",
					"crucible.out.server",
					"crucible.out.client",
				}
			do
				assert(not string.find(name, forbidden, 1, true), `{name} loaded from the root`)
			end
		end
		place.place:Destroy()
		local graph = spec.runtime()
		local emit = graph.module("emit")
		emit.toServer("Iso.Thing", emit.n.string)
		emit.toClient("Iso.Other", emit.n["nil"])
		spec.eq(graph.network.installed, false, "a declaration creates no port, remote or listener")
	end,
}
