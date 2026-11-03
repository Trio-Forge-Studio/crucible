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
						s.api.untracked(function()
							w.event:set(event)
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
}
