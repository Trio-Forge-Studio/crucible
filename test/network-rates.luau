local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("../packages/crucible/lune")

local function declare(r)
	local n = r.n
	return {
		Tap = r.emit.toServer("Rate.Tap", n["nil"], 2),
		Slow = r.emit.toServer("Rate.Slow", n["nil"], 0.5),
		Free = r.emit.toServer("Rate.Free", n["nil"], nil),
		Buy = r.emit.askServer("Rate.Buy", n.string, n.string, nil, 1),
		Gone = r.emit.askServer("Rate.Gone", n.string, n.string, nil, 1),
		Spin = r.emit.toServer("Rate.Spin", n["nil"], 5, 1),
		Hold = r.emit.toServer("Rate.Hold", n["nil"], 1, 3),
		Boost = r.emit.askServer("Rate.Boost", n.string, n.string, nil, 5, 1),
		Wait = r.emit.toServer("Rate.Wait", n["nil"], 1),
	}
end

local function collect(done: () -> boolean): boolean
	for _ = 1, 400 do
		local junk = {}
		for i = 1, 4000 do
			junk[i] = { i }
		end
		if done() then
			return true
		end
	end
	return done()
end

local function client(c)
	c.m = declare(c)
end

local function server(link)
	link.calls = {}
	local function count(key)
		link.calls[key] = (link.calls[key] or 0) + 1
	end
	local m
	link:boot(function(s)
		m = declare(s)
		s.service("Rated", function(self)
			for _, name in { "Tap", "Slow", "Free", "Spin", "Hold" } do
				self[name] = s.emit.receive(self, name, m[name], function(player)
					count(`{name} {player.Name}`)
				end)
			end
			for _, name in { "Buy", "Boost" } do
				self[name] = s.emit.receive(self, name, m[name], function(player, item)
					count(`{name} {player.Name}`)
					return `sold {item}`
				end)
			end
			self.wait = s.emit.receive(self, "wait", m.Wait, function()
				link.held = coroutine.running()
				coroutine.yield()
			end)
		end)
	end)
	assert(link.server.booted, tostring(link.server.bootError))
	return m
end

local function sends(c, message, times)
	for _ = 1, times do
		spec.eq(c.Network.send(message), true, "the client accepts every send")
	end
end

