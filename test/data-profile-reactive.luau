local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("./lib/load")

local function declare(r)
	return { PlayerData = fixture.declare(r) }
end

local function twoLoaded()
	local w = World.new({ declare = declare })
	local ann = w:player("Ann", 4120)
	local bea = w:player("Bea", 4121)
	w:settle()
	w:settle()
	return w, ann, bea, w.server.d.PlayerData
end

return {
	["9 125 one player's write never invalidates another player's handles or view"] = function()
		local w, ann, bea, data = twoLoaded()
		local api = w.server.api
		local runs = 0
		local beaCoins = api.computed(function()
			runs += 1
			return data(bea.player).currency()
		end)
		api.scope(function()
			api.effect(function()
				beaCoins()
			end)
		end)
		w:settle()
		local before = runs
		local beaFrames = #w:frames(bea)
		data(ann.player).currency:set(50)
		data(ann.player):update(function(d)
			local copy = table.clone(d)
			copy.xp = 9
			return copy
		end)
		w:settle()
		w:heartbeat()
		spec.eq(runs, before, "Bea's computed did not rerun for Ann's writes")
		spec.eq(#w:frames(bea), beaFrames, "Bea received no frame for Ann's writes")
		spec.eq(ann.d.PlayerData().currency(), 50, "Ann's client got her write")
		w:finish()
	end,

	["9 43 a departed player's slot is dropped once their release settles"] = function()
		local w, ann, _, data = twoLoaded()
		local root = data(ann.player)
		local held = root.currency
		held:set(31)
		spec.eq(held(), 31, "live")
		w:leave(ann)
		w:advance(0.01)
		spec.eq(held(), 0, "the leave sealed at once, so reads return the template")
		assert(rawequal(data(ann.player), root), "the slot and its root live until the release settles")
		w:settle(2)
		spec.eq(w:envelope(fixture.STORE, 4120).lease, nil, "released")
		spec.eq(held(), 0, "a held handle reads the template once the slot is dropped")
		spec.eq(held:set(5), false, "and writes nothing")
		local after = data(ann.player)
		assert(not rawequal(after, root), "the departed player gets an inert root")
		assert(not rawequal(after, data(ann.player)), "which is not cached")
		spec.eq(data:loaded(ann.player), false, "loaded is false")
		w:finish()
	end,

	["10 players() changes when a session becomes or stops being writable, never on a data write"] = function()
		local w = World.new({ declare = declare })
		local data = w.server.d.PlayerData
		local api = w.server.api
		local seen = {}
		api.scope(function()
			api.effect(function()
				local names = {}
				for _, player in data:players() do
					table.insert(names, player.Name)
				end
				table.sort(names)
				table.insert(seen, table.concat(names, ","))
			end)
		end)
		w:settle()
		local ann = w:player("Ann", 4120)
		w:settle()
		data(ann.player).currency:set(3)
		w:settle()
		local box = w:run(function()
			return data:handoff(ann.player)
		end)
		w:settle(2)
		spec.eq(box.value.ok, true, "handoff")
		spec.list(seen, { "", "Ann", "" }, "players() moved only on the load and the seal")
		w:finish()
	end,

	["9 loaded(player) is a tracked read"] = function()
		local w = World.new({ declare = declare })
		local data = w.server.d.PlayerData
		local api = w.server.api
		local ann = w:player("Ann", 4120)
		local seen = {}
		api.scope(function()
			api.effect(function()
				table.insert(seen, tostring(data:loaded(ann.player)))
			end)
		end)
		w:settle()
		data(ann.player).currency:set(3)
		w:settle()
		w:run(function()
			return data:handoff(ann.player)
		end)
		w:settle(2)
		spec.list(seen, { "false", "true", "false" }, "loaded moved on the load and the seal only")
		w:finish()
	end,
}
