local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")

local function declare(r)
	local n = r.n
	return {
		Echo = r.emit.askServer("Ask.Echo", n.string, n.string, nil),
		Short = r.emit.askServer("Ask.Short", n.string, n.string, 1),
		Maybe = r.emit.askServer("Ask.Maybe", n["nil"], n.union({ n["nil"], n.bool }), nil),
		Same = r.emit.askServer("Ask.Same", n.string, n.string, nil),
		Put = r.emit.toServer("Ask.Put", n.object({ { "items", n.array(n.string) } })),
		Box = r.emit.askServer("Ask.Box", n.object({ { "items", n.array(n.string) } }), n.string, nil),
		Note = r.emit.toClient("Ask.Note", n.object({ { "items", n.array(n.string) } })),
		Part = r.emit.askServer("Ask.Part", n.instance({ "Part" }), n.string, nil),
	}
end

local function client(c)
	c.m = declare(c)
	c.got = {}
	c.service("Listener", function(self)
		self.note = c.emit.receive(self, "note", c.m.Note, function(value)
			table.insert(c.got, table.concat(value.items, ","))
		end)
	end)
end

local function hex(n)
	return string.format("%02x", n)
end

-- A server whose Echo-family callbacks are scripted by `answer(name, player, value)`.
local function server(link, answer)
	local m
	link:boot(function(s)
		m = declare(s)
		s.service("Answers", function(self)
			for _, name in { "Echo", "Short", "Maybe", "Same", "Box", "Part" } do
				self[name] = s.emit.receive(self, name, m[name], function(player, value)
					return answer(name, player, value)
				end)
			end
			self.put = s.emit.receive(self, "put", m.Put, function(_, value)
				table.insert(link.puts, table.concat(value.items, ","))
			end)
		end)
	end)
	assert(link.server.booted, tostring(link.server.bootError))
	return m
end

