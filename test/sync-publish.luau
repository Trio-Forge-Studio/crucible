local roblox = require("@lune/roblox")
local World = require("./lib/sync-world")
local spec = require("./lib/load")

local CALLBACK = 8

local function rejects(f, part: string, label: string)
	local ok, err = pcall(f)
	spec.eq(ok, false, label)
	spec.has(err, part, label)
	return err
end

local function declare(r)
	local n = r.n
	local settings = n.object({ { "music", n.bool }, { "volume", n.f64 } })
	return {
		Owner = r.emit.replicatedPerPlayer("Data.Owner", nil, n.union({ n["nil"], n.object({ { "coins", n.f64 } }) })),
		Profile = r.emit.replicated(
			"Data.Profile",
			{ coins = 0, settings = { music = true, volume = 1 }, tags = {} },
			n.object({ { "coins", n.f64 }, { "settings", settings }, { "tags", n.array(n.string) } })
		),
		Score = r.emit.replicated("Job.Score", 0, n.f64),
		Small = r.emit.replicated("Job.Small", 0, n.u8),
		Spare = r.emit.replicated("Job.Spare", 0, n.f64),
		Title = r.emit.replicated("Job.Title", "", n.string),
		Where = r.emit.replicated("Job.Where", roblox.CFrame.new(), n.cframe),
	}
end

local function publisher(s, d, w)
	w.owned = {}
	s.service("JobService", function(self)
		w.profile = s.entry.publish(d.Profile)
		w.score = s.entry.publish(d.Score)
		w.small = s.entry.publish(d.Small)
		w.title = s.entry.publish(d.Title)
		w.where = s.entry.publish(d.Where)
		w.owner = s.entry.publish(d.Owner, function(player)
			return w.owned[player.Name]
		end)
		w.service = self
	end)
end

local function newWorld()
	return World.new({ declare = declare, serve = publisher })
end

-- A fresh module graph holding only the declaration registry, for counting specs.
local function registry()
	local g = spec.runtime()
	return g.module("internal/sync/declarations"), g.module("internal/network/schema").n
end

