local roblox = require("@lune/roblox")
local DM = require("./lib/data-migrations")
local World = require("./lib/sample-world")
local spec = require("./lib/load")

local STORE = "Sample.Players"
local COINS_VIEW = "Crucible.Data.Sample.Players.coins"
local E = roblox.Enum.KeyCode.E

local ANN, BOB = 4120, 4121
local ANN_PICKUPS, BOB_COINS, BOB_PICKUPS = 4242, 777, 31337

-- A saved profile as the current build writes it: schema version 1, which is the sample's one migration.
local function profile(coins: number, pickups: number)
	return DM.envelope({ schemaVersion = 1, payload = { coins = coins, pickups = pickups } })
end

local function seed(w, userId: number, coins: number, pickups: number)
	w.backend:seed(STORE, tostring(userId), profile(coins, pickups))
end

local function newWorld(seeds)
	local w = World.new()
	for userId, saved in seeds or {} do
		seed(w, userId, saved[1], saved[2])
	end
	w:bootServer()
	w:settle()
	return w
end

local function connect(w, name: string, userId: number)
	local client = w:connect(name, userId)
	w:settle()
	return client
end

local function shared(realm, name: string)
	return realm.load(realm.place:GetService("ReplicatedStorage").TS[name])
end

local function stored(w, userId: number)
	return w:envelope(STORE, userId)
end

-- Every leaf value and key in the sync frames the server sent `client`, as text.
local function wireTexts(w, client): string
	local out = {}
	local function walk(value)
		if type(value) == "table" then
			for key, entry in value do
				table.insert(out, tostring(key))
				walk(entry)
			end
		elseif value ~= nil then
			table.insert(out, tostring(value))
		end
	end
	for _, decoded in w:frames(client) do
		for _, entry in decoded.frame.values do
			table.insert(out, entry.key)
			walk(entry.value)
		end
	end
	return "|" .. table.concat(out, "|") .. "|"
end

local function logged(w, part: string): number
	local count = 0
	for _, entry in w.server.engine.gameLogs do
		if string.find(entry.message, part, 1, true) then
			count += 1
		end
	end
	return count
end

local function saved(w, player)
	local profileStore = shared(w.server, "profile").PlayerData
	local box = w:run(function()
		return profileStore.save(profileStore, player, 5)
	end)
	w:advance(1)
	assert(box.done, "the save barrier returned")
	assert(box.value.ok, "the profile saved")
end

