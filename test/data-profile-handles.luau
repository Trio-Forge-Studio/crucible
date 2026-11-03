local DM = require("./lib/data-migrations")
local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("./lib/load")

local function declare(r)
	return { PlayerData = fixture.declare(r) }
end

local function newWorld(options)
	options = options or {}
	return World.new({ declare = options.declare or declare, serve = options.serve, backend = options.backend })
end

-- A world with Ann (4120) loaded.
local function loaded(options)
	local w = newWorld(options)
	local ann = w:player("Ann", 4120)
	w:settle()
	assert(w.server.d.PlayerData:loaded(ann.player), "Ann's profile loaded")
	return w, ann, ann.player, w.server.d.PlayerData
end

local function throws(fn, part: string, label: string)
	local ok, err = pcall(fn)
	assert(not ok, `{label}: expected a throw`)
	spec.has(err, part, label)
	return err
end

-- Yields `PlayerData:save(player)` on a server thread and returns its result.
local function save(w, data, player)
	local box = w:run(function()
		return data:save(player)
	end)
	w:advance(1)
	assert(box.done, "save returned")
	return box.value
end

local function stored(w)
	return w:envelope(fixture.STORE, 4120)
end

-- A record saved by a newer build at the same version: keys this build does not declare, at the root, in a nested
-- object, in an array element and in a map value.
local function seedUnknown(backend)
	backend:seed(
		fixture.STORE,
		"4120",
		DM.envelope({
			payload = {
				career = { jobs = { { coins = 3, tipJar = "kept" } } },
				currency = 7,
				mastery = { house1 = { level = 2, stars = 5 } },
				notes = "",
				purchaseLog = {},
				questProgress = {},
				settings = { rating = "PG", vibration = true, volume = 0.5 },
				title = "Mover",
				upgrades = {},
				analytics = { playTime = 0 },
				vault = { pin = 0 },
				xp = 0,
			},
		})
	)
end

