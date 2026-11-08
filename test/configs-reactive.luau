local ConfigBackend = require("./lib/config-backend")
local World = require("./lib/sync-world")
local spec = require("./lib/load")

-- Amendment A7: configs.md cases 2 and 4 in their effect form, and the client and publish halves of cases 13 and 15,
-- over reactive and networking's fake link.
local NO_EVENT = { name = "", startsAt = 0, endsAt = 0, multipliers = { coins = 1, xp = 1 } }
local DOUBLE = {
	name = "Double Coins Weekend",
	startsAt = 1793462400,
	endsAt = 1793635200,
	multipliers = { coins = 2, xp = 1.5 },
}

local function eventNode(n)
	return n.object({
		{ "endsAt", n.f64 },
		{ "multipliers", n.object({ { "coins", n.f64 }, { "xp", n.f64 } }) },
		{ "name", n.string },
		{ "startsAt", n.f64 },
	})
end

local function declare(r)
	local d = { CurrentEvent = r.emit.replicated("Events.Current", NO_EVENT, eventNode(r.n)) }
	if r.side == "client" then
		r.seen = {}
		r.service("Banner", nil, {
			awake = function()
				r.api.effect(function()
					table.insert(r.seen, `{d.CurrentEvent().name}|{r.api.syncStatus(d.CurrentEvent)}`)
				end)
			end,
		})
	end
	if r.side == "server" then
		d.GlobalEvent = r.emit.config("GlobalEvent", NO_EVENT, eventNode(r.n))
		d.Coins = r.emit.config("Shop.Coins", 0, r.n.f64)
		d.Gems = r.emit.config("Shop.Gems", 0, r.n.f64)
	end
	return d
end

-- A server whose EventService republishes the global event in an effect, and whose ShopService records every run of
-- an effect over two keys.
local function newWorld(configs)
	return World.new({
		declare = declare,
		configs = function()
			return configs.port
		end,
		serve = function(s, d, w)
			w.runs = {}
			w.published = 0
			s.service("EventService", function()
				w.event = s.entry.publish(d.CurrentEvent)
			end, {
				awake = function()
					s.api.effect(function()
						local event = d.GlobalEvent()
						s.charm.untracked(function()
							w.event(event)
							w.published += 1
						end)
					end)
					s.api.effect(function()
						table.insert(w.runs, `{d.Coins()}/{d.Gems()}`)
					end)
				end,
			})
		end,
	})
end

local function settle(w, configs)
	configs:advance(0)
	w.link:flush()
	configs:advance(1)
	w.link:flush()
	w:heartbeat()
end

local function refresh(w, configs)
	local box = w.link:run(function()
		return w.server.entry.refreshConfigs()
	end)
	settle(w, configs)
	return box
end

