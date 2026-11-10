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

-- Ann's client, after it received her loaded profile.
local function ownerLoaded()
	local w = World.new({ declare = declare })
	local ann = w:player("Ann", 4120)
	w:settle()
	w:settle()
	local client = ann.d.PlayerData
	assert(client:loaded(), "Ann's client holds her profile")
	return w, ann, client, w.server.d.PlayerData
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

return {
	["9 125 one player's write never invalidates another player's handles or view"] = function()
		local w, ann, bea, data = twoLoaded()
		local api = w.server.api
		local runs = 0
		local beaCoins = w.server.charm.computed(function()
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

	["121 an untracked read of a handle leaves nothing subscribed to the player's slot"] = function()
		local w = World.new({ declare = declare })
		local ann = w:player("Ann", 4120)
		w:settle()
		local data, charm = w.server.d.PlayerData, w.server.charm
		data(ann.player).upgrades.speed:set(3)
		data(ann.player).questProgress:set({ 1, 2, 3 })
		local slot
		charm.effect(function()
			data(ann.player).currency()
			slot = charm.getActiveSub().deps.dep.deps.dep
		end)()
		local function subscribers()
			local total = 0
			local link = slot.subs
			while link do
				total += 1
				link = link.nextSub
			end
			return total
		end
		local baseline = subscribers()
		local held = data(ann.player).upgrades.speed
		for _ = 1, 10 do
			spec.eq(held(), 3, "a held entry reads its value")
		end
		for i = 1, 50 do
			spec.eq(data(ann.player).upgrades[`key{i}`](), nil, "a missing key reads nil")
			spec.eq(data(ann.player).questProgress[i % 3](), i % 3 + 1, "an element reads its value")
		end
		spec.eq(subscribers() - baseline, 0, "untracked reads subscribe nothing to the slot")
		local seen
		local stop = charm.effect(function()
			seen = held()
		end)
		spec.eq(seen, 3, "a tracked read sees the value")
		spec.eq(subscribers() - baseline, 1, "a tracked read subscribes the handle's computed")
		held:set(4)
		spec.eq(seen, 4, "and reruns on a write")
		stop()
		spec.eq(subscribers() - baseline, 0, "its consumer stopping releases the computed")
		spec.eq(held(), 4, "an untracked read after the consumer stopped is current")
		data(ann.player).upgrades.speed:set(5)
		spec.eq(held(), 5, "and stays current with no consumer")
		spec.eq(subscribers() - baseline, 0, "and subscribes nothing")
		w:finish()
	end,

	["124 an unchanged client root is one table, read tracked or not, until a write replaces it"] = function()
		local w, ann, client, data = ownerLoaded()
		local first = client()()
		assert(rawequal(client()(), first), "two untracked reads of an unchanged root are one table")
		local tracked
		local runs = 0
		local stop = ann.charm.effect(function()
			runs += 1
			tracked = client()()
		end)
		assert(rawequal(tracked, first), "a tracked read returns the untracked read's table")
		assert(rawequal(client()(), first), "and so does an untracked read after it")
		data(ann.player).currency:set(42)
		w:settle()
		spec.eq(runs, 2, "the write reran the tracked read once")
		assert(not rawequal(tracked, first), "the write made a new table")
		spec.eq(tracked.currency, 42, "with the new value")
		assert(table.isfrozen(tracked), "frozen")
		spec.eq(first.currency, 0, "the old table keeps the old value")
		assert(rawequal(client()(), tracked), "an untracked read returns the tracked read's new table")
		stop()
		assert(rawequal(client()(), tracked), "and keeps it after the consumer stopped")
		data(ann.player).xp:set(7)
		w:settle()
		local after = client()()
		assert(not rawequal(after, tracked), "a write with no consumer makes a new table too")
		spec.eq(after.xp, 7, "with the new value")
		assert(rawequal(after.currency, tracked.currency), "keeping the fields that did not change")
		w:finish()
	end,

	["121 124 untracked client reads leave at most one reader on a field view, however many run"] = function()
		local w, ann, client, data = ownerLoaded()
		local charm = ann.charm
		data(ann.player).upgrades.speed:set(3)
		w:settle()
		local view
		for _, declaration in ann.graph.module("internal/sync/declarations").declarations() do
			if declaration.name == fixture.view("upgrades") then
				view = declaration
			end
		end
		local node
		charm.effect(function()
			view.node()
			node = charm.getActiveSub().deps.dep
		end)()
		local function subscribers()
			local total = 0
			local link = node.subs
			while link do
				total += 1
				link = link.nextSub
			end
			return total
		end
		local baseline = subscribers()
		local function readAll()
			for i = 1, 50 do
				client()()
				spec.eq(client().upgrades[`key{i}`](), nil, "a missing key reads nil")
				spec.eq(client().upgrades.speed(), 3, "an entry reads its value")
			end
		end
		readAll()
		local added = subscribers() - baseline
		assert(added <= 1, `at most the store's one root reader reads the view, got {added}`)
		readAll()
		spec.eq(subscribers() - baseline, added, "more reads add none")
		local stop = charm.effect(function()
			client()()
			client().upgrades.speed()
		end)
		stop()
		readAll()
		spec.eq(subscribers() - baseline, added, "nor do reads after a tracked consumer stopped")
		w:finish()
	end,

	["124 a client root snapshot nothing holds is not kept past a write"] = function()
		local w, ann, client, data = ownerLoaded()
		local weak = setmetatable({}, { __mode = "v" })
		-- Read in a separate frame, so no register of this one still holds the snapshot.
		local function read()
			weak.root = client()()
		end
		read()
		data(ann.player).upgrades.speed:set(4)
		w:settle()
		assert(
			collect(function()
				return weak.root == nil
			end),
			"the snapshot from before the write is collected"
		)
		spec.eq(client()().upgrades.speed, 4, "and a read after it is current")
		w:finish()
	end,

	["124 a watch on a loaded client root waits for a write and fires once for it"] = function()
		local w, ann, client, data = ownerLoaded()
		local calls = {}
		local dispose = ann.api.scope(function()
			ann.api.watch(function()
				return client()()
			end, function(value, previous)
				table.insert(calls, { value = value, previous = previous })
			end)
		end)
		spec.eq(#calls, 0, "registering the watch calls nothing")
		data(ann.player).currency:set(42)
		w:settle()
		spec.eq(#calls, 1, "a write calls it once")
		spec.eq(calls[1].value.currency, 42, "with the new root")
		spec.eq(calls[1].previous.currency, 0, "and the root before the write")
		w:heartbeat()
		spec.eq(#calls, 1, "and nothing more without a write")
		dispose()
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
