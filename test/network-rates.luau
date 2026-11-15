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
	}
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
			for _, name in { "Tap", "Slow", "Free" } do
				self[name] = s.emit.receive(self, name, m[name], function(player)
					count(`{name} {player.Name}`)
				end)
			end
			self.buy = s.emit.receive(self, "buy", m.Buy, function(player, item)
				count(`Buy {player.Name}`)
				return `sold {item}`
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
	end,
}
