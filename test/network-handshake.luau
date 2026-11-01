local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")

local KICK = "Connection problem, please rejoin."

local function declare(r)
	return {
		Note = r.emit.toClient("Test.Note", r.n.string),
		Ping = r.emit.toServer("Test.Ping", r.n.string),
		Tag = r.emit.toClient("Test.Tag", r.n.instance({ "Part" })),
	}
end

local function opcodes(link, from, to)
	local out = {}
	for _, entry in
		link:sent(function(e)
			return e.from == from and e.to == to
		end)
	do
		table.insert(out, entry.opcode)
	end
	return out
end

local function texts(list)
	return table.concat(list, " ")
end

return {
	["46 48 79 READY flushes constructor awake and start sends in call order and ClientReady follows the drain"] = function()
		local link = Link.new()
		local server, client = {}, {}
		local ann = link:join("Ann", function(c)
			local m = declare(c)
			c.service("Listener", function(self)
				self.note = c.emit.receive(self, "note", m.Note, function(text)
					table.insert(client, text)
				end)
				c.Network.send(m.Ping, "c-constructor")
			end, {
				awake = function()
					c.Network.send(m.Ping, "c-awake")
				end,
				start = function()
					c.Network.send(m.Ping, "c-start")
				end,
			})
		end, { boot = false })
		link:boot(function(s)
			local m = declare(s)
			s.service("Greeter", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function(player, text)
					table.insert(server, `{player.Name}:{text}`)
				end)
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function(player)
					s.Network.send(player, m.Note, "ready")
				end)
				spec.eq(s.Network.send(ann.player, m.Note, "constructor"), true, "queued from a constructor")
			end, {
				awake = function()
					s.Network.send(ann.player, m.Note, "awake")
				end,
				start = function()
					s.Network.send(ann.player, m.Note, "start")
				end,
			})
		end)
		assert(link.server.booted, tostring(link.server.bootError))
		spec.eq(#opcodes(link, "Server", "Ann"), 0, "nothing fires to a waiting peer")
		link:start(ann)
		assert(ann.booted, tostring(ann.bootError))
		spec.eq(ann.attributes.CrucibleBooted, true, "client committed")
		spec.list(opcodes(link, "Ann", "Server"), { 4, 0, 0, 0 }, "READY precedes the flushed queue")
		spec.list(client, { "constructor", "awake", "start", "ready" }, "server queue drains before ClientReady sends")
		spec.list(server, { "Ann:c-constructor", "Ann:c-awake", "Ann:c-start" }, "client queue in call order")
		link:assertClean()
	end,
	["22 a send to a ready peer is one remote call fired before send returns"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", function(c)
			declare(c)
		end)
		local before = #link:sent()
		spec.eq(link.server.Network.send(ann.player, m.Note, "one"), true, "accepted")
		spec.eq(#link:sent(), before + 1, "fired before returning")
		spec.eq(link.server.Network.send(ann.player, m.Note, "two"), true, "accepted")
		spec.eq(#link:sent(), before + 2, "one call per send, no batching")
		link:assertClean()
	end,
	["47 74 a game packet before READY and a second READY drop under the sentinel without a reflush"] = function()
		local link = Link.new()
		local readies, pings = 0, 0
		local m
		link:boot(function(s)
			m = declare(s)
			s.service("Greeter", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function()
					pings += 1
				end)
				self.ready = s.emit.receive(self, "ready", s.ClientReady, function()
					readies += 1
				end)
			end)
		end)
		local ann = link:join("Ann", nil, { boot = false })
		local ix = string.format("%02x", m.Ping.index)
		link:inject(ann, h.bytes(`00 {ix} 00 00`), {})
		link:inject(ann, h.bytes(`00 {ix} 00 00`), {})
		link:flush()
		spec.eq(pings, 0, "no callback before READY")
		spec.eq(link.server.logged("a game packet before READY from Ann"), 1, "phase drop diagnosed once")
		spec.eq(#link.server.diagnostics(), 1, "the sentinel covers the second drop")
		spec.eq(link.server.Network.send(ann.player, m.Note, "held"), true, "queued")
		link:inject(ann, h.bytes("04"), {})
		link:flush()
		spec.eq(readies, 1, "ClientReady once")
		local bea = link:join("Bea", nil, { boot = false })
		link:inject(bea, h.bytes("04"), {})
		link:inject(bea, h.bytes("04"), {})
		link:inject(ann, h.bytes("04"), {})
		link:flush()
		spec.eq(readies, 2, "a second READY reruns nothing")
		spec.eq(link.server.logged("a second READY from Bea"), 1, "duplicate READY dropped under the sentinel")
		spec.eq(#link:sent(function(e)
			return e.to == "Ann"
		end), 1, "Ann's queue drained once")
		link:inject(ann, h.bytes(`00 {ix} 00 00`), {})
		link:flush()
		spec.eq(pings, 1, "game packets flow once ready")
		spec.eq(#link.kicks, 0, "no drop kicks")
		link:assertClean()
	end,
	["47 77 a game packet reaching a client before its READY fails its session and the server kicks"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", function(c)
			declare(c)
			coroutine.yield()
		end, { boot = false })
		link:start(ann)
		assert(ann.booted == nil, "client boot paused in load")
		local ix = string.format("%02x", m.Note.index)
		link:injectToClient(ann, h.bytes(`00 {ix} 00 00`), {})
		link:flush()
		spec.eq(ann.logged("Crucible network session failed (malformed): a game packet before READY"), 1, "violation")
		spec.list(opcodes(link, "Ann", "Server"), { 5 }, "FAIL malformed best effort")
		spec.eq(
			buffer.readu8(
				link:sent(function(e)
					return e.from == "Ann"
				end)[1].packet,
				1
			),
			3,
			"reason malformed"
		)
		spec.eq(#link.kicks, 1, "server kicks on any client FAIL")
		spec.eq(link.kicks[1].message, KICK, "KICK_MESSAGE")
		link:resume(ann.thread)
		link:flush()
		assert(ann.booted, tostring(ann.bootError))
		spec.list(opcodes(link, "Ann", "Server"), { 5 }, "a failed session never fires READY")
		link:assertClean()
	end,
	["50 77 queues hold 256 packets and 1 MiB of charge; the next send fails the peer, clears it and kicks"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local N = link.server.Network
		local ann = link:join("Ann", nil, { boot = false })
		for i = 1, 256 do
			assert(N.send(ann.player, m.Note, tostring(i)), `packet {i} queued`)
		end
		spec.eq(#link.kicks, 0, "256 fit")
		spec.eq(N.send(ann.player, m.Note, "257"), false, "packet 257 fails the peer")
		spec.eq(#link.kicks, 1, "kicked")
		spec.eq(link.kicks[1].message, KICK, "KICK_MESSAGE")
		spec.list(opcodes(link, "Server", "Ann"), { 5 }, "FAIL overflow before the kick, nothing queued fires")
		spec.eq(link.server.logged("Crucible network peer Ann failed (overflow); kicking"), 1, "reported once")
		spec.eq(N.send(ann.player, m.Note, "late"), false, "a failed peer accepts nothing")
		local big = string.rep("x", 65536)
		local function fill(player, last)
			for _ = 1, 15 do
				assert(N.send(player, m.Note, big), "a full string packet queues")
			end
			return N.send(player, m.Note, string.rep("y", last))
		end
		-- 15 packets of 3 + 3 + 65,536 bytes, then 3 + 3 + 65,440 reaches 1,048,576 exactly.
		local bea = link:join("Bea", nil, { boot = false })
		spec.eq(fill(bea.player, 65440), true, "exactly MAX_QUEUED_BYTES queues")
		spec.eq(N.send(bea.player, m.Note, ""), false, "one more byte fails")
		local cid = link:join("Cid", nil, { boot = false })
		spec.eq(fill(cid.player, 65441), false, "one byte over fails")
		-- A one-slot packet is 3 header bytes charged 3 + 64; it fits only if 67 bytes remain.
		local part = link:instance("Part")
		local function slotted(spare)
			local client = link:join("Slot" .. spare, nil, { boot = false })
			for _ = 1, 15 do
				assert(N.send(client.player, m.Note, big), "filler")
			end
			assert(N.send(client.player, m.Note, string.rep("z", 65446 - 6 - 67 + spare)), "filler to the edge")
			return N.send(client.player, m.Tag, part)
		end
		spec.eq(slotted(0), true, "a slot charges 64 bytes beside its 3")
		spec.eq(slotted(1), false, "one byte short for the slot charge fails")
		spec.eq(#link.kicks, 4, "each overflow kicks only its peer")
		link:assertClean()
	end,
	["50 49 a peer that never readies keeps its queue until it boots or leaves"] = function()
		local link = Link.new()
		local m
		local got = {}
		link:boot(function(s)
			m = declare(s)
		end)
		local function client(c)
			local mm = declare(c)
			c.service("Listener", function(self)
				self.note = c.emit.receive(self, "note", mm.Note, function(text)
					table.insert(got, text)
				end)
			end)
		end
		local ann = link:join("Ann", client, { boot = false })
		local bea = link:join("Bea", client, { boot = false })
		for i = 1, 10 do
			link.server.Network.send(ann.player, m.Note, tostring(i))
			link.server.Network.send(bea.player, m.Note, "b" .. i)
		end
		link:advance(3600)
		spec.eq(#link.kicks, 0, "no ready deadline")
		spec.eq(#link:sent(), 0, "nothing fired while waiting")
		link:leave(bea)
		spec.eq(link.server.Network.send(bea.player, m.Note, "gone"), false, "no peer after leaving")
		link:start(ann)
		spec.eq(texts(got), "1 2 3 4 5 6 7 8 9 10", "the whole queue in send order after an hour")
		link:assertClean()
	end,
}