-- Appends each value `read` returns that differs from the last one seen, so a list is what a reader observed and not
-- how often its effect ran.
local function track(effect, read)
	local seen = {}
	effect(function()
		local value = tostring(read())
		if seen[#seen] ~= value then
			table.insert(seen, value)
		end
	end)
	return seen
end

-- A server with a targeted Shop.Disc and a global Shop.G. With `target`, a service's awake creates effects over
-- `Disc(target)` and `configReady(target)`, which run before ConfigRuntime starts and joins any player.
local function playerWorld(configs, target)
	return World.new({
		declare = function(r)
			local d = {}
			if r.side == "server" then
				d.Disc = r.emit.playerConfig("Shop.Disc", 0, r.n.f64)
				d.Global = r.emit.config("Shop.G", 0, r.n.f64)
			end
			return d
		end,
		configs = function()
			return configs.port
		end,
		serve = function(s, d, w)
			w.s, w.d = s, d
			if target == nil then
				return
			end
			s.service("Shop", function() end, {
				awake = function()
					w.values = track(s.api.effect, function()
						return d.Disc(target)
					end)
					w.ready = track(s.api.effect, function()
						return s.entry.configReady(target)
					end)
				end,
			})
		end,
	})
end

local function step(w, configs, seconds)
	configs:advance(seconds)
	w.link:flush()
	w:heartbeat()
end

local function watch(w, read)
	local seen
	w.s.api.scope(function()
		seen = track(w.s.api.effect, read)
	end)
	w:heartbeat()
	return seen
end

return {
	["A7 configs 2 a first fetch applies every key in one pass: an effect over two keys runs once with both"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.Coins", 10)
		configs:setGlobal("Shop.Gems", 20)
		local w = newWorld(configs)
		settle(w, configs)
		spec.list(w.runs, { "0/0", "10/20" }, "the fallbacks, then one run with both fetched keys")
		w:finish()
	end,

	["A7 configs 4 a refresh changing two keys runs a dependent effect once"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.Coins", 10)
		configs:setGlobal("Shop.Gems", 20)
		local w = newWorld(configs)
		settle(w, configs)
		configs:setGlobal("Shop.Coins", 11)
		configs:setGlobal("Shop.Gems", 21)
		settle(w, configs)
		spec.list(w.runs, { "0/0", "10/20" }, "UpdateAvailable alone changes nothing")
		local box = refresh(w, configs)
		spec.eq(box.value, true, "the refresh is current")
		spec.list(w.runs, { "0/0", "10/20", "11/21" }, "one run with both changed keys")
		w:finish()
	end,

	["A7 configs 13 a client sees NO_EVENT and pending, then the event and ready, published once per changing refresh"] = function()
		local configs = ConfigBackend.new()
		local w = newWorld(configs)
		local client = w:join("Ann")
		local current = client.d.CurrentEvent
		settle(w, configs)
		spec.eq(client.seen[1], "|pending", "the client read NO_EVENT and pending before the first frame")
		spec.eq(client.api.syncStatus(current), "ready", "ready after the snapshot")
		spec.eq(current().name, "", "still NO_EVENT: no event is live")
		local before = w.published
		configs:setGlobal("GlobalEvent", DOUBLE)
		refresh(w, configs)
		spec.eq(current().name, DOUBLE.name, "the client reads the event")
		spec.eq(current().multipliers.coins, 2, "with its multipliers")
		spec.eq(w.published, before + 1, "published once for the changing refresh")
		refresh(w, configs)
		spec.eq(w.published, before + 1, "a refresh that changes nothing publishes nothing")
		w:finish()
	end,

	["A7 configs 15 an undeclared field is kept in the admitted config and never sent when published"] = function()
		local configs = ConfigBackend.new()
		local withSecret = table.clone(DOUBLE)
		withSecret.secret = "UNDECLARED-CONFIG-FIELD"
		configs:setGlobal("GlobalEvent", withSecret)
		local w = newWorld(configs)
		local client = w:join("Ann")
		settle(w, configs)
		spec.eq(
			w.server.d.GlobalEvent().secret,
			"UNDECLARED-CONFIG-FIELD",
			"the server's admitted value keeps the field"
		)
		spec.eq(client.d.CurrentEvent().name, DOUBLE.name, "the client got the event")
		spec.eq(client.d.CurrentEvent().secret, nil, "without the undeclared field")
		for _, decoded in w:frames(client) do
			local value = World.value(decoded, "Events.Current")
			if type(value) == "table" then
				spec.eq(value.secret, nil, "no decoded frame carries the undeclared field")
			end
		end
		w:finish()
	end,

	["93 96 an effect created before a player joins follows their config and readiness through join, snapshot and leave"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.Disc", 5)
		configs:setForPlayer(7, "Shop.Disc", 9)
		configs:delay(7, 3)
		local target = { UserId = 7, Name = "Player7" }
		local w = playerWorld(configs, target)
		step(w, configs, 0)
		step(w, configs, 1)
		spec.list(w.values, { "0" }, "a player who has not joined reads the fallback")
		spec.list(w.ready, { "false" }, "and is not ready")
		table.insert(configs.players, target)
		configs.joined.fire(target)
		step(w, configs, 1)
		spec.list(w.values, { "0", "5" }, "once joined, the global value until their snapshot lands")
		spec.list(w.ready, { "false" }, "still not ready")
		step(w, configs, 5)
		spec.list(w.values, { "0", "5", "9" }, "their targeted value once their snapshot is admitted")
		spec.list(w.ready, { "false", "true" }, "ready once their snapshot is admitted")
		configs:leave(target)
		w.link:flush()
		w:heartbeat()
		spec.list(w.values, { "0", "5", "9", "0" }, "a departed player reads the fallback")
		spec.list(w.ready, { "false", "true", "false" }, "and is not ready")
		w:finish()
	end,

	["93 95 96 an effect created before boot joins a present player follows the fallback, the global value, then theirs"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.Disc", 5)
		configs:setForPlayer(7, "Shop.Disc", 9)
		configs:delay("global", 2)
		configs:delay(7, 4)
		local target = { UserId = 7, Name = "Player7" }
		table.insert(configs.players, target)
		local w = playerWorld(configs, target)
		step(w, configs, 0)
		step(w, configs, 1)
		spec.list(w.values, { "0" }, "the fallback before either snapshot lands")
		step(w, configs, 2)
		spec.list(w.values, { "0", "5" }, "the global value once the global snapshot lands")
		spec.list(w.ready, { "false" }, "not ready before their own snapshot")
		step(w, configs, 5)
		spec.list(w.values, { "0", "5", "9" }, "their targeted value once their snapshot lands")
		spec.list(w.ready, { "false", "true" }, "ready once their snapshot lands")
		w:finish()
	end,

	["93 96 an effect created after a player joins follows the global, targeted, refreshed and fallback values"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.Disc", 5)
		configs:setForPlayer(7, "Shop.Disc", 9)
		configs:delay(7, 3)
		local w = playerWorld(configs)
		settle(w, configs)
		local player = configs:join(7)
		configs:advance(0)
		w.link:flush()
		local values = watch(w, function()
			return w.d.Disc(player)
		end)
		local ready = watch(w, function()
			return w.s.entry.configReady(player)
		end)
		step(w, configs, 5)
		spec.list(values, { "5", "9" }, "the global value, then the targeted one")
		spec.list(ready, { "false", "true" }, "readiness follows")
		configs:setForPlayer(7, "Shop.Disc", 12)
		configs:delay(7, 0)
		local box = w.link:run(function()
			return w.s.entry.refreshConfigs()
		end)
		step(w, configs, 0)
		step(w, configs, 2)
		spec.eq(box.value, true, "the refresh is current")
		spec.list(values, { "5", "9", "12" }, "a refresh reruns the player's reader")
		configs:leave(player)
		w.link:flush()
		w:heartbeat()
		spec.list(values, { "5", "9", "12", "0" }, "a departed player reads the fallback")
		spec.list(ready, { "false", "true", "false" }, "and is not ready")
		w:finish()
	end,

	["93 96 configReady() and a global config are tracked from before the global snapshot lands"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.G", 3)
		configs:delay("global", 2)
		local w = playerWorld(configs)
		local ready = watch(w, function()
			return w.s.entry.configReady()
		end)
		local value = watch(w, function()
			return w.d.Global()
		end)
		step(w, configs, 0)
		step(w, configs, 5)
		spec.list(ready, { "false", "true" }, "global readiness is tracked")
		spec.list(value, { "0", "3" }, "the global value is tracked")
		w:finish()
	end,

	["93 96 an effect created before the first join follows leave and rejoin of the same player"] = function()
		local configs = ConfigBackend.new()
		configs:setGlobal("Shop.Disc", 5)
		configs:setForPlayer(7, "Shop.Disc", 9)
		configs:delay(7, 3)
		local target = { UserId = 7, Name = "Player7" }
		local w = playerWorld(configs)
		step(w, configs, 0)
		step(w, configs, 1)
		local values = watch(w, function()
			return w.d.Disc(target)
		end)
		local ready = watch(w, function()
			return w.s.entry.configReady(target)
		end)
		table.insert(configs.players, target)
		configs.joined.fire(target)
		step(w, configs, 5)
		configs:leave(target)
		step(w, configs, 1)
		table.insert(configs.players, target)
		configs.joined.fire(target)
		step(w, configs, 1)
		spec.list(
			values,
			{ "0", "5", "9", "0", "5" },
			"each join reruns the reader at once, with the global value until the snapshot"
		)
		step(w, configs, 5)
		spec.list(values, { "0", "5", "9", "0", "5", "9" }, "then the targeted value again")
		spec.list(ready, { "false", "true", "false", "true" }, "readiness follows the rejoin")
		w:finish()
	end,
}