return {
	["18 a payload is encoded at the call, so mutating it afterwards sends nothing new"] = function()
		local link = Link.new()
		link.puts = {}
		local boxed
		local m = server(link, function(name, _, value)
			if name == "Box" then
				boxed = table.concat(value.items, ",")
			end
			return "ok"
		end)
		local ann = link:join("Ann", client)
		local payload = { items = { "a", "b" } }
		ann.Network.send(ann.m.Put, payload)
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Box, payload)
		end)
		link.server.Network.send(ann.player, m.Note, payload)
		table.insert(payload.items, "mutated")
		payload.items[1] = "z"
		link:flush()
		spec.list(link.puts, { "a,b" }, "client send")
		spec.eq(boxed, "a,b", "client ask")
		spec.list(ann.got, { "a,b" }, "server send")
		spec.eq(asked.value.ok, true, "settled")
		link:assertClean()
	end,
	["38 39 overlapping asks settle by nonce and undefined and false are values"] = function()
		local link = Link.new()
		local held, maybes = {}, 0
		server(link, function(name, _, value)
			if name == "Maybe" then
				maybes += 1
				return if maybes == 1 then nil else false
			end
			held[value] = coroutine.running()
			return `echo {coroutine.yield()}`
		end)
		local ann = link:join("Ann", client)
		local a = link:run(function()
			return ann.Network.ask(ann.m.Echo, "a")
		end)
		local b = link:run(function()
			return ann.Network.ask(ann.m.Echo, "b")
		end)
		link:flush()
		link:resume(held.b, "B")
		link:flush()
		spec.eq(b.value.value, "echo B", "second ask settled first")
		spec.eq(a.done, nil, "first ask still pending")
		link:resume(held.a, "A")
		link:flush()
		spec.eq(a.value.value, "echo A", "matched by nonce")
		local none = link:run(function()
			return ann.Network.ask(ann.m.Maybe)
		end)
		link:flush()
		spec.eq(none.value.ok, true, "undefined is a success")
		spec.eq(none.value.value, nil, "with no value")
		local no = link:run(function()
			return ann.Network.ask(ann.m.Maybe)
		end)
		link:flush()
		spec.eq(no.value.ok, true, "false is a success")
		spec.eq(no.value.value, false, "false")
		local nonces = {}
		for _, entry in
			link:sent(function(e)
				return e.from == "Ann" and e.opcode == 1
			end)
		do
			table.insert(nonces, buffer.readu8(entry.packet, 2))
		end
		spec.list(nonces, { 1, 2, 3, 4 }, "one nonce per call, from 1")
		link:assertClean()
	end,
	["44 a reply racing the deadline settles once and the loser is ignored"] = function()
		local link = Link.new()
		local held
		server(link, function()
			held = coroutine.running()
			coroutine.yield()
			return "late"
		end)
		local ann = link:join("Ann", client)
		local resumes = 0
		local timed = link:run(function()
			local result = ann.Network.ask(ann.m.Short, "x")
			resumes += 1
			return result
		end)
		link:advance(1)
		spec.eq(timed.value.reason, "timeout", "the declared one-second deadline")
		link:resume(held)
		link:flush()
		spec.eq(resumes, 1, "resumed once")
		spec.eq(ann.logged("Crucible ignored a late or unknown reply to Ask.Short"), 1, "late reply ignored")
		local fast = link:run(function()
			return ann.Network.ask(ann.m.Short, "y")
		end)
		link:flush()
		link:resume(held)
		link:flush()
		link:advance(5)
		spec.eq(fast.value.ok, true, "the reply won")
		spec.eq(fast.value.value, "late", "value")
		spec.eq(link:pending(), 0, "the deadline was cancelled")
		link:assertClean()
	end,
	["5 37 an ask with no declared timeout settles timeout at DEFAULT_ASK_TIMEOUT_SECONDS"] = function()
		local link = Link.new()
		server(link, function()
			coroutine.yield()
		end)
		local ann = link:join("Ann", client)
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Echo, "x")
		end)
		link:advance(9.9)
		spec.eq(asked.done, nil, "pending before 10 seconds")
		link:advance(0.1)
		spec.eq(asked.value.reason, "timeout", "timeout at 10 seconds")
		link:assertClean()
	end,
	["44 a cancelled caller's reply is dropped and its entry counts toward MAX_PENDING_ASKS until then"] = function()
		local link = Link.new()
		local held = {}
		server(link, function(_, _, value)
			held[value] = coroutine.running()
			coroutine.yield()
			return value
		end)
		local ann = link:join("Ann", client)
		local callers = {}
		for i = 1, 64 do
			callers[i] = link:run(function()
				return ann.Network.ask(ann.m.Echo, tostring(i))
			end)
		end
		link:flush()
		coroutine.close(callers[1].thread)
		local over = link:run(function()
			return ann.Network.ask(ann.m.Echo, "65")
		end)
		spec.eq(over.value.reason, "overloaded", "64 pending, the cancelled one included")
		link:resume(held["1"])
		link:flush()
		spec.eq(callers[1].done, nil, "the cancelled caller never resumes")
		local next = link:run(function()
			return ann.Network.ask(ann.m.Echo, "66")
		end)
		link:flush()
		spec.eq(next.done, nil, "admitted once the reply freed the entry")
		link:resume(held["66"])
		link:flush()
		spec.eq(next.value.value, "66", "settled")
		link:assertClean()
	end,
	["44 an error in the caller after ask returns is the engine's and never fails the session"] = function()
		local link = Link.new()
		server(link, function(_, _, value)
			return value
		end)
		local ann = link:join("Ann", client)
		link:run(function()
			ann.Network.ask(ann.m.Echo, "x")
			error("caller bug")
		end)
		link:flush()
		spec.eq(#link.errors, 1, "reported by the engine")
		spec.has(link.errors[1], "caller bug", "the caller's error")
		table.clear(link.errors)
		spec.eq(#link:sent(function(e)
			return e.opcode == 5
		end), 0, "no FAIL")
		local again = link:run(function()
			return ann.Network.ask(ann.m.Echo, "y")
		end)
		link:flush()
		spec.eq(again.value.value, "y", "the session continues")
		link:assertClean()
	end,
	["41 74 the nonce mark advances at admission and replays drop without a reply"] = function()
		local link = Link.new()
		local calls = {}
		local m = server(link, function(_, _, value)
			table.insert(calls, value)
			return value
		end)
		local ann = link:join("Ann", nil, { boot = false })
		link:inject(ann, h.bytes("04"), {})
		link:flush()
		local ix = hex(m.Echo.index)
		local px = hex(m.Part.index)
		local function request(nonce, text)
			link:inject(
				ann,
				h.bytes(`01 {ix} {hex(nonce)} 00 {hex(#text)} {(text:gsub(".", function(c)
					return hex(string.byte(c)) .. " "
				end))}`),
				{}
			)
			link:flush()
		end
		local function replies()
			local out = {}
			for _, e in
				link:sent(function(entry)
					return entry.to == "Ann"
				end)
			do
				table.insert(out, h.hex(e.packet))
			end
			return table.concat(out, " | ")
		end
		request(5, "a")
		request(5, "b")
		request(3, "c")
		spec.list(calls, { "a" }, "only the first nonce 5 and nothing below it")
		spec.eq(
			link.server.logged("Crucible dropped Ask.Echo with a nonce already used from Ann"),
			1,
			"once per message"
		)
		link:inject(ann, h.bytes(`01 {ix} 06 00 01 ff`), {})
		link:flush()
		request(6, "d")
		spec.list(calls, { "a" }, "a request that failed to decode consumed nonce 6")
		link:inject(ann, h.bytes(`01 {px} 07 01`), { ["1"] = 5 })
		link:flush()
		request(7, "e")
		spec.list(calls, { "a", "e" }, "a bad sidecar is not admitted and leaves the mark")
		spec.eq(
			replies(),
			`02 {ix} 05 00 01 61 | 03 {ix} 06 03 | 02 {ix} 07 00 01 65`,
			"OK, ERROR invalid for the undecodable request, OK; no reply to a refused one"
		)
		spec.eq(#link.kicks, 0, "no kick for any of it")
		link:assertClean()
	end,
	["40 37 an ask that times out in the pre-ready queue leaves it and nonces reach the wire with a gap"] = function()
		local link = Link.new()
		local m = server(link, function(_, _, value)
			return value
		end)
		local ann = link:join("Ann", nil, { boot = false })
		local session = ann.graph.module("internal/network/client-session").ClientSession.new(ann.port)
		local box = {}
		link:run(function()
			session:open()
		end)
		link:flush()
		session:loaded()
		local mm = declare(ann)
		local short = link:run(function()
			return session:ask(mm.Short, "dropped")
		end)
		link:advance(0.5)
		local long = link:run(function()
			return session:ask(mm.Echo, "kept")
		end)
		link:advance(0.6)
		spec.eq(short.value.reason, "timeout", "the deadline counts the wait for readiness")
		box.committed = pcall(function()
			session:commit()
		end)
		link:flush()
		assert(box.committed, "commit")
		local sent = link:sent(function(e)
			return e.from == "Ann"
		end)
		spec.eq(#sent, 2, "READY and one REQUEST")
		spec.eq(h.hex(sent[2].packet), `01 {hex(m.Echo.index)} 02 00 04 6b 65 70 74`, "nonce 2 after the removed 1")
		spec.eq(long.value.value, "kept", "the kept ask settles")
		link:assertClean()
	end,
	["38 14 asks fail locally as unavailable overloaded or disconnected without sending"] = function()
		local link = Link.new()
		link:boot(function(s)
			declare(s)
		end)
		local ann = link:join("Ann", function(c)
			client(c)
			c.m.Missing = c.emit.askServer("Ask.ClientOnly", c.n.string, c.n.string, nil)
		end)
		local before = #link:sent()
		local missing = link:run(function()
			return ann.Network.ask(ann.m.Missing, "x")
		end)
		spec.eq(missing.value.reason, "unavailable", "unpublished")
		spec.eq(#link:sent(), before, "nothing sent")
		link:run(function()
			ann.Network.ask(ann.m.Missing, "x")
		end)
		spec.eq(ann.logged("Crucible Ask.ClientOnly is not in the server's network catalog"), 1, "diagnosed once")
		local norecv = link:run(function()
			return ann.Network.ask(ann.m.Echo, "x")
		end)
		link:flush()
		spec.eq(norecv.value.reason, "unavailable", "no live server receiver")
		local ok, err = pcall(ann.Network.ask, ann.m.Echo, 5)
		spec.eq(ok, false, "an invalid request throws")
		spec.eq(err, "Ask.Echo request: expected string", "naming message, path and reason")
		link:injectToClient(ann, h.bytes("05 02"), {})
		link:flush()
		spec.eq(
			ann.logged("Crucible network session ended: the server sent FAIL (closing)"),
			1,
			"a server FAIL ends it"
		)
		local after = link:run(function()
			return ann.Network.ask(ann.m.Echo, "x")
		end)
		spec.eq(after.value.reason, "disconnected", "after the session ended")
		spec.eq(ann.Network.send(ann.m.Put, { items = {} }), false, "send returns false")
		link:assertClean()
	end,
	["43 77 a reply naming another message or an unknown ERROR reason fails the client session"] = function()
		local link = Link.new()
		local m = server(link, function()
			coroutine.yield()
			return "never"
		end)
		local ann = link:join("Ann", client)
		local pending = link:run(function()
			return ann.Network.ask(ann.m.Echo, "x")
		end)
		link:flush()
		link:injectToClient(ann, h.bytes(`03 {hex(m.Same.index)} 01 00`), {})
		link:flush()
		spec.eq(ann.logged("a reply to Ask.Echo naming Ask.Same"), 1, "index mismatch is a protocol violation")
		spec.eq(pending.value.reason, "disconnected", "the pending ask settles with the session")
		spec.eq(link.kicks[1].message, "Connection problem, please rejoin.", "the server kicks on the FAIL")
		local other = Link.new()
		server(other, function()
			coroutine.yield()
		end)
		local bea = other:join("Bea", client)
		other:run(function()
			bea.Network.ask(bea.m.Echo, "x")
		end)
		other:flush()
		other:injectToClient(bea, h.bytes(`03 {hex(bea.m.Echo.index)} 01 09`), {})
		other:flush()
		spec.eq(bea.logged("an unknown ERROR reason 9"), 1, "unknown reason")
		spec.eq(#other.kicks, 1, "kicked")
		link:assertClean()
		other:assertClean()
	end,
}
