local World = require("./lib/sync-world")
local spec = require("./lib/load")

local CALLBACK = 8

local function declare(r)
	local n = r.n
	local owner = n.object({ { "coins", n.f64 }, { "name", n.string }, { "played", n.f64 } })
	return {
		Board = r.emit.replicated("Job.Board", { title = "" }, n.object({ { "title", n.string } })),
		Owner = r.emit.replicatedPerPlayer("Data.Owner", nil, n.union({ n["nil"], owner })),
		Score = r.emit.replicated("Job.Score", 0, n.f64),
	}
end

-- A profile service whose projection spreads the whole stored profile, as FD's ownerView does.
local function profiles(s, d, w)
	w.profiles = s.api.state({})
	w.pulls = {}
	s.service("ProfileService", function(self)
		w.owner = s.entry.publish(d.Owner, function(player)
			w.pulls[player.Name] = (w.pulls[player.Name] or 0) + 1
			local profile = w.profiles()[player.Name]
			if profile == nil then
				return nil
			end
			if profile.broken then
				error(`projection broke for {player.Name}`)
			end
			local view = table.clone(profile)
			view.played = 1
			return view
		end)
		w.score = s.entry.publish(d.Score)
		w.board = s.entry.publish(d.Board)
		w.service = self
	end)
end

local function newWorld()
	return World.new({ declare = declare, serve = profiles })
end

local function load(w, name, profile)
	local next = table.clone(w.profiles())
	next[name] = profile
	w.profiles:set(next)
end

local function ownerValues(w, client)
	local out = {}
	for _, decoded in w:frames(client) do
		local value, present = World.value(decoded, "Data.Owner")
		if present then
			table.insert(out, value)
		end
	end
	return out
end