return {
	["a rated event runs its callback only within the rate and counts each drop"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		sends(ann, ann.m.Tap, 5)
		link:flush()
		spec.eq(link.calls["Tap Ann"], 2, "a bucket holds one second of requests")
		spec.eq(link.server.Network.limited(m.Tap), 3, "each drop counted")
		spec.eq(
			link.server.logged("Crucible dropped Rate.Tap from Ann: over its rate of 2 per second"),
			1,
			"reported once"
		)
		link:advance(0.5)
		sends(ann, ann.m.Tap, 2)
		link:flush()
		spec.eq(link.calls["Tap Ann"], 3, "half a second refills one request")
		spec.eq(link.server.Network.limited(m.Tap), 4, "the second waits")
		link:advance(30)
		sends(ann, ann.m.Tap, 3)
		link:flush()
		spec.eq(link.calls["Tap Ann"], 5, "a long pause refills no more than the bucket holds")
		spec.eq(link.server.Network.limited(m.Tap), 5, "the third is dropped")
		spec.eq(
			link.server.logged("Crucible dropped Rate.Tap from Ann: over its rate of 2 per second"),
			1,
			"still reported once"
		)
		spec.eq(#link.kicks, 0, "a rate never kicks")
		link:assertClean()
	end,
	["a rate below one holds one request and refills over its period"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		sends(ann, ann.m.Slow, 2)
		link:flush()
		spec.eq(link.calls["Slow Ann"], 1, "one request at once")
		link:advance(1)
		sends(ann, ann.m.Slow, 1)
		link:flush()
		spec.eq(link.calls["Slow Ann"], 1, "half the period refills half a request")
		link:advance(2)
		sends(ann, ann.m.Slow, 1)
		link:flush()
		spec.eq(link.calls["Slow Ann"], 2, "a full period refills one")
		spec.eq(link.server.Network.limited(m.Slow), 2, "both drops counted")
		link:assertClean()
	end,
	["a message without a rate is never limited"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		sends(ann, ann.m.Free, 50)
		link:flush()
		spec.eq(link.calls["Free Ann"], 50, "every request runs")
		spec.eq(link.server.Network.limited(m.Free), 0, "none counted")
		link:assertClean()
	end,
	["each player has its own allowance and a new peer starts full"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		sends(ann, ann.m.Tap, 3)
		sends(bea, bea.m.Tap, 2)
		link:flush()
		spec.eq(link.calls["Tap Ann"], 2, "Ann spends her own")
		spec.eq(link.calls["Tap Bea"], 2, "Bea keeps hers")
		spec.eq(link.server.Network.limited(m.Tap), 1, "counted across players")
		link:leave(ann)
		link.present[ann.player] = ann
		link.gone[ann.player] = nil
		link.server.added(ann.player)
		link:inject(ann, h.bytes("04"), {})
		for _ = 1, 3 do
			link:inject(ann, h.bytes(string.format("00 %02x 00", m.Tap.index)), {})
		end
		link:flush()
		spec.eq(link.calls["Tap Ann"], 4, "the player's next peer starts with a full bucket")
		spec.eq(link.server.Network.limited(m.Tap), 2, "and spends it like the first")
		link:assertClean()
	end,
	["an ask over its rate answers limited without running its callback"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		local first = link:run(function()
			return ann.Network.ask(ann.m.Buy, "hat")
		end)
		local second = link:run(function()
			return ann.Network.ask(ann.m.Buy, "hat")
		end)
		link:flush()
		spec.eq(first.value.ok, true, "within the rate")
		spec.eq(first.value.value, "sold hat", "answered by the callback")
		spec.eq(second.value.ok, false, "over the rate")
		spec.eq(second.value.reason, "limited", "the client sees limited")
		spec.eq(link.calls["Buy Ann"], 1, "the callback ran once")
		spec.eq(link.server.Network.limited(m.Buy), 1, "counted")
		local none = link:run(function()
			return ann.Network.ask(ann.m.Gone, "hat")
		end)
		link:flush()
		spec.eq(none.value.reason, "unavailable", "no receiver is still unavailable")
		local spent = link:run(function()
			return ann.Network.ask(ann.m.Gone, "hat")
		end)
		link:flush()
		spec.eq(spent.value.reason, "limited", "the rate is checked before the receiver")
		link:advance(1)
		local later = link:run(function()
			return ann.Network.ask(ann.m.Buy, "cap")
		end)
		link:flush()
		spec.eq(later.value.value, "sold cap", "a second later the ask runs")
		link:assertClean()
	end,
	["a reused nonce is dropped before it spends a token"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		local first = link:run(function()
			return ann.Network.ask(ann.m.Buy, "hat")
		end)
		link:flush()
		spec.eq(first.value.ok, true, "the first ask runs")
		link:advance(1)
		link:inject(ann, h.bytes(string.format("01 %02x 01 00 03 68 61 74", m.Buy.index)), {})
		link:flush()
		spec.eq(link.server.Network.limited(m.Buy), 0, "a replay spends nothing")
		local next = link:run(function()
			return ann.Network.ask(ann.m.Buy, "cap")
		end)
		link:flush()
		spec.eq(next.value.value, "sold cap", "the bucket is still full")
		link:assertClean()
	end,
	["a malformed event spends the allowance and counts when it is over"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		for _ = 1, 2 do
			link:inject(ann, h.bytes(string.format("00 %02x 01", m.Tap.index)), { ["1"] = 5 })
		end
		link:inject(ann, h.bytes(string.format("00 %02x 00 ff", m.Tap.index)), {})
		link:flush()
		spec.eq(
			link.server.Network.limited(m.Tap),
			1,
			"two bad sidecars spent the allowance and trailing bytes are over it"
		)
		spec.eq(
			link.server.logged("Crucible dropped Rate.Tap from Ann: invalid sidecar slot"),
			1,
			"a bad body is still reported"
		)
		local oversized = buffer.create(300000)
		buffer.writeu8(oversized, 1, m.Tap.index)
		link:inject(ann, oversized, {})
		link:flush()
		spec.eq(
			link.server.logged("Crucible dropped a malformed network packet from Ann"),
			1,
			"an oversized packet is dropped before its message is read"
		)
		spec.eq(link.server.Network.limited(m.Tap), 1, "so it names no message to count against")
		sends(ann, ann.m.Tap, 1)
		link:flush()
		spec.eq(link.calls["Tap Ann"], nil, "the valid request finds the allowance spent")
		spec.eq(link.server.Network.limited(m.Tap), 2, "and is counted")
		link:advance(0.5)
		sends(ann, ann.m.Tap, 1)
		link:flush()
		spec.eq(link.calls["Tap Ann"], 1, "the refill admits it")
		spec.eq(#link.kicks, 0, "a malformed body never kicks")
		link:assertClean()
	end,
	["a malformed ask spends the allowance and answers limited when it is over"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		local ix = string.format("%02x", m.Buy.index)
		local function replies()
			local out = {}
			for _, entry in
				link:sent(function(sent)
					return sent.to == "Ann" and sent.opcode ~= 5
				end)
			do
				table.insert(out, h.hex(entry.packet))
			end
			return table.concat(out, " | ")
		end
		link:inject(ann, h.bytes(`01 {ix} 01 01`), { ["1"] = 5 })
		link:flush()
		spec.eq(link.server.Network.limited(m.Buy), 0, "a bad sidecar spent the one request")
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Buy, "hat")
		end)
		link:flush()
		spec.eq(asked.value.reason, "limited", "the valid ask finds the allowance spent")
		spec.eq(link.calls["Buy Ann"], nil, "the callback never ran")
		spec.eq(link.server.Network.limited(m.Buy), 1, "and is counted")
		link:advance(1)
		link:inject(ann, h.bytes(`01 {ix} 02 00 01 ff`), {})
		link:inject(ann, h.bytes(`01 {ix} 03 01`), { ["1"] = 5 })
		link:flush()
		spec.eq(link.server.Network.limited(m.Buy), 2, "a bad sidecar over the rate is counted")
		spec.eq(
			replies(),
			`03 {ix} 01 04 | 03 {ix} 02 03 | 03 {ix} 03 04`,
			"ERROR limited, ERROR invalid for the undecodable ask that spent the refill, ERROR limited"
		)
		link:advance(1)
		link:inject(ann, h.bytes(`01 {ix} 04 00 03 63 61 70`), {})
		link:flush()
		spec.eq(link.calls["Buy Ann"], 1, "the refill admits a valid ask")
		spec.eq(#link.kicks, 0, "a malformed body never kicks")
		link:assertClean()
	end,
	["a burst of one spaces requests by the rate"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		sends(ann, ann.m.Spin, 2)
		link:flush()
		spec.eq(link.calls["Spin Ann"], 1, "a same-frame duplicate is refused")
		link:advance(0.1)
		sends(ann, ann.m.Spin, 1)
		link:flush()
		spec.eq(link.calls["Spin Ann"], 1, "0.1 seconds later is too soon")
		link:advance(0.1)
		sends(ann, ann.m.Spin, 1)
		link:flush()
		spec.eq(link.calls["Spin Ann"], 2, "0.2 seconds after the first, one more runs")
		link:advance(10)
		sends(ann, ann.m.Spin, 2)
		link:flush()
		spec.eq(link.calls["Spin Ann"], 3, "a long pause still holds one")
		spec.eq(link.server.Network.limited(m.Spin), 3, "every refusal counted")
		link:assertClean()
	end,
	["a burst of one refuses a same-frame second ask"] = function()
		local link = Link.new()
		server(link)
		local ann = link:join("Ann", client)
		local first = link:run(function()
			return ann.Network.ask(ann.m.Boost, "gem")
		end)
		local second = link:run(function()
			return ann.Network.ask(ann.m.Boost, "gem")
		end)
		link:flush()
		spec.eq(first.value.value, "sold gem", "the first boost runs")
		spec.eq(second.value.reason, "limited", "a same-frame second boost is refused")
		link:advance(0.2)
		local third = link:run(function()
			return ann.Network.ask(ann.m.Boost, "gem")
		end)
		link:flush()
		spec.eq(third.value.value, "sold gem", "0.2 seconds later it runs")
		spec.eq(link.calls["Boost Ann"], 2, "two boosts ran")
		link:assertClean()
	end,
	["a burst above the rate holds that many requests"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		sends(ann, ann.m.Hold, 4)
		link:flush()
		spec.eq(link.calls["Hold Ann"], 3, "a burst of three at a rate of one")
		link:advance(1)
		sends(ann, ann.m.Hold, 2)
		link:flush()
		spec.eq(link.calls["Hold Ann"], 4, "one second refills one")
		link:advance(60)
		sends(ann, ann.m.Hold, 4)
		link:flush()
		spec.eq(link.calls["Hold Ann"], 7, "a long pause refills the burst and no more")
		spec.eq(link.server.Network.limited(m.Hold), 3, "every refusal counted")
		link:assertClean()
	end,
	["limited replies free the client's pending asks"] = function()
		local link = Link.new()
		local m = server(link)
		local ann = link:join("Ann", client)
		local function burst(item)
			local results = {}
			for i = 1, 64 do
				results[i] = link:run(function()
					return ann.Network.ask(ann.m.Buy, item)
				end)
			end
			link:flush()
			local reasons = {}
			for _, result in results do
				local key = if result.value.ok then "ok" else result.value.reason
				reasons[key] = (reasons[key] or 0) + 1
			end
			return reasons
		end
		local first = burst("hat")
		spec.eq(first.ok, 1, "one ask runs")
		spec.eq(first.limited, 63, "the rest are limited")
		for round = 1, 3 do
			local again = burst("hat")
			spec.eq(again.limited, 64, `round {round}: every slot was freed, so none is overloaded`)
		end
		spec.eq(link.server.Network.limited(m.Buy), 63 + 3 * 64, "each refusal counted")
		link:advance(1)
		local later = link:run(function()
			return ann.Network.ask(ann.m.Buy, "cap")
		end)
		link:flush()
		spec.eq(later.value.value, "sold cap", "an ask after the refill runs")
		link:assertClean()
	end,
	["a peer that ends frees its allowances though a handler still holds it"] = function()
		local link = Link.new()
		local Session = link.server.graph.module("internal/network/server-session").ServerSession
		local open = Session.open
		local session
		Session.open = function(self, ...)
			session = self
			return open(self, ...)
		end
		local m = server(link)
		local ann = link:join("Ann", client)
		sends(ann, ann.m.Wait, 1)
		sends(ann, ann.m.Tap, 1)
		link:flush()
		spec.eq(coroutine.status(link.held), "suspended", "the handler yields")
		local weak = setmetatable({}, { __mode = "v" })
		local function probe()
			for definition, bucket in session.peers[ann.player].buckets do
				weak[definition.name] = bucket
			end
		end
		probe()
		spec.eq(weak["Rate.Tap"] ~= nil and weak["Rate.Wait"] ~= nil, true, "one allowance for each rated message")
		link:leave(ann)
		spec.eq(
			collect(function()
				return weak["Rate.Tap"] == nil and weak["Rate.Wait"] == nil
			end),
			true,
			"the allowances are collected while the handler holds its peer"
		)
		spec.eq(coroutine.status(link.held), "suspended", "the handler still yields")
		spec.eq(link.server.Network.limited(m.Tap), 0, "nothing was limited")
	end,
	["a rate is checked when the message is declared"] = function()
		local link = Link.new()
		local s = link.server
		local n = s.n
		for i, rate in { 0, -1, 1001, 0 / 0, "2" } do
			local ok, err = pcall(s.emit.toServer, `Rate.Bad{i}`, n["nil"], rate)
			spec.eq(ok, false, `toServer refuses {rate}`)
			spec.has(tostring(err), "invalid message rate", `toServer names the rate for {rate}`)
		end
		local ok, err = pcall(s.emit.askServer, "Rate.BadAsk", n.string, n.string, nil, 1001)
		spec.eq(ok, false, "askServer refuses 1001")
		spec.has(tostring(err), "invalid message rate", "askServer names the rate")
		spec.eq(pcall(s.emit.toServer, "Rate.Most", n["nil"], 1000), true, "1000 is the most")
		for i, burst in { 0, -1, 1.5, 1001, 0 / 0, math.huge, "2" } do
			local refused, reason = pcall(s.emit.toServer, `Rate.BadBurst{i}`, n["nil"], 5, burst)
			spec.eq(refused, false, `toServer refuses a burst of {burst}`)
			spec.has(tostring(reason), "invalid message burst", `toServer names the burst for {burst}`)
		end
		local alone, aloneErr = pcall(s.emit.toServer, "Rate.Alone", n["nil"], nil, 1)
		spec.eq(alone, false, "a burst needs a rate")
		spec.has(tostring(aloneErr), "invalid message burst", "and names the burst")
		local ask, askErr = pcall(s.emit.askServer, "Rate.BadBurstAsk", n.string, n.string, nil, 5, 0)
		spec.eq(ask, false, "askServer refuses a burst of 0")
		spec.has(tostring(askErr), "invalid message burst", "askServer names the burst")
		spec.eq(pcall(s.emit.toServer, "Rate.Burst1", n["nil"], 5, 1), true, "1 is the least burst")
		spec.eq(pcall(s.emit.toServer, "Rate.Burst1000", n["nil"], 0.5, 1000), true, "1000 is the most burst")
	end,
}