return {
	["compiled sample server starts dependencies behaviours and shutdown under the fake engine"] = function()
		local w = World.new()
		local engine = w.server.engine
		local member = engine.instance("sample", "Part", { Identity = "fleet-23" })
		local missing = engine.instance("missing", "Part", {})
		engine.addTag(missing, "S2Member")
		engine.addTag(member, "S2Member")
		local entry = w:bootServer()
		assert(entry:GetAttribute("CrucibleServer") == true, "server boot returned")
		spec.list(w.server.messages, {
			"S2 store awake",
			"S2 controller awake",
			"S2 member awake fleet-23 17",
			"S2 member enable",
			"S2 controller start",
			"S2 member start",
			"S2 server booted",
		}, "compiled lifecycle")
		engine.removeTag(member, "S2Member")
		w.link:flush()
		assert(table.find(w.server.messages, "S2 member disable"), "tag removal disables")
		assert(table.find(w.server.messages, "S2 member destroy"), "tag removal destroys")
		assert(#w.server.closers == 1, "one shutdown callback")
		local closed = w:run(w.server.closers[1])
		w:advance(10)
		assert(closed.done, "shutdown finished")
		spec.list(
			{ w.server.messages[#w.server.messages - 1], w.server.messages[#w.server.messages] },
			{ "S2 controller shutdown", "S2 store shutdown" },
			"dependent shuts down first"
		)
		assert(#engine.logs == 1 and engine.logs[1].level == 0, "only missing required attribute warning")
		assert(w.server.imported.client == nil, "server does not load client")
		w.place:Destroy()
	end,

	["compiled sample client starts and runs the render hook under the fake engine"] = function()
		local w = newWorld()
		local ann = connect(w, "Ann", ANN)
		assert(ann.main:GetAttribute("CrucibleClient") == true, "client boot returned")
		assert(ann.player:GetAttribute("CrucibleBooted") == true, "boot sets local player flag")
		ann.engine.cycle(1 / 60, true)
		assert(table.find(ann.messages, "S2 renderer awake"), "renderer awoke")
		assert(table.find(ann.messages, "S2 renderer start"), "renderer started")
		assert(table.find(ann.messages, "S2 render 1"), "renderer ran")
		assert(#ann.closers == 0, "client does not bind shutdown")
		assert(#ann.engine.logs == 0, "no runtime faults")
		assert(ann.imported.server == nil, "client does not load server")
		w.place:Destroy()
	end,

	["compiled sample a press sends the message and the server writes the profile through its handle"] = function()
		local w = newWorld({ [ANN] = { 12, ANN_PICKUPS } })
		local ann = connect(w, "Ann", ANN)
		spec.eq(w:hud(ann), "Coins: 12   Total: 12   Last: +0", "hud after the profile loads")
		w:press(ann, E)
		w:settle()
		spec.eq(w:hud(ann), "Coins: 13   Total: 13   Last: +1", "the server's reply and the owner's view")
		saved(w, ann.player)
		local envelope = stored(w, ANN)
		spec.eq(envelope.payload.coins, 13, "the committed coins")
		spec.eq(envelope.payload.pickups, ANN_PICKUPS + 1, "the server-only counter counted the press")
		w.place:Destroy()
	end,

	["compiled sample the profile survives a leave and a rejoin"] = function()
		local w = newWorld()
		local ann = connect(w, "Ann", ANN)
		for _ = 1, 3 do
			w:press(ann, E)
			w:settle()
		end
		spec.eq(w:hud(ann), "Coins: 3   Total: 3   Last: +1", "three presses")
		w:leave(ann)
		w:settle(2)
		local envelope = stored(w, ANN)
		spec.eq(envelope.payload.coins, 3, "the release committed the coins")
		spec.eq(envelope.payload.pickups, 3, "the release committed the counter")
		local again = connect(w, "Ann", ANN)
		spec.eq(w:hud(again), "Coins: 3   Total: 3   Last: +0", "the rejoined client reads the saved coins")
		w.place:Destroy()
	end,

	["compiled sample a profile saved before the migration is migrated on load"] = function()
		local w = World.new()
		w.backend:seed(STORE, tostring(ANN), DM.envelope({ schemaVersion = 0, payload = { gold = 9 } }))
		w:bootServer()
		w:settle()
		local ann = connect(w, "Ann", ANN)
		spec.eq(w:hud(ann), "Coins: 9   Total: 9   Last: +0", "gold became coins")
		w:press(ann, E)
		w:settle()
		saved(w, ann.player)
		local envelope = stored(w, ANN)
		spec.eq(envelope.schemaVersion, 1, "written at the current schema version")
		spec.eq(envelope.payload.coins, 10, "migrated coins plus the press")
		spec.eq(envelope.payload.pickups, 1, "the new server-only field took its default")
		w.place:Destroy()
	end,

	["compiled sample the owner sees its public field and never the server-only field or another profile"] = function()
		local w = newWorld({ [ANN] = { 12, ANN_PICKUPS }, [BOB] = { BOB_COINS, BOB_PICKUPS } })
		local ann = connect(w, "Ann", ANN)
		local bob = connect(w, "Bob", BOB)
		w:press(ann, E)
		w:settle()
		local annData = shared(ann, "profile").PlayerData
		spec.eq(annData().coins(), 13, "the owner's handle reads its coins")
		local ok, held = pcall(function()
			return annData().pickups()
		end)
		assert(not (ok and held == ANN_PICKUPS), "the client handle never holds the server-only value")
		local bobData = shared(bob, "profile").PlayerData
		spec.eq(bobData().coins(), BOB_COINS, "the other owner reads its own coins")
		local annWire = wireTexts(w, ann)
		spec.has(annWire, "|13|", "Ann's own coins reach Ann")
		for _, secret in { ANN_PICKUPS, ANN_PICKUPS + 1, BOB_PICKUPS } do
			assert(not string.find(annWire, `|{secret}|`, 1, true), `no server-only value {secret} reaches Ann`)
		end
		assert(not string.find(annWire, `|{BOB_COINS}|`, 1, true), "Bob's coins never reach Ann as a profile value")
		assert(not string.find(annWire, "pickups", 1, true), "no server-only field name reaches Ann")
		local bobWire = wireTexts(w, bob)
		assert(not string.find(bobWire, "|13|", 1, true), "Ann's coins never reach Bob as a profile value")
		for _, decoded in w:frames(ann) do
			for _, entry in decoded.frame.values do
				if entry.key == COINS_VIEW then
					assert(
						entry.value == nil or entry.value == 12 or entry.value == 13,
						"only Ann's coins in Ann's view"
					)
				end
			end
		end
		w.place:Destroy()
	end,

	["compiled sample every client sees the replicated total"] = function()
		local w = newWorld({ [ANN] = { 12, ANN_PICKUPS }, [BOB] = { BOB_COINS, BOB_PICKUPS } })
		local ann = connect(w, "Ann", ANN)
		local bob = connect(w, "Bob", BOB)
		spec.eq(w:hud(ann), "Coins: 12   Total: 789   Last: +0", "Ann sees the total of both profiles")
		spec.eq(w:hud(bob), "Coins: 777   Total: 789   Last: +0", "Bob sees the same total and his own coins")
		w:press(bob, E)
		w:settle()
		spec.eq(w:hud(ann), "Coins: 12   Total: 790   Last: +0", "Bob's press moves Ann's total, not Ann's coins")
		spec.eq(w:hud(bob), "Coins: 778   Total: 790   Last: +1", "Bob sees his coins and the total")
		local totals = {}
		for _, client in { ann, bob } do
			local TotalCoins = shared(client, "state").TotalCoins
			table.insert(totals, TotalCoins())
		end
		spec.list(totals, { 790, 790 }, "the replicated value on each client")
		w.place:Destroy()
	end,

	["compiled sample a config change changes the next press and reruns the effect that reads it"] = function()
		local w = newWorld()
		local ann = connect(w, "Ann", ANN)
		spec.eq(logged(w, "coin multiplier 1"), 1, "the effect ran once with the fallback")
		w.configs:setGlobal("GlobalEvent", { multiplier = 3, endsAt = 0 })
		local refresh = w:run(function()
			return w.server.graph.module("server").refreshConfigs()
		end)
		w:advance(1)
		assert(refresh.done and refresh.value == true, "the refresh finished")
		spec.eq(logged(w, "coin multiplier 3"), 1, "the effect reran with the new value")
		w:press(ann, E)
		w:settle()
		spec.eq(w:hud(ann), "Coins: 3   Total: 3   Last: +3", "the press paid the new multiplier")
		w.configs:setGlobal("GlobalEvent", { multiplier = 5, endsAt = 1 })
		w:run(function()
			return w.server.graph.module("server").refreshConfigs()
		end)
		w:advance(1)
		spec.eq(logged(w, "coin multiplier 1"), 2, "an event that ended pays the fallback multiplier")
		w:press(ann, E)
		w:settle()
		spec.eq(w:hud(ann), "Coins: 4   Total: 4   Last: +1", "the press after the event paid one")
		w.place:Destroy()
	end,

	["compiled sample a pickup outside the allowed size changes nothing"] = function()
		local w = newWorld({ [ANN] = { 12, ANN_PICKUPS } })
		local ann = connect(w, "Ann", ANN)
		local Collect = shared(ann, "messages").Collect
		local Network = ann.graph.module("client").Network
		for _, size in { 0, 6 } do
			Network.send(Collect, { size = size })
			w:settle()
		end
		spec.eq(w:hud(ann), "Coins: 12   Total: 12   Last: +0", "neither size paid")
		Network.send(Collect, { size = 5 })
		w:settle()
		spec.eq(w:hud(ann), "Coins: 17   Total: 17   Last: +5", "the largest allowed size paid")
		w.place:Destroy()
	end,
}