-- Drives Lune's incremental collector by allocation until `done()` or a bound; Lune refuses a full collect.
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
	["12 117 a write before DataRuntime constructs or after a rolled-back boot returns false"] = function()
		local early = { UserId = 9, Name = "Early" }
		local seen = {}
		local w = World.new({
			declare = declare,
			serve = function(s, d, world)
				world.link.present[early] = {}
				seen.write = d.PlayerData(early).currency:set(5)
				seen.read = d.PlayerData(early).currency()
				s.service("Broken", function()
					error("constructor failed")
				end)
			end,
			mustBoot = false,
		})
		spec.eq(seen.write, false, "a write at module load, before boot, returns false")
		spec.eq(seen.read, 0, "and a read gives the template's value")
		assert(not w.booted, "boot rolled back")
		local data = w.server.d.PlayerData
		spec.eq(data(early).currency:set(5), false, "a write after the rollback returns false")
		spec.eq(data(early).currency(), 0, "and a read gives the template's value")
		spec.eq(data:loaded(early), false, "not loaded")
	end,

	["E1 116 117 nested reads and a nested set"] = function()
		local w, _, player, data = loaded()
		spec.eq(data(player).settings.volume(), 0.5, "a nested object field reads its default")
		spec.eq(data(player).settings.vibration:set(false), true, "a nested set is accepted")
		spec.eq(data(player).settings.vibration(), false, "the nested read follows")
		spec.eq(data(player).settings.volume(), 0.5, "a sibling keeps its value")
		w:finish()
	end,

	["E2 116 120 map keys by property and by variable and set(undefined) removing an entry"] = function()
		local w, _, player, data = loaded()
		local id = "speed"
		spec.eq(data(player).upgrades[id](), nil, "an absent entry reads undefined")
		spec.eq(data(player).upgrades[id]:set(3), true, "set creates the entry")
		spec.eq(data(player).upgrades.speed(), 3, "the same entry reached by property")
		spec.eq(
			data(player).upgrades.boost:update(function(v)
				return (v or 0) + 2
			end),
			true,
			"update from absent"
		)
		spec.eq(data(player).upgrades.boost(), 2, "update created the entry")
		spec.eq(data(player).upgrades.speed:set(nil), true, "set(undefined) on an entry")
		spec.eq(data(player).upgrades.speed(), nil, "the entry is gone")
		spec.eq(data(player).upgrades().speed, nil, "the map no longer holds the key")
		spec.eq(data(player).upgrades().boost, 2, "the other entry stays")
		w:finish()
	end,

	["E3 E4 12 an equal set is no revision and update stores its result"] = function()
		local w, _, player, data = loaded()
		local runs = 0
		spec.eq(data(player).currency:set(5), true, "first set")
		spec.eq(data(player).currency:set(5), true, "an equal set returns true")
		spec.eq(
			data(player).currency:update(function(c)
				runs += 1
				return c * 2
			end),
			true,
			"update"
		)
		spec.eq(runs, 1, "update calls its callback once")
		spec.eq(data(player).currency(), 10, "update stored its result")
		spec.eq(save(w, data, player).ok, true, "save confirms")
		spec.eq(stored(w).commit.revision, 2, "two revisions: the equal set made none")
		spec.eq(stored(w).payload.currency, 10, "the saved currency")
		w:finish()
	end,

	["E5 119 insert and remove with 0-based indices and out-of-range throws"] = function()
		local w, _, player, data = loaded()
		local quests = data(player).questProgress
		spec.eq(quests:insert(10), true, "append")
		spec.eq(quests:insert(5, 0), true, "insert at 0")
		spec.eq(quests:insert(7, 2), true, "insert at the length appends")
		spec.list(quests(), { 5, 10, 7 }, "elements in order")
		spec.eq(quests[0](), 5, "index 0 is the first element")
		spec.eq(quests[2](), 7, "index 2 is the third")
		spec.eq(quests[3](), nil, "past the end reads undefined")
		spec.eq(quests[1]:set(11), true, "an element's set")
		spec.eq(quests:remove(0), true, "remove at 0")
		spec.list(quests(), { 11, 7 }, "after remove")
		throws(function()
			quests:insert(1, 3)
		end, "FD.Players.questProgress[3]: the index is not an integer from 0 to 2", "insert past the length")
		throws(function()
			quests[2]:set(1)
		end, "FD.Players.questProgress[2]: the index is not an integer from 0 to 1", "set out of range")
		throws(function()
			quests:remove(2)
		end, "FD.Players.questProgress[2]: the index is not an integer from 0 to 1", "remove out of range")
		throws(function()
			quests[0.5]:set(1)
		end, "the index is not an integer", "a fractional index")
		spec.list(quests(), { 11, 7 }, "refused writes changed nothing")
		w:finish()
	end,

	["E6 121 cached identity and no handle per repeated read"] = function()
		local w, _, player, data = loaded()
		local root = data(player)
		assert(rawequal(root, data(player)), "the root is the same handle on every call")
		assert(rawequal(root.settings, data(player).settings), "an object field is the same handle")
		local id = "speed"
		assert(rawequal(root.upgrades[id], root.upgrades.speed), "a map entry reached two ways is one handle")
		local element = root.questProgress[0]
		assert(rawequal(element, data(player).questProgress[0]), "an array element is the same handle")
		local before = collectgarbage("count")
		for _ = 1, 100 do
			root.settings.volume()
		end
		assert(collectgarbage("count") - before < 64, "100 more reads of one path make no new handle")
		w:finish()
	end,

	["E7 12 a root update changes two fields in one revision and keeps the root's unknown key"] = function()
		local backend = require("./lib/data-backend").new()
		seedUnknown(backend)
		local w, _, player, data = loaded({ backend = backend })
		spec.eq(
			data(player):update(function(d)
				local copy = table.clone(d)
				copy.currency = d.currency - 2
				copy.xp = d.xp + 5
				return copy
			end),
			true,
			"one root update"
		)
		spec.eq(data(player).currency(), 5, "currency")
		spec.eq(data(player).xp(), 5, "xp")
		spec.eq(data(player)().title, "Mover", "the root read carries the unknown key")
		spec.eq(save(w, data, player).ok, true, "save")
		spec.eq(stored(w).commit.revision, 1, "both fields in one revision")
		spec.eq(stored(w).payload.title, "Mover", "the unknown root key is saved in place")
		w:finish()
	end,

	["E8 118 78 a path copy keeps unknown keys in a nested object and an array element"] = function()
		local backend = require("./lib/data-backend").new()
		seedUnknown(backend)
		local w, _, player, data = loaded({ backend = backend })
		spec.eq(data(player).settings.volume:set(0.9), true, "nested set")
		spec.eq(
			data(player).career.jobs[0].coins:update(function(c)
				return c + 1
			end),
			true,
			"an element field's update"
		)
		spec.eq(data(player).mastery.house1.level:set(3), true, "a map value's field")
		save(w, data, player)
		local payload = stored(w).payload
		spec.eq(payload.settings.rating, "PG", "the nested object's unknown key stays")
		spec.eq(payload.settings.volume, 0.9, "the nested write")
		spec.eq(payload.career.jobs[1].tipJar, "kept", "the element's unknown key stays")
		spec.eq(payload.career.jobs[1].coins, 4, "the element write")
		spec.eq(payload.mastery.house1.stars, 5, "the map value's unknown key stays")
		spec.eq(payload.title, "Mover", "the root's unknown key stays")
		w:finish()
	end,

	["E9 15 reads are frozen and mutating one throws"] = function()
		local w, _, player, data = loaded()
		data(player).upgrades.speed:set(1)
		local settings = data(player).settings()
		assert(table.isfrozen(settings), "a nested read is frozen")
		assert(table.isfrozen(data(player)()), "the root read is frozen")
		assert(table.isfrozen(data(player).upgrades()), "a map read is frozen")
		local ok = pcall(function()
			settings.volume = 1
		end)
		assert(not ok, "mutating a read throws")
		spec.eq(data(player).settings.volume(), 0.5, "the stored value is unchanged")
		w:finish()
	end,

	["E10 117 a write keeps untouched subtrees by identity"] = function()
		local w, _, player, data = loaded()
		local settings = data(player).settings()
		local career = data(player).career()
		data(player).currency:set(9)
		assert(rawequal(settings, data(player).settings()), "settings keeps its identity across a currency write")
		assert(rawequal(career, data(player).career()), "career too")
		data(player).settings.volume:set(0.1)
		assert(not rawequal(settings, data(player).settings()), "the written subtree is a new table")
		assert(rawequal(career, data(player).career()), "career still keeps its identity")
		w:finish()
	end,

	["E11 122 a handle is a callable table and the Vide rule wraps it"] = function()
		local w, _, player, data = loaded()
		local handle = data(player).currency
		spec.eq(type(handle), "table", "typeof(handle) is table")
		local function videRead(x)
			return if type(x) == "function" then x() else x
		end
		assert(rawequal(videRead(handle), handle), "a function test treats the handle as a static value")
		spec.eq(
			videRead(function()
				return handle()
			end),
			0,
			"() => h() reads through"
		)
		w:finish()
	end,

	["E12 116 a map key named after a member has no handle and the map's update writes it"] = function()
		local w, _, player, data = loaded()
		local upgrades = data(player).upgrades
		throws(function()
			upgrades["set"]()
		end, 'a map key named "set" has no handle', "using the method as a handle")
		throws(function()
			local _ = upgrades["update"].level
		end, 'a map key named "update" has no handle', "indexing the method")
		spec.eq(
			upgrades:update(function(m)
				local copy = table.clone(m)
				copy.set = 4
				return copy
			end),
			true,
			"the map's update writes the key"
		)
		spec.eq(upgrades().set, 4, "the key is stored and read through the map")
		w:finish()
	end,

	["E13 117 12 reads before load return the template and writes return false calling nothing"] = function()
		local w = newWorld()
		local ann = w:player("Ann", 4120)
		local data = w.server.d.PlayerData
		local player = ann.player
		spec.eq(data:loaded(player), false, "loading")
		spec.eq(data(player).currency(), 0, "currency reads the template")
		spec.eq(data(player).settings.volume(), 0.5, "a nested default")
		spec.eq(data(player).upgrades.speed(), nil, "an entry inside a collection reads undefined")
		local called = false
		spec.eq(data(player).currency:set(5), false, "set returns false")
		spec.eq(
			data(player).currency:update(function(c)
				called = true
				return c
			end),
			false,
			"update returns false"
		)
		spec.eq(called, false, "the callback is not called")
		local root = data(player)
		w:settle()
		spec.eq(data:loaded(player), true, "loaded")
		assert(rawequal(root, data(player)), "the handle made before the load is the loaded one")
		spec.eq(root.currency(), 0, "and reads the live value")
		w:finish()
	end,

	["E14 118 a write under an absent entry throws and setting the entry whole works"] = function()
		local w, _, player, data = loaded()
		throws(
			function()
				data(player).mastery.house1.level:set(2)
			end,
			"FD.Players.mastery.house1.level: its parent does not exist; set the parent whole",
			"below an absent entry"
		)
		throws(function()
			data(player).career.jobs[0].coins:set(2)
		end, "FD.Players.career.jobs[0].coins: its parent does not exist", "below an absent element")
		spec.eq(data(player).mastery.house1:set({ level = 2 }), true, "the entry set whole")
		spec.eq(data(player).mastery.house1.level(), 2, "then its field reads")
		spec.eq(data(player).mastery.house1.level:set(3), true, "and writes")
		w:finish()
	end,

	["E15 13 123 a handle write inside an update callback throws the busy message and changes nothing"] = function()
		local w, _, player, data = loaded()
		throws(function()
			data(player):update(function(d)
				data(player).currency:set(1)
				return d
			end)
		end, "FD.Players: 4120's profile is inside a data callback; return the data instead", "the nested write")
		spec.eq(data(player).currency(), 0, "unchanged")
		w:finish()
	end,

	["E16 117 watch fires only for its own path"] = function()
		local w, _, player, data = loaded()
		local seen = {}
		w.server.api.scope(function()
			w.server.api.watch(data(player).currency, function(now, before)
				table.insert(seen, `{before}->{now}`)
			end)
		end)
		data(player).settings.volume:set(0.2)
		w:settle()
		data(player).currency:set(3)
		w:settle()
		data(player).upgrades.speed:set(1)
		w:settle()
		spec.list(seen, { "0->3" }, "one firing, for the currency write")
		w:finish()
	end,

	["E19 121 assigning a field on a handle throws"] = function()
		local w, _, player, data = loaded()
		throws(function()
			data(player).currency.path = 1
		end, "a handle has no fields to assign", "assignment")
		throws(function()
			data(player).path = 1
		end, "a handle has no fields to assign", "assignment on the root")
		w:finish()
	end,

	["E20 121 identity under garbage collection"] = function()
		local w, ann, player, data = loaded()
		data(player).mastery.house1:set({ level = 1 })
		local leaf = data(player).mastery.house1.level
		local weak = setmetatable({}, { __mode = "v" })
		weak.entry = data(player).mastery.house1
		weak.free = data(player).mastery.house2
		weak.field = data(player).settings
		assert(
			collect(function()
				return weak.free == nil
			end),
			"an entry handle nothing holds is collected"
		)
		assert(weak.entry ~= nil, "a held leaf keeps its weakly cached ancestor")
		assert(rawequal(data(player).mastery.house1.level, leaf), "the held leaf is found again")
		assert(weak.field ~= nil, "an object-field child is kept by its parent")
		assert(rawequal(weak.field, data(player).settings), "and stays canonical")
		local tree = setmetatable({}, { __mode = "v" })
		tree.root = data(player)
		tree.child = data(player).settings
		tree.entry = data(player).mastery.house1
		tree.leaf = leaf
		leaf = nil :: any
		weak.entry = nil
		weak.field = nil
		w:leave(ann)
		w:settle(2)
		assert(
			collect(function()
				return tree.root == nil and tree.child == nil and tree.entry == nil and tree.leaf == nil
			end),
			"a dropped slot's root is collected with its whole tree, parent links included"
		)
		w:finish()
	end,

	["121 an object-field child nothing else holds outlives an entry child the same collection frees"] = function()
		local w, _, player, data = loaded()
		-- Reached in a separate frame, so no register of this one still holds either handle.
		local function reach()
			local weak = setmetatable({}, { __mode = "v" })
			weak.field = data(player).settings
			weak.entry = data(player).mastery.house9
			return weak
		end
		local weak = reach()
		assert(
			collect(function()
				return weak.entry == nil
			end),
			"an entry handle nothing holds is collected"
		)
		assert(weak.field ~= nil, "an object-field child is kept by its parent")
		assert(rawequal(weak.field, data(player).settings), "and stays canonical")
		w:finish()
	end,

	["E23 123 a read of the player's own handles inside onLoad throws and fails the load"] = function()
		local seen = {}
		local w = newWorld({
			serve = function(s, d)
				s.service("Stamp", function(self)
					self.claim = d.PlayerData:onLoad(function(player, data, reason)
						table.insert(seen, reason)
						local _ = d.PlayerData(player).currency()
						return data
					end)
				end)
			end,
		})
		local ann = w:player("Ann", 4120)
		w:settle()
		spec.list(seen, { "join" }, "the claim ran once")
		spec.eq(w.server.d.PlayerData:loaded(ann.player), false, "the load failed")
		spec.eq(w:kicked(ann.player), "Your data could not load. Please rejoin.", "kicked with LOAD_FAILED_MESSAGE")
		assert(w:logged("onLoad claim of Stamp failed") >= 1, "the diagnostic names the claim's owner")
		spec.eq(stored(w).lease, nil, "the acquisition was abandoned")
		spec.eq(stored(w).payload, nil, "nothing was saved")
		w:finish()
	end,

	["E23 123 a read inside onRelease throws, the claim is skipped and its input kept"] = function()
		local w, ann, player, data = loaded({
			serve = function(s, d)
				s.service("Leaving", function(self)
					self.claim = d.PlayerData:onRelease(function(who, profile)
						local copy = table.clone(profile)
						copy.xp = d.PlayerData(who).xp() + 100
						return copy
					end)
				end)
				s.service("Count", function(self)
					self.claim = d.PlayerData:onRelease(function(_, profile)
						local copy = table.clone(profile)
						copy.currency = profile.currency + 1
						return copy
					end)
				end)
			end,
		})
		data(player).xp:set(4)
		w:leave(ann)
		w:settle(2)
		local payload = stored(w).payload
		spec.eq(payload.xp, 4, "the reading claim was skipped, keeping its input")
		spec.eq(payload.currency, 1, "the other claim still ran")
		assert(w:logged("onRelease claim of Leaving was skipped") >= 1, "the skip is diagnosed with its owner")
		spec.eq(stored(w).lease, nil, "released")
		w:finish()
	end,

	["E23 123 a read from a thread the claim spawned throws and another player's handles read normally"] = function()
		local results = {}
		local w = newWorld({
			serve = function(s, d)
				s.service("Spawner", function(self)
					self.claim = d.PlayerData:onLoad(function(player, data)
						if player.Name == "Bea" then
							results.other = d.PlayerData(results.ann).currency()
							World.spawn(function()
								results.spawned = pcall(function()
									return d.PlayerData(player).currency()
								end)
							end)
						end
						return data
					end)
				end)
			end,
		})
		local ann = w:player("Ann", 4120)
		w:settle()
		results.ann = ann.player
		w.server.d.PlayerData(ann.player).currency:set(42)
		local bea = w:player("Bea", 4121)
		w:settle()
		spec.eq(results.other, 42, "another player's handle reads that player's value inside the claim")
		spec.eq(results.spawned, false, "a read from a spawned thread throws")
		spec.eq(w.server.d.PlayerData:loaded(bea.player), true, "Bea's load still succeeded")
		w:finish()
	end,

	["123 a handle read inside a gate callback returns the committed value before this write"] = function()
		local w, _, player, data = loaded()
		data(player).currency:set(5)
		local inside
		data(player):update(function(d)
			inside = data(player).currency()
			local copy = table.clone(d)
			copy.currency = 6
			return copy
		end)
		spec.eq(inside, 5, "the read inside the callback is the committed value")
		spec.eq(data(player).currency(), 6, "after the write")
		w:finish()
	end,

	["123 a handle write to another player inside a callback and from a thread it spawned throws"] = function()
		local w = newWorld()
		local ann = w:player("Ann", 4120)
		local bea = w:player("Bea", 4121)
		w:settle()
		local data = w.server.d.PlayerData
		throws(function()
			data(ann.player):update(function(d)
				data(bea.player).currency:set(9)
				return d
			end)
		end, "a write inside a data callback; return the data instead", "another player's write on the callback thread")
		local spawned
		data(ann.player):update(function(d)
			World.spawn(function()
				spawned = { pcall(function()
					data(bea.player).currency:set(9)
				end) }
			end)
			local copy = table.clone(d)
			copy.currency = 1
			return copy
		end)
		spec.eq(spawned[1], false, "a spawned thread's write to another player throws")
		spec.has(spawned[2], "a write inside a data callback", "with the callback message")
		spec.eq(data(bea.player).currency(), 0, "Bea is unchanged")
		spec.eq(data(ann.player).currency(), 1, "the callback's own result stands")
		w:finish()
	end,

	["123 RX 7 a handle write inside a computed throws"] = function()
		local w, _, player, data = loaded()
		local api = w.server.api
		local c = api.computed(function()
			data(player).currency:set(1)
			return 1
		end)
		throws(function()
			c()
		end, "computed must not write state", "the computed's write")
		spec.eq(data(player).currency(), 0, "unchanged")
		local live = w:run(function()
			return data:inspect(4120)
		end)
		w:settle()
		spec.eq(live.value.value.currency, 0, "the session committed nothing either")
		w:finish()
	end,

	["115 the root is cached across a resume, inert for an absent player, and per player on the server"] = function()
		local w, ann, player, data = loaded()
		local root = data(player)
		local box = w:run(function()
			return data:handoff(player)
		end)
		w:advance(2)
		spec.eq(box.value.ok, true, "handoff confirmed")
		spec.eq(data(player).currency(), 0, "a sealed session reads the template")
		local resumed = w:run(function()
			return data:resume(player)
		end)
		w:settle(2)
		spec.eq(resumed.value, true, "resumed")
		assert(rawequal(root, data(player)), "the resume keeps the root handle")
		local stranger = { Name = "Ghost", UserId = 9 }
		local inert = data(stranger)
		assert(not rawequal(inert, data(stranger)), "an absent player's root is not cached")
		spec.eq(inert.currency(), 0, "it reads the template")
		spec.eq(inert.currency:set(4), false, "and writes nothing")
		throws(function()
			data()
		end, "FD.Players is per player on the server; call FD.Players(player)", "no argument on the server")
		w:leave(ann)
		w:finish()
	end,

	["116 an optional field and object clear with set(undefined) and the clear survives a reload"] = function()
		local backend = require("./lib/data-backend").new()
		local w, ann, player, data = loaded({ backend = backend })
		spec.eq(data(player).nickname:set("Ace"), true, "set an optional field")
		spec.eq(data(player).nickname(), "Ace", "read it")
		spec.eq(data(player).nickname:set(nil), true, "clear it")
		spec.eq(data(player).nickname(), nil, "cleared")
		data(player).currency:set(3)
		w:leave(ann)
		w:settle(2)
		spec.eq(stored(w).payload.nickname, nil, "the clear is saved")
		local w2 = newWorld({ backend = backend })
		local again = w2:player("Ann", 4120)
		w2:settle()
		spec.eq(w2.server.d.PlayerData(again.player).nickname(), nil, "the clear survives a reload")
		spec.eq(w2.server.d.PlayerData(again.player).currency(), 3, "with the rest of the data")
		w:finish()
	end,
}
