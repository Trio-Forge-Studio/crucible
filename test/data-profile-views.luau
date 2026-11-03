local Backend = require("./lib/data-backend")
local DM = require("./lib/data-migrations")
local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("./lib/load")

local SECRET_NOTE = "SECRET-NOTE-4120"
local UNKNOWN_ROOT = "UNKNOWN-ROOT-4120"
local UNKNOWN_NESTED = "UNKNOWN-NESTED-4120"
local BEA_COINS = 777777

local function declare(r)
	return { PlayerData = fixture.declare(r) }
end

local function newWorld(options)
	options = options or {}
	return World.new({
		declare = options.declare or declare,
		serve = options.serve,
		backend = options.backend,
		mustBoot = options.mustBoot,
	})
end

local function throws(fn, part: string, label: string)
	local ok, err = pcall(fn)
	assert(not ok, `{label}: expected a throw`)
	spec.has(err, part, label)
	return err
end

-- Every leaf value and every key in a decoded value, as text.
local function texts(value, out)
	out = out or {}
	if type(value) == "table" then
		for key, entry in value do
			table.insert(out, tostring(key))
			texts(entry, out)
		end
	elseif value ~= nil then
		table.insert(out, tostring(value))
	end
	return out
end

local function wireTexts(w, client)
	local out = {}
	for _, decoded in w:frames(client) do
		for _, entry in decoded.frame.values do
			table.insert(out, entry.key)
			texts(entry.value, out)
		end
	end
	return table.concat(out, "|")
end

local function seedAnn(backend)
	backend:seed(
		fixture.STORE,
		"4120",
		DM.envelope({
			payload = {
				analytics = { playTime = 5 },
				career = { jobs = {} },
				currency = 12,
				mastery = {},
				notes = SECRET_NOTE,
				purchaseLog = { p1 = 1 },
				questProgress = {},
				secretRoot = UNKNOWN_ROOT,
				settings = { hidden = UNKNOWN_NESTED, vibration = true, volume = 0.5 },
				upgrades = {},
				vault = { pin = 1234 },
				xp = 0,
			},
		})
	)
end

local function sortedKeys(decoded)
	local keys = World.keys(decoded)
	table.sort(keys)
	return keys
end

-- The view keys a frame carries: the lifecycle, then every public field but `skip`, whose value did not change.
local function expectedKeys(withLive: boolean, skip: string?)
	local keys = {}
	if withLive then
		table.insert(keys, fixture.LIVE)
	end
	for _, field in fixture.PUBLIC do
		if field ~= skip then
			table.insert(keys, fixture.view(field))
		end
	end
	table.sort(keys)
	return keys
end

-- Ann (4120) joined and loaded, with her client's frames counted from here.
local function loaded(options)
	local w = newWorld(options)
	local ann = w:player("Ann", 4120)
	w:settle()
	w:settle()
	assert(w.server.d.PlayerData:loaded(ann.player), "loaded")
	return w, ann, ann.player, w.server.d.PlayerData
end