return {
	["40 an untransformed declaration throws at evaluation naming the transformer plugin"] = function()
		local api = World.new({ declare = declare }).server.api
		rejects(function()
			api.replicated("Job.Phase", nil)
		end, "Crucible replicated state needs the @trioforge/crucible/transformer plugin", "replicated")
		rejects(function()
			api.replicatedPerPlayer("Data.Owner", nil)
		end, "Crucible replicated state needs the @trioforge/crucible/transformer plugin", "replicatedPerPlayer")
	end,
	["41 the runtime re-checks each name, its duplicates and the realm's count, naming the name"] = function()
		local declarations, n = registry()
		for _, bad in { "", "1A", ".A", "A..B", "A.", "Crucible.State", string.rep("a", 65), "A-B" } do
			rejects(function()
				declarations.declareReplicated("all", bad, 0, n.f64)
			end, `Crucible replicated name {bad} is invalid`, `{bad} is refused`)
		end
		declarations.declareReplicated("all", string.rep("a", 64), 0, n.f64)
		declarations.declareReplicated("all", "Job.Phase", 0, n.f64)
		rejects(function()
			declarations.declareReplicated("player", "Job.Phase", 0, n.f64)
		end, "Crucible replicated name Job.Phase is declared twice", "a duplicate")
		for i = 3, 256 do
			declarations.declareReplicated("all", `Fill.N{i}`, 0, n.f64)
		end
		rejects(function()
			declarations.declareReplicated("all", "Fill.Over", 0, n.f64)
		end, "Fill.Over would pass 256 replicated declarations in this realm", "the 257th")
	end,
	["41 R-D1 a framework declaration needs the Crucible segment and counts against the cap and the duplicate check"] = function()
		local declarations, n = registry()
		spec.eq(declarations.replicatedCapacity(), 256, "an empty realm admits 256")
		rejects(function()
			declarations.declareFrameworkReplicated("FD.Players", nil, n["nil"])
		end, "a framework declaration starts with Crucible.", "a game name is refused")
		rejects(function()
			declarations.declareFrameworkReplicated(string.rep("C", 65), nil, n["nil"])
		end, "is invalid", "the grammar still applies")
		declarations.declareReplicated("all", "Job.Phase", 0, n.f64)
		local live = declarations.declareFrameworkReplicated("Crucible.Live.FD.Players", 0, n.f64)
		spec.eq(type(live), "function", "a declaration handle")
		spec.eq(declarations.declarationOf(live).audience, "player", "per player")
		spec.eq(declarations.replicatedCapacity(), 254, "both kinds count")
		rejects(function()
			declarations.declareFrameworkReplicated("Crucible.Live.FD.Players", 0, n.f64)
		end, "Crucible.Live.FD.Players is declared twice", "the duplicate check covers it")
		for i = 3, 256 do
			declarations.declareReplicated("all", `Fill.N{i}`, 0, n.f64)
		end
		spec.eq(declarations.replicatedCapacity(), 0, "the exact remaining room")
		rejects(function()
			declarations.declareFrameworkReplicated("Crucible.Data.FD.Players.coins", 0, n.f64)
		end, "would pass 256 replicated declarations", "the cap covers it")
	end,
	["41 a declaration after the server or client registry sealed throws, and the client seals at its commit"] = function()
		local w = newWorld()
		rejects(
			function()
				w.server.emit.replicated("Job.Late", 0, w.server.n.f64)
			end,
			"Job.Late was declared after the replicated registry sealed; declare replicated state in modules that boot loads",
			"server"
		)
		local during
		local ann = w:join("Ann", {
			declare = function(c)
				local d = declare(c)
				during = pcall(c.emit.replicated, "Job.Loading", 0, c.n.f64)
				return d
			end,
		})
		spec.eq(during, true, "a client module evaluated during load declares")
		rejects(function()
			ann.emit.replicated("Job.Late", 0, ann.n.f64)
		end, "Job.Late was declared after the replicated registry sealed", "client after its commit")
		w:finish()
	end,
	["43 45 reads and statuses on each side, and a server per-player read throws"] = function()
		local w = newWorld()
		local s = w.server
		spec.eq(s.api.syncStatus(s.d.Spare), "pending", "server: pending with no publication")
		spec.eq(s.api.syncStatus(s.d.Score), "ready", "server: ready while published")
		w.score:set(4)
		spec.eq(s.d.Score(), 4, "server: the authoritative value, including a write a moment ago")
		rejects(
			function()
				s.d.Owner()
			end,
			"Data.Owner is per player; on the server, read the sources its publisher projects from",
			"server per-player read"
		)
		rejects(function()
			s.api.syncStatus(s.d.Owner)
		end, "Data.Owner is per player", "server per-player status")
		local before
		local ann = w:join("Ann", {
			declare = function(c)
				local d = declare(c)
				before = d.Score()
				return d
			end,
		})
		spec.eq(before, 0, "client: initial before the snapshot")
		spec.eq(ann.d.Score(), 4, "client: the last frame applied")
		w.owned.Ann = { coins = 3 }
		w.score:set(5)
		local statuses = {}
		ann.api.scope(function()
			ann.api.effect(function()
				table.insert(statuses, ann.api.syncStatus(ann.d.Owner))
			end)
		end)
		w.link:flush()
		w:heartbeat()
		spec.eq(ann.d.Owner(), nil, "client per-player: a plain state projection does not rerun")
		spec.eq(type(ann.d.Score), "function", "a declaration is a plain function")
		w.server.life.fault(w.server.state(w.service), CALLBACK, "retired")
		w.link:flush()
		w:heartbeat()
		spec.list(statuses, { "ready", "failed" }, "syncStatus tracks inside an effect")
		spec.eq(s.api.syncStatus(s.d.Score), "failed", "server: failed after retirement")
		w:finish()
	end,
	["47 publish outside a server Subsystem's construction throws, and a helper the constructor calls is accepted"] = function()
		local placements = {}
		local w = World.new({
			declare = declare,
			serve = function(s, d, world)
				local function claimScore()
					return s.entry.publish(d.Score)
				end
				s.service("Helper", function()
					world.score = claimScore()
				end)
				s.service("Late", nil, {
					awake = function()
						placements.awake = select(2, pcall(s.entry.publish, d.Title))
					end,
				})
				local Door = s.behaviour("Door", function()
					placements.behaviour = select(2, pcall(s.entry.publish, d.Small))
				end)
				world.door = Door
			end,
		})
		w.score:set(2)
		spec.eq(w.server.d.Score(), 2, "a helper the constructor calls publishes")
		local message = "publish runs while a server Subsystem is constructed; make it a field initializer"
		spec.has(placements.awake, message, "awake")
		local later = select(2, pcall(w.server.entry.publish, w.server.d.Spare))
		spec.has(later, message, "a later call")
		local door = w.server.engine.instance("door", "Model")
		w.server.engine.addTag(door, "Door")
		w.link:flush()
		spec.has(placements.behaviour, message, "a behaviour")
		w:finish()
	end,
	["48 a second publisher fails boot naming both owners by class id"] = function()
		for _, case in { { "First", "Second" }, { "First", "First" } } do
			local w = World.new({
				declare = declare,
				mustBoot = false,
				serve = function(s, d)
					s.service(case[1], function()
						s.entry.publish(d.Score)
						if case[1] == case[2] then
							s.entry.publish(d.Score)
						end
					end)
					if case[1] ~= case[2] then
						s.service(case[2], function()
							s.entry.publish(d.Score)
						end)
					end
				end,
			})
			spec.eq(w.booted, false, "boot failed")
			spec.has(
				w.bootError,
				`Job.Score is published by {case[1]} and {case[2]}; one Subsystem publishes each value`,
				"naming both"
			)
		end
	end,
	["49 admission validates every declared position, naming the declaration and path but never the value"] = function()
		local w = newWorld()
		local cycle = {}
		cycle.settings = { music = true, volume = 1 }
		cycle.coins = 1
		cycle.tags = cycle
		for _, case in
			{
				{ w.score, 0 / 0, "Job.Score: expected finite number" },
				{ w.score, math.huge, "Job.Score: expected finite number" },
				{ w.score, "12", "Job.Score: expected number, got string" },
				{ w.small, 256, "Job.Small: u8 out of range" },
				{ w.small, 1.5, "Job.Small: u8 out of range" },
				{ w.title, "\255\254", "Job.Title: invalid UTF-8" },
				{ w.title, string.rep("y", 49153), "Job.Title: longer than SYNC_STRING_BYTES" },
				{ w.where, roblox.CFrame.new(0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 1), "Job.Where: invalid CFrame" },
				{
					w.profile,
					{ coins = 1, settings = { music = 1, volume = 1 }, tags = {} },
					"Data.Profile.settings.music: expected boolean, got number",
				},
				{ w.profile, { coins = 1, tags = {} }, "Data.Profile.settings: missing" },
				{
					w.profile,
					{ coins = 1, settings = { music = true, volume = 1 }, tags = { "a", nil, "c" } },
					"Data.Profile.tags: sparse array",
				},
				{
					w.profile,
					{ coins = 1, settings = { music = true, volume = 1 }, tags = { "a", 2 } },
					"Data.Profile.tags[2]: expected string, got number",
				},
				{
					w.profile,
					setmetatable({ coins = 1, settings = { music = true, volume = 1 }, tags = {} }, {}),
					"Data.Profile: metatable",
				},
				{ w.profile, cycle, "Data.Profile.tags: cycle" },
			}
		do
			local err = rejects(function()
				case[1]:set(case[2])
			end, case[3], case[3])
			if type(case[2]) == "string" then
				spec.eq(string.find(err, case[2], 1, true), nil, "the value is never in the message")
			end
		end
		spec.eq(w.server.d.Score(), 0, "a rejected write stores nothing")
		w:finish()
	end,
	["49 K13 admission keeps conforming tables by identity, copies only what had undeclared keys, and freezes every path"] = function()
		local w = newWorld()
		local s = w.server
		spec.eq(table.isfrozen(s.d.Profile()), true, "the declared initial is frozen")
		local settings = { music = false, volume = 0.5 }
		local value = { coins = 2, settings = settings, tags = { "a" } }
		w.profile:set(value)
		spec.eq(rawequal(s.d.Profile(), value), true, "a conforming table is kept by identity")
		spec.eq(
			table.isfrozen(value) and table.isfrozen(settings) and table.isfrozen(value.tags),
			true,
			"and frozen in place"
		)
		rejects(function()
			settings.volume = 1
		end, "readonly", "mutating a table shared with server state throws at the write")
		local runs = 0
		s.api.scope(function()
			s.api.effect(function()
				s.d.Profile()
				runs += 1
			end)
		end)
		w.link:flush()
		w.profile:set(value)
		w.link:flush()
		spec.eq(runs, 1, "admitting an admitted value returns it unchanged")
		local extra = { coins = 3, settings = settings, tags = value.tags, legacy = { [1] = 1, [3] = 3 } }
		w.profile:set(extra)
		local stored = s.d.Profile()
		spec.eq(rawequal(stored, extra), false, "a table with an undeclared key is replaced")
		spec.eq(stored.legacy, nil, "by one holding exactly its declared fields")
		spec.eq(rawequal(stored.settings, settings), true, "sharing every kept subtree")
		spec.eq(table.isfrozen(stored), true, "frozen")
		spec.eq(table.isfrozen(extra), false, "the caller's table with the undeclared key is untouched")
		w.profile:update(function(current)
			return { coins = current.coins + 1, settings = current.settings, tags = { "b" } }
		end)
		spec.eq(table.isfrozen(s.d.Profile().tags), true, "update's result is frozen")
		spec.eq(s.d.Profile().coins, 4, "update stored the change")
		local projected = { coins = 9 }
		w.owned.Ann = projected
		local ann = w:join("Ann")
		spec.eq(table.isfrozen(projected), true, "a projection result is frozen")
		spec.eq(table.isfrozen(ann.d.Profile()), true, "a client apply is frozen")
		spec.eq(table.isfrozen(ann.d.Profile().settings), true, "to every table")
		w:finish()
	end,
	["50 a per-player publish returns an opaque Publication, and a public one a State over the readers' node"] = function()
		local w = newWorld()
		spec.eq(type(w.owner), "table", "a table")
		spec.eq(next(w.owner), nil, "holding nothing")
		spec.eq(table.isfrozen(w.owner), true, "frozen")
		spec.eq(getmetatable(w.owner), nil, "not callable")
		w.score:set(8)
		spec.eq(w.score(), 8, "the State reads the same node")
		spec.eq(w.server.d.Score(), 8, "as every server reader")
		w:finish()
	end,
	["73 ServerSync's own end sends every live peer a failed frame before its view ends"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		w.server.life.fault(w.server.state(w:sync()), CALLBACK, "framework fault")
		w.link:flush()
		local frames = w:frames(ann)
		spec.eq(frames[#frames].frame.kind, "failed", "a failed frame")
		spec.eq(ann.api.syncStatus(ann.d.Score), "failed", "the client fails every status")
		w.score:set(4)
		w:heartbeat()
		spec.eq(#w:frames(ann), #frames, "nothing after it")
		w:finish()
	end,
	["74 a failed boot leaves declarations at initial, pending and unsealed, and a second boot seals them again"] = function()
		local seals = 0
		local w = World.new({
			declare = declare,
			mustBoot = false,
			patch = function(world)
				local declarations = world.server.declarations
				local seal = declarations.sealDeclarations
				declarations.sealDeclarations = function()
					seals += 1
					seal()
				end
			end,
			serve = function(s, d, world)
				s.service("JobService", function()
					world.score = s.entry.publish(d.Score)
					world.score:set(5)
					world.profile = s.entry.publish(d.Profile)
				end)
				s.service("Broken", function()
					error("constructor failed")
				end, nil, { deps = { "JobService" } })
			end,
		})
		local s = w.server
		spec.eq(w.booted, false, "boot failed")
		spec.eq(seals, 1, "the first boot sealed the registry")
		spec.eq(s.d.Score(), 0, "the value is back at its admitted initial")
		spec.eq(s.api.syncStatus(s.d.Score), "pending", "and pending")
		spec.eq(s.declarations.declarationOf(s.d.Score).publication, nil, "the publication is gone")
		local late = s.emit.replicated("Job.AfterRollback", 0, s.n.f64)
		spec.eq(late(), 0, "the registry is unsealed")
		local ok, err = pcall(s.entry.boot, { load = { "crucible-path:ServerScriptService/Game" } })
		spec.eq(ok, false, "a second boot")
		spec.has(err, "boot already ran on this side", "stops at the core registry's one-boot rule")
		spec.eq(seals, 2, "after sealing the replicated registry again")
		spec.eq(s.d.Score(), 0, "and rolling it back again")
	end,
	["53 79 an unpublished declaration is never sent and is not diagnosed"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		for _, decoded in w:frames(ann) do
			local _, present = World.value(decoded, "Job.Spare")
			spec.eq(present, false, "never sent")
			for _, key in decoded.frame.failed do
				assert(key ~= "Job.Spare", "never failed")
			end
		end
		spec.eq(ann.api.syncStatus(ann.d.Spare), "pending", "pending")
		spec.eq(w:logged("Spare"), 0, "no diagnostic")
		spec.eq(#w.link.kicks, 0, "nobody kicked")
		w:finish()
	end,
}