return {
	["50 privacy by structure: a client never receives another player's per-player value on its decoded frames"] = function()
		local w = newWorld()
		load(w, "Ann", { coins = 111, name = "Ann" })
		load(w, "Bea", { coins = 222, name = "Bea" })
		local ann = w:join("Ann")
		local bea = w:join("Bea")
		load(w, "Ann", { coins = 112, name = "Ann" })
		load(w, "Bea", { coins = 223, name = "Bea" })
		w:heartbeat()
		spec.eq(ann.d.Owner().coins, 112, "Ann reads her own value")
		spec.eq(bea.d.Owner().coins, 223, "Bea reads hers")
		for _, value in ownerValues(w, ann) do
			spec.eq(value.name, "Ann", "every Data.Owner on Ann's wire is Ann's")
			assert(value.coins ~= 222 and value.coins ~= 223, "Bea's secret never reaches Ann")
		end
		for _, value in ownerValues(w, bea) do
			spec.eq(value.name, "Bea", "every Data.Owner on Bea's wire is Bea's")
		end
		spec.eq(#ownerValues(w, ann), 2, "Ann got her snapshot and her update")
		w:finish()
	end,
	["49 K3 privacy by type: undeclared keys and a legacy sparse table are dropped before any frame, and nothing fails"] = function()
		local w = newWorld()
		load(w, "Ann", {
			coins = 5,
			name = "Ann",
			purchaseIds = { "receipt-1", "receipt-2" },
			redeemedCodes = { SECRET = true },
			analytics = { sessions = 9 },
			legacy = { [1] = "a", [3] = "c" },
		})
		local ann = w:join("Ann")
		local value = ownerValues(w, ann)[1]
		for _, key in { "purchaseIds", "redeemedCodes", "analytics", "legacy" } do
			spec.eq(value[key], nil, `the decoded frame has no {key}`)
		end
		spec.eq(value.coins, 5, "declared fields arrive")
		spec.eq(value.played, 1, "including the one the projection added")
		local declaration = w.server.declarations.declarationOf(w.server.d.Owner)
		local accepted = w:sync().peers[ann.player].accepted[declaration].value
		for _, key in { "purchaseIds", "redeemedCodes", "analytics", "legacy" } do
			spec.eq(accepted[key], nil, `the server's admitted value has no {key}`)
		end
		spec.eq(ann.api.syncStatus(ann.d.Owner), "ready", "nothing failed")
		w.board:set({ title = "posted", internal = "hidden" })
		spec.eq(w.server.d.Board().internal, nil, "a public publish drops an extra field on the server")
		spec.eq(w.server.d.Board().title, "posted", "and keeps the declared one")
		w:heartbeat()
		local decoded = w:frames(ann)[2]
		spec.eq(World.value(decoded, "Job.Board").internal, nil, "the decoded public value has no extra field")
		spec.eq(w:logged("failed"), 0, "no diagnostic")
		w:finish()
	end,
	["50 a projection reads undefined until the profile loads, sends it next Heartbeat, and ends when the player leaves"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		spec.eq(ann.d.Owner(), nil, "undefined before the profile loads")
		spec.eq(ann.api.syncStatus(ann.d.Owner), "ready", "a ready root undefined")
		load(w, "Ann", { coins = 3, name = "Ann" })
		spec.eq(ann.d.Owner(), nil, "nothing before the Heartbeat")
		w:heartbeat()
		spec.eq(ann.d.Owner().coins, 3, "the profile next Heartbeat")
		local pulls = w.pulls.Ann
		w.link:leave(ann)
		w.link:flush()
		load(w, "Ann", { coins = 4, name = "Ann" })
		w:heartbeat()
		spec.eq(w.pulls.Ann, pulls, "no pull after the player left")
		w:finish()
	end,
	["51 79 a throwing projection fails that player's declaration alone, reports once, and recovers on its next value"] = function()
		local w = newWorld()
		load(w, "Ann", { coins = 1, name = "Ann" })
		load(w, "Bea", { coins = 2, name = "Bea" })
		local ann = w:join("Ann")
		local bea = w:join("Bea")
		load(w, "Bea", { coins = 2, name = "Bea", broken = true })
		load(w, "Ann", { coins = 10, name = "Ann" })
		w.score:set(7)
		w:heartbeat()
		local frames = w:frames(bea)
		spec.list(frames[2].frame.failed, { "Data.Owner" }, "Bea's frame lists the declaration failed")
		spec.list(World.keys(frames[2]), { "Job.Score" }, "her other values continue")
		spec.eq(bea.api.syncStatus(bea.d.Owner), "failed", "Bea's status is failed")
		spec.eq(bea.d.Owner().coins, 2, "her last value is kept")
		spec.eq(bea.d.Score(), 7, "other declarations continue")
		spec.eq(ann.d.Owner().coins, 10, "Ann continues")
		spec.eq(ann.api.syncStatus(ann.d.Owner), "ready", "Ann is ready")
		spec.eq(w:logged("Data.Owner for Bea failed"), 1, "one diagnostic naming the declaration and the player")
		load(w, "Bea", { coins = 2, name = "Bea" })
		w:heartbeat()
		frames = w:frames(bea)
		spec.eq(World.value(frames[3], "Data.Owner").coins, 2, "the recovered value is sent even though it is equal")
		spec.eq(bea.api.syncStatus(bea.d.Owner), "ready", "ready again")
		load(w, "Bea", { coins = 2, name = "Bea", broken = true })
		w:heartbeat()
		spec.eq(bea.api.syncStatus(bea.d.Owner), "failed", "failed again")
		spec.eq(w:logged("Data.Owner for Bea failed"), 1, "a second failure reports nothing new")
		w:finish()
	end,
	["49 51 a declared field of the wrong type behind a cast fails like a throw and recovers the same way"] = function()
		local w = newWorld()
		load(w, "Ann", { coins = 1, name = "Ann" })
		local ann = w:join("Ann")
		load(w, "Ann", { coins = "lots", name = "Ann" })
		w:heartbeat()
		spec.list(w:frames(ann)[2].frame.failed, { "Data.Owner" }, "listed failed")
		spec.eq(ann.api.syncStatus(ann.d.Owner), "failed", "failed")
		spec.eq(ann.d.Owner().coins, 1, "the last value is kept")
		spec.eq(w:logged("Data.Owner.coins: expected number, got string"), 1, "the reason names the path")
		spec.eq(w:logged("lots"), 0, "and never the value")
		load(w, "Ann", { coins = 2, name = "Ann" })
		w:heartbeat()
		spec.eq(ann.d.Owner().coins, 2, "the next accepted value")
		spec.eq(ann.api.syncStatus(ann.d.Owner), "ready", "ready again")
		w:finish()
	end,
	["52 54 a publisher fault fails its declarations for every live client and a later joiner, and its handle refuses writes"] = function()
		local w = newWorld()
		load(w, "Ann", { coins = 1, name = "Ann" })
		w.score:set(3)
		local ann = w:join("Ann")
		w.server.life.fault(w.server.state(w.service), CALLBACK, "publisher faulted")
		w.link:flush()
		w:heartbeat()
		local frames = w:frames(ann)
		spec.list(frames[2].frame.failed, { "Data.Owner", "Job.Board", "Job.Score" }, "every live peer's next frame")
		spec.eq(ann.api.syncStatus(ann.d.Score), "failed", "public status failed")
		spec.eq(ann.d.Score(), 3, "the last value is kept")
		spec.eq(ann.api.syncStatus(ann.d.Owner), "failed", "per-player status failed")
		spec.eq(ann.d.Owner().coins, 1, "its last value is kept")
		spec.eq(w.server.api.syncStatus(w.server.d.Score), "failed", "the server's status is failed")
		local ok, err = pcall(function()
			w.score:set(4)
		end)
		spec.eq(ok, false, "a write after retirement throws")
		spec.has(err, "Job.Score's publisher ProfileService has ended", "naming the declaration and the publisher")
		local bea = w:join("Bea")
		spec.list(w:frames(bea)[1].frame.failed, { "Data.Owner", "Job.Board", "Job.Score" }, "a later snapshot")
		spec.eq(bea.api.syncStatus(bea.d.Score), "failed", "a later joiner sees failed")
		spec.eq(bea.d.Score(), 0, "with initial")
		w:heartbeat()
		spec.eq(#w:frames(ann), 2, "a retired declaration is announced once")
		w:finish()
	end,
}