return {
	["125 129 K1 a snapshot carries the lifecycle and every public field, never a serverOnly field, an unknown key or another player's data"] = function()
		local backend = Backend.new()
		seedAnn(backend)
		local w = newWorld({ backend = backend })
		local bea = w:player("Bea", 4121)
		w:settle()
		w.server.d.PlayerData(bea.player).currency:set(BEA_COINS)
		local ann = w:player("Ann", 4120)
		w:settle()
		w:settle()
		local frames = w:frames(ann)
		assert(#frames >= 1, "Ann received frames")
		local loadFrame
		for _, decoded in frames do
			local live = World.value(decoded, fixture.LIVE)
			if live == true then
				loadFrame = decoded
			end
		end
		assert(loadFrame, "a frame carried the lifecycle true")
		spec.list(sortedKeys(frames[1]), expectedKeys(true), "the snapshot carries every view declaration")
		spec.eq(World.value(frames[1], fixture.LIVE), false, "the snapshot's lifecycle is false while the server loads")
		spec.list(
			sortedKeys(loadFrame),
			expectedKeys(true, "nickname"),
			"the load's frame carries every declaration whose value changed; the absent nickname did not"
		)
		local settings = World.value(loadFrame, fixture.view("settings"))
		spec.eq(settings.hidden, nil, "the nested unknown key is stripped from the decoded field")
		spec.eq(World.value(loadFrame, fixture.view("currency")), 12, "the public field's value")
		local wire = wireTexts(w, ann)
		for _, secret in
			{
				SECRET_NOTE,
				UNKNOWN_ROOT,
				UNKNOWN_NESTED,
				tostring(BEA_COINS),
				"1234",
				"secretRoot",
				"notes",
				"purchaseLog",
				"vault",
				"analytics",
			}
		do
			assert(string.find(wire, secret, 1, true) == nil, `{secret} never reaches Ann's decoded frames`)
		end
		spec.eq(ann.d.PlayerData().currency(), 12, "Ann reads her own currency")
		spec.eq(ann.d.PlayerData().settings().hidden, nil, "the client value has no unknown key")
		w:finish()
	end,

	["125 a currency write's frame carries only the currency declaration"] = function()
		local w, ann, player, data = loaded()
		local before = #w:frames(ann)
		data(player).currency:set(5)
		w:heartbeat()
		local after = w:framesSince(ann, before)
		spec.eq(#after, 1, "one frame")
		spec.list(World.keys(after[1]), { fixture.view("currency") }, "only the currency declaration")
		spec.eq(World.value(after[1], fixture.view("currency")), 5, "with the new value")
		w:finish()
	end,

	["125 RX 60 a root update of two fields arrives as one frame and one apply, with no effect between"] = function()
		local w, ann, player, data = loaded()
		local seen = {}
		ann.api.scope(function()
			ann.api.effect(function()
				table.insert(seen, `{ann.d.PlayerData().currency()}/{ann.d.PlayerData().xp()}`)
			end)
		end)
		w:heartbeat()
		local before = #w:frames(ann)
		data(player):update(function(d)
			local copy = table.clone(d)
			copy.currency = 8
			copy.xp = 3
			return copy
		end)
		w:heartbeat()
		w:heartbeat()
		local after = w:framesSince(ann, before)
		spec.eq(#after, 1, "one frame")
		spec.list(sortedKeys(after[1]), { fixture.view("currency"), fixture.view("xp") }, "both fields in it")
		spec.list(seen, { "0/0", "8/3" }, "the effect never saw half of the update")
		w:finish()
	end,

	["125 E25 a serverOnly-only write, an unknown-key change and a reverted write send nothing"] = function()
		local w, ann, player, data = loaded()
		data(player).currency:set(4)
		w:heartbeat()
		local before = #w:frames(ann)
		data(player).notes:set("server only")
		data(player).analytics.playTime:set(60)
		w:heartbeat()
		data(player).settings:update(function(s)
			local copy = table.clone(s)
			copy.extra = "unknown"
			return copy
		end)
		w:heartbeat()
		data(player).currency:set(99)
		data(player).currency:set(4)
		w:heartbeat()
		spec.eq(#w:framesSince(ann, before), 0, "no frame for any of them")
		spec.eq(data(player).notes(), "server only", "the serverOnly write is stored")
		spec.eq(data(player).settings().extra, "unknown", "the unknown key is stored")
		w:finish()
	end,

	["126 loaded() is false before the snapshot, while loading and after a handoff seal, true after the load"] = function()
		local w = newWorld()
		local ann = w:player("Ann", 4120)
		local data = w.server.d.PlayerData
		spec.eq(ann.d.PlayerData:loaded(), false, "before the snapshot")
		spec.eq(ann.d.PlayerData().currency(), 0, "a client read before load returns the client template")
		spec.eq(ann.d.PlayerData().settings.volume(), 0.5, "a nested template value")
		w:heartbeat()
		spec.eq(ann.d.PlayerData:loaded(), false, "while the server still loads")
		w:settle()
		w:settle()
		spec.eq(ann.d.PlayerData:loaded(), true, "after the load")
		data(ann.player).currency:set(30)
		w:heartbeat()
		spec.eq(ann.d.PlayerData().currency(), 30, "the live value")
		local before = #w:frames(ann)
		local box = w:run(function()
			return data:handoff(ann.player)
		end)
		w:advance(2)
		w:heartbeat()
		spec.eq(box.value.ok, true, "handoff")
		spec.eq(ann.d.PlayerData:loaded(), false, "after the handoff seal")
		spec.eq(ann.d.PlayerData().currency(), 0, "reads return the template after a seal")
		local seal = w:framesSince(ann, before)
		spec.eq(#seal, 1, "the seal is one frame")
		spec.list(
			sortedKeys(seal[1]),
			expectedKeys(true, "nickname"),
			"carrying the lifecycle and every field it unset"
		)
		spec.eq(World.value(seal[1], fixture.LIVE), false, "the lifecycle false")
		spec.eq(World.value(seal[1], fixture.view("currency")), nil, "the fields undefined")
		w:finish()
	end,

	["126 107 M3 after DataRuntime retires the views fail for every owner and reads keep the last values"] = function()
		local w = newWorld()
		local ann = w:player("Ann", 4120)
		local bea = w:player("Bea", 4121)
		w:settle()
		w:settle()
		local data = w.server.d.PlayerData
		data(ann.player).currency:set(41)
		data(bea.player).currency:set(42)
		w:heartbeat()
		spec.eq(ann.d.PlayerData:loaded(), true, "Ann loaded")
		w:retireData()
		w:heartbeat()
		w:heartbeat()
		for _, client in { ann, bea } do
			spec.eq(client.d.PlayerData:loaded(), false, `{client.name}'s loaded() is false`)
		end
		spec.eq(ann.d.PlayerData().currency(), 41, "a failed view keeps returning the last received value")
		spec.eq(bea.d.PlayerData().currency(), 42, "for every owner")
		w:advance(5)
		assert(w:kicked(ann.player) ~= nil, "Ann is kicked once her release settles")
	end,

	["126 M3 one failed field view turns loaded() false and its reads keep the last value"] = function()
		local w, ann, player, data = loaded()
		data(player).currency:set(17)
		w:heartbeat()
		spec.eq(ann.d.PlayerData:loaded(), true, "loaded")
		local revision = #w:frames(ann)
		w:inject(ann, { failed = { fixture.view("currency") }, kind = "update", revision = revision + 1, values = {} })
		spec.eq(ann.d.PlayerData:loaded(), false, "a failed field declaration makes loaded() false")
		spec.eq(ann.d.PlayerData().currency(), 17, "the failed field keeps its last value")
		spec.eq(ann.d.PlayerData().settings.volume(), 0.5, "the other fields read as before")
	end,

	["124 134 E17 the field declarations are the public fields, the client root holds them, and client writes throw"] = function()
		local w, ann, player, data = loaded()
		local names = {}
		for _, declaration in ann.graph.module("internal/sync/declarations").declarations() do
			if string.sub(declaration.name, 1, 9) == "Crucible." then
				table.insert(names, declaration.name)
			end
		end
		spec.list(names, expectedKeys(true), "Crucible.Live and one Crucible.Data per public field, nothing else")
		data(player).upgrades.speed:set(2)
		w:heartbeat()
		local root = ann.d.PlayerData()()
		local keys = {}
		for key in root do
			table.insert(keys, key)
		end
		table.sort(keys)
		spec.list(
			keys,
			{ "career", "currency", "mastery", "questProgress", "settings", "upgrades", "xp" },
			"the client root's keys"
		)
		assert(table.isfrozen(root), "frozen")
		spec.eq(ann.d.PlayerData().upgrades.speed(), 2, "a client map entry read")
		assert(rawequal(ann.d.PlayerData(), ann.d.PlayerData()), "the client root is made once per realm")
		local client = ann.d.PlayerData
		throws(function()
			client().currency:set(1)
		end, "FD.Players() is read-only on the client", "a client write behind a cast")
		throws(function()
			client(player)
		end, "a client reads only its own profile; call FD.Players()", "a client call with a player")
		throws(function()
			client:save(player)
		end, "FD.Players.save is server-only", "a server member on the client")
		w:finish()
	end,

	["127 14 E24 a public string over SYNC_STRING_BYTES is refused at prepare and the same string in a serverOnly field is accepted"] = function()
		local w, _, player, data = loaded()
		local long = string.rep("x", 49_153)
		throws(function()
			data(player).nickname:set(long)
		end, "FD.Players.nickname", "the public string is refused naming the store and path")
		spec.eq(data(player).nickname(), nil, "nothing stored")
		spec.eq(data(player).notes:set(long), true, "the serverOnly string is accepted")
		spec.eq(#data(player).notes(), 49_153, "stored")
		spec.eq(data(player).nickname:set(string.rep("y", 49_152)), true, "a public string at the limit is accepted")
		w:finish()
	end,

	["127 a write over the store's share is refused naming the share"] = function()
		local w, _, player, data = loaded()
		local upgrades = {}
		for i = 1, 8000 do
			upgrades[`u{i}`] = i
		end
		throws(function()
			data(player).upgrades:set(upgrades)
		end, "FD.Players: the owner's views would cost", "the share refusal")
		spec.eq(next(data(player).upgrades()), nil, "nothing stored")
		w:finish()
	end,

	["127 with two profile stores each share is half, and a template over its share fails boot"] = function()
		local n
		local function twoStores(r)
			n = r.n
			local api = r.graph.module("")
			local _, replica = fixture.nodes(r.n)
			return {
				PlayerData = fixture.declare(r),
				Extra = r.emit.profileStore("FD.Extra", {
					template = { blob = string.rep("b", 33_000) },
				}, r.n.object({ { "blob", r.n.string } }), r.n.object({ { "blob", r.n.string } }), {}),
				_unused = { api, replica },
			}
		end
		local w = newWorld({ declare = twoStores, mustBoot = false })
		assert(not w.booted, "boot failed")
		spec.has(w.bootError, "FD.Extra: the owner's views would cost", "naming the store over its share")
		spec.has(w.bootError, "over this store's share of 32768 (2 profile stores)", "each share is half")
		assert(n ~= nil, "declared")
	end,

	["127 a load over a view bound fails with the data untouched"] = function()
		local backend = Backend.new()
		local seed = DM.envelope({
			payload = {
				analytics = { playTime = 0 },
				career = { jobs = {} },
				currency = 0,
				mastery = {},
				nickname = string.rep("n", 50_000),
				notes = "",
				purchaseLog = {},
				questProgress = {},
				settings = { vibration = true, volume = 0.5 },
				upgrades = {},
				vault = { pin = 0 },
				xp = 0,
			},
		})
		backend:seed(fixture.STORE, "4120", seed)
		local w = newWorld({ backend = backend })
		local ann = w:player("Ann", 4120)
		w:settle(2)
		spec.eq(w.server.d.PlayerData:loaded(ann.player), false, "the load failed")
		spec.eq(w:kicked(ann.player), "Your data could not load. Please rejoin.", "LOAD_FAILED_MESSAGE")
		assert(w:logged("profile does not fit the owner's views") >= 1, "diagnosed")
		local stored = w:envelope(fixture.STORE, 4120)
		DM.deepEq(stored.payload, seed.payload, "the payload is untouched")
		spec.eq(stored.lease, nil, "the acquisition was abandoned")
		w:finish()
	end,

	["134 a store whose declarations would pass MAX_REPLICATED_DECLARATIONS throws at evaluation with none declared"] = function()
		local g = spec.runtime({}, require("./lib/engine").new().loggingPort)
		local emit = g.module("emit")
		local registry = g.module("internal/sync/declarations")
		for i = 1, 248 do
			emit.replicated(`Game.Filler{i}`, 0, emit.n.f64)
		end
		local r = { graph = g, emit = emit, n = emit.n }
		throws(function()
			fixture.declare(r)
		end, "FD.Players needs 9 replicated declarations; 248 of 256 are in use", "the count message")
		spec.eq(registry.declarationNamed(fixture.LIVE), nil, "no lifecycle declared")
		spec.eq(registry.declarationNamed(fixture.view("currency")), nil, "no field declared")
		spec.eq(registry.replicatedCapacity(), 8, "the realm's room is unchanged")
	end,
}
