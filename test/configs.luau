local spec = require("./lib/load")
local Engine = require("./lib/engine")
local Backend = require("./lib/config-backend")
local task = require("@lune/task")

local Want = { Destroyed = 2 }

local function fakeGame(services)
	return {
		GetService = function(_, name)
			table.insert(services, name)
			if name == "RunService" then
				return {
					IsServer = function()
						return true
					end,
				}
			end
			error(`configs reached the {name} service`)
		end,
	}
end

local function world()
	local backend = Backend.new()
	local engine = Engine.new()
	engine.port.clock = function()
		return backend.now
	end
	engine.port.wait = function() end
	engine.port.spawn = function(callback)
		task.spawn(callback)
	end
	engine.port.bindToClose = function() end
	local services = {}
	local g = spec.runtime({ game = fakeGame(services) }, engine.loggingPort, backend.port)
	local w = { backend = backend, engine = engine, g = g, services = services }
	w.server = g.module("server")
	w.emit = g.module("emit")
	w.n = w.emit.n
	w.boot = g.module("internal/boot")
	w.life = g.module("internal/lifecycle")
	w.registry = g.module("internal/registry")
	function w.ignite()
		w.boot.ignite(engine.port, {
			name = "server",
			renders = false,
			booted = function() end,
		}, {})
	end
	function w.start()
		w.ignite()
		backend:advance(0)
	end
	function w.refresh()
		local box = { done = false }
		task.spawn(function()
			box.value = w.server.refreshConfigs()
			box.done = true
		end)
		return box
	end
	function w.diagnostics(part)
		local found = {}
		for _, log in engine.logs do
			if part == nil or string.find(log.message, part, 1, true) then
				table.insert(found, log.message)
			end
		end
		return found
	end
	function w.runtimeState()
		for _, record in w.registry.records() do
			if record.id == "Crucible.ConfigRuntime" then
				return record.state
			end
		end
		return nil
	end
	return w
end

local function eventNode(n)
	return n.object({
		{ "endsAt", n.f64 },
		{ "multipliers", n.object({ { "coins", n.f64 }, { "xp", n.f64 } }) },
		{ "name", n.string },
		{ "startsAt", n.f64 },
	})
end

local NO_EVENT = { name = "", startsAt = 0, endsAt = 0, multipliers = { coins = 1, xp = 1 } }
local DOUBLE = {
	name = "Double Coins Weekend",
	startsAt = 1793462400,
	endsAt = 1793635200,
	multipliers = { coins = 2, xp = 1.5 },
}

local function deepFrozen(value)
	if type(value) ~= "table" then
		return true
	end
	if not table.isfrozen(value) then
		return false
	end
	for _, child in value do
		if not deepFrozen(child) then
			return false
		end
	end
	return true
end

local function throws(callback, part, label)
	local ok, err = pcall(callback)
	spec.eq(ok, false, label .. " throws")
	spec.has(err, part, label .. " message")
end

local tests = {}

tests["1 94 a read before the first fetch returns the frozen fallback"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	local before = event()
	spec.eq(before.name, "", "fallback before boot")
	spec.eq(rawequal(before, NO_EVENT), false, "a copy, not the declared table")
	spec.eq(deepFrozen(before), true, "fallback frozen")
	spec.eq(w.server.configReady(), false, "not ready before boot")
	w.backend.globals.GlobalEvent = DOUBLE
	w.backend:delay("global", 5)
	w.start()
	w.backend:advance(4.9)
	spec.eq(event().name, "", "fallback while the first fetch is in flight")
	spec.eq(w.server.configReady(), false, "not ready while fetching")
	w.backend:advance(0.1)
	spec.eq(event().name, "Double Coins Weekend", "fetched value")
	spec.eq(w.server.configReady(), true, "ready after the first fetch")
end

tests["2 99 a first fetch applies every key with readiness"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local label = w.emit.config("Shop.Label", "none", w.n.string)
	w.backend.globals["Shop.Coins"] = 3
	w.backend.globals["Shop.Label"] = "sale"
	w.backend:delay("global", 2)
	w.start()
	local seen = {}
	task.spawn(function()
		for _ = 1, 8 do
			table.insert(seen, `{coins()} {label()} {w.server.configReady()}`)
			w.backend:sleep(0.5)
		end
	end)
	w.backend:advance(4)
	for _, row in seen do
		spec.eq(row == "1 none false" or row == "3 sale true", true, "no observer sees a partial apply: " .. row)
	end
	spec.eq(seen[#seen], "3 sale true", "applied")
end

tests["3 98 UpdateAvailable changes nothing until refreshConfigs"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	w.backend.globals.GlobalEvent = DOUBLE
	w.start()
	local requests = #w.backend.requests
	w.backend:setGlobal(
		"GlobalEvent",
		{ name = "Later", startsAt = 1, endsAt = 2, multipliers = { coins = 3, xp = 1 } }
	)
	w.backend:advance(600)
	spec.eq(event().name, "Double Coins Weekend", "old value kept")
	spec.eq(#w.backend.requests, requests, "no request on its own")
	local result = w.refresh()
	spec.eq(result.value, true, "refresh current")
	spec.eq(event().name, "Later", "new value after refresh")
end

tests["4 99 100 a refresh applies changed keys together and keeps unchanged ones by identity"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	local label = w.emit.config("Shop.Label", "none", w.n.string)
	w.backend.globals["Shop.Coins"] = 3
	w.backend.globals.GlobalEvent = DOUBLE
	w.backend.globals["Shop.Label"] = "a"
	w.start()
	local held = event()
	w.backend.globals["Shop.Coins"] = 4
	w.backend.globals.GlobalEvent = table.clone(DOUBLE)
	w.backend:setGlobal("Shop.Label", "b")
	w.backend:delay("global", 1)
	local result = w.refresh()
	w.backend:advance(0.5)
	spec.eq(`{coins()} {label()}`, "3 a", "nothing applied mid-refresh")
	w.backend:advance(0.5)
	spec.eq(result.value, true, "refresh result")
	spec.eq(`{coins()} {label()}`, "4 b", "both keys applied")
	spec.eq(rawequal(event(), held), true, "an equal value keeps its identity")
end

tests["5 97 110 an invalid value keeps the last good value and diagnoses once without the value"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	w.backend.globals.GlobalEvent =
		{ name = "Bad", startsAt = 1, endsAt = "Sunday", multipliers = { coins = 2, xp = 1 } }
	w.start()
	spec.eq(event().name, "", "invalid first value reads the fallback")
	spec.eq(#w.diagnostics("GlobalEvent.endsAt"), 1, "diagnosed with its path")
	w.backend:setGlobal("GlobalEvent", DOUBLE)
	spec.eq(w.refresh().value, true, "good refresh")
	w.backend:setGlobal("GlobalEvent", {
		name = "Sunday Sale",
		startsAt = 1,
		endsAt = "Sunday",
		multipliers = { coins = 2, xp = 1 },
	})
	spec.eq(w.refresh().value, true, "the scope is current")
	spec.eq(event().name, "Double Coins Weekend", "last good value kept")
	local found = w.diagnostics("GlobalEvent.endsAt")
	spec.eq(#found, 2, "once per generation")
	spec.eq(string.find(found[2], "Sunday", 1, true), nil, "no value in the diagnostic")
	spec.has(found[2], "(global snapshot): GlobalEvent.endsAt: expected a number", "names the scope, path and reason")
	spec.eq(w.refresh().value, true, "nothing outdated")
	spec.eq(#w.diagnostics("GlobalEvent.endsAt"), 2, "a refresh with nothing new does not diagnose again")
	for _, log in w.engine.logs do
		spec.eq(log.level, 0, "a warning")
	end
end

tests["5 97 JSON text is decoded and bad JSON keeps the value"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	local label = w.emit.config("Label", "none", w.n.string)
	w.backend.globals.GlobalEvent =
		'{"name":"Text","startsAt":1,"endsAt":2,"multipliers":{"coins":2,"xp":1},"theme":"gold"}'
	w.backend.globals.Label = '{"not":"decoded"}'
	w.start()
	spec.eq(event().name, "Text", "decoded")
	spec.eq(event().theme, "gold", "undeclared field kept")
	spec.eq(label(), '{"not":"decoded"}', "a string config stays text")
	w.backend:setGlobal("GlobalEvent", '{"name": ')
	spec.eq(w.refresh().value, true, "refresh")
	spec.eq(event().name, "Text", "bad JSON keeps the value")
	spec.eq(#w.diagnostics("invalid JSON"), 1, "diagnosed")
	w.backend:setGlobal("GlobalEvent", string.rep(" ", 100001))
	w.refresh()
	spec.eq(#w.diagnostics("over 100000 bytes"), 1, "text over the byte limit refused before decoding")
end

tests["5 97 admission checks every declared shape"] = function()
	local w = world()
	local n = w.n
	local shape = w.emit.config(
		"Shape",
		{ a = { 1 }, m = {}, t = { 1, "x" }, u = 1 },
		n.object({
			{ "a", n.array(n.u8) },
			{ "m", n.map(n.string, n.bool) },
			{ "o", n.union({ n["nil"], n.string }) },
			{ "t", n.tuple({ n.u16, n.string }) },
			{ "u", n.union({ n.literal(1), n.literal(2) }) },
		})
	)
	w.start()
	local function try(value)
		local before = #w.diagnostics("Shape")
		w.backend:setGlobal("Shape", value)
		w.refresh()
		return #w.diagnostics("Shape") == before
	end
	spec.eq(try({ a = { 255 }, m = { k = true }, t = { 65535, "y" }, u = 2, o = "z" }), true, "valid")
	spec.eq(shape().o, "z", "optional present")
	spec.eq(try({ a = { 256 }, m = {}, t = { 1, "x" }, u = 1 }), false, "u8 range")
	spec.eq(try({ a = { 1.5 }, m = {}, t = { 1, "x" }, u = 1 }), false, "integer")
	spec.eq(try({ a = { [1] = 1, [3] = 1 }, m = {}, t = { 1, "x" }, u = 1 }), false, "dense array")
	spec.eq(try({ a = {}, m = { k = 1 }, t = { 1, "x" }, u = 1 }), false, "map value")
	spec.eq(try({ a = {}, m = {}, t = { 1 }, u = 1 }), false, "tuple length")
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" }, u = 3 }), false, "literal union")
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" } }), false, "required field missing")
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" }, u = 1, o = 4 }), false, "optional field type")
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" }, u = 1, x = { 0 / 0 } }), false, "undeclared value must be JSON")
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" }, u = 1, x = "\255" }), false, "UTF-8")
	spec.eq(shape().o, "z", "the last good value survived every refusal")
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" }, u = 1 }), true, "absent optional")
	spec.eq(shape().o, nil, "absent optional stays absent")
	local deep = {}
	local node = deep
	for _ = 1, 30 do
		node.d = {}
		node = node.d
	end
	spec.eq(try({ a = {}, m = {}, t = { 1, "x" }, u = 1, x = deep }), false, "depth bound")
end

tests["6 97 a missing key reads as the fallback"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.start()
	spec.eq(coins(), 3, "loaded")
	w.backend:setGlobal("Shop.Coins", nil)
	spec.eq(w.refresh().value, true, "refresh")
	spec.eq(coins(), 1, "deleted key is a reset to the fallback")
end

tests["7 94 a throwing first fetch retries with capped full jitter while reads stay at the fallback"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.backend:failNext("global", "throw", 6)
	w.backend:failNext("global", "load-failed", 1)
	w.start()
	local waits = { 1, 2, 4, 8, 16, 30, 30 }
	for i, seconds in waits do
		spec.eq(w.backend:count("fetch global"), i, `attempt {i}`)
		spec.eq(coins(), 1, "fallback while failing")
		w.backend:advance(seconds - 0.01)
		spec.eq(w.backend:count("fetch global"), i, `no retry before {seconds}s`)
		w.backend:advance(0.01)
	end
	spec.eq(w.backend:count("fetch global"), 8, "eighth attempt")
	spec.eq(coins(), 3, "applied once a snapshot loads")
	spec.eq(#w.diagnostics("request (global snapshot)"), 1, "a failing loop diagnoses once")
	spec.eq(w.backend.updateListeners, 1, "only the loaded snapshot listens")
end

tests["8 95 96 a player value is the global value until their snapshot is applied"] = function()
	local w = world()
	local discount = w.emit.playerConfig("Shop.WelcomeDiscount", 0, w.n.f64)
	w.backend.globals["Shop.WelcomeDiscount"] = 5
	w.start()
	w.backend.targeted[7] = { ["Shop.WelcomeDiscount"] = 9 }
	w.backend:delay(7, 3)
	local player = w.backend:join(7)
	spec.eq(discount(player), 5, "untargeted dashboard value before the snapshot")
	spec.eq(w.server.configReady(player), false, "not ready before the snapshot")
	w.backend:advance(3)
	spec.eq(discount(player), 9, "targeted value")
	spec.eq(w.server.configReady(player), true, "ready after")
	local other = w.backend:join(8)
	w.backend:advance(3)
	spec.eq(discount(other), 5, "another player's targeting does not leak")
	w.backend:setForPlayer(8, "Shop.WelcomeDiscount", 11)
	w.backend:delay(8, 0)
	spec.eq(w.refresh().value, true, "refresh")
	spec.eq(discount(other), 11, "a refresh applies targeting")
	spec.eq(discount(player), 9, "unchanged")
	spec.eq(w.backend.updateListeners, 3, "one listener per snapshot")
	w.backend:leave(other)
	spec.eq(w.backend.updateListeners, 2, "leaving disconnects the player's snapshot")
	spec.eq(w.server.configReady(other), false, "false after leaving")
	spec.eq(discount(other), 0, "a departed player gets the fallback")
	spec.eq(discount(nil), 0, "no player reads the fallback")
end

tests["8 95 a global config reads only the global snapshot"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.backend.targeted[7] = { ["Shop.Coins"] = 99 }
	w.backend:join(7)
	w.start()
	spec.eq(coins(), 3, "global value")
	spec.eq(w.backend:count("fetch 7"), 1, "a present player's snapshot is fetched at activation")
end

tests["9 101 a leave during a fetch discards the late result and creates no slot"] = function()
	local w = world()
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Discount"] = 5
	w.start()
	w.backend.targeted[7] = { ["Shop.Discount"] = "nine" }
	w.backend:delay(7, 3)
	local player = w.backend:join(7)
	w.backend:advance(1)
	w.backend:leave(player)
	w.backend:advance(5)
	spec.eq(w.server.configReady(player), false, "no ready flag")
	spec.eq(discount(player), 0, "fallback, no slot")
	spec.eq(w.backend.updateListeners, 1, "the late snapshot is not listened to")
	spec.eq(w.refresh().value, true, "a departed player is not a scope")
	spec.eq(w.backend:count("fetch 7"), 1, "no new fetch for the departed player")
	spec.eq(#w.diagnostics("player 7"), 0, "a departed player's late snapshot is never read")
end

tests["10 100 concurrent refreshConfigs calls make one set of requests"] = function()
	local w = world()
	w.emit.config("Shop.Coins", 1, w.n.f64)
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend:join(7)
	w.start()
	w.backend:setGlobal("Shop.Coins", 2)
	w.backend:delay("global", 1)
	w.backend:delay(7, 1)
	local first = w.refresh()
	local second = w.refresh()
	w.backend:advance(1)
	spec.eq(w.backend:count("refresh global"), 1, "one global refresh")
	spec.eq(w.backend:count("refresh 7"), 1, "one player refresh")
	spec.eq(first.value, true, "first")
	spec.eq(second.done and second.value, true, "the joiner gets the same result")
	w.backend:delay("global", 0)
	w.backend:delay(7, 0)
	w.backend:setGlobal("Shop.Coins", 3)
	local third = w.refresh()
	spec.eq(third.value, true, "a later call starts its own refresh")
	spec.eq(w.backend:count("refresh"), 4, "a later call refreshes both outdated snapshots again")
	spec.eq(w.refresh().value, true, "nothing outdated is current")
	spec.eq(w.backend:count("refresh"), 4, "a current snapshot is not refreshed")
end

tests["11 100 a refresh past its bound returns false keeps values and discards the late result"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	local player = w.backend:join(7)
	w.start()
	w.backend:setGlobal("Shop.Coins", 4)
	w.backend:setGlobal("Shop.Discount", 6)
	w.backend:delay("global", 15)
	local result = w.refresh()
	w.backend:advance(9.99)
	spec.eq(result.done, false, "still inside the bound")
	w.backend:advance(0.01)
	spec.eq(result.value, false, "false at the bound")
	spec.eq(coins(), 3, "late scope keeps its value")
	spec.eq(discount(player), 6, "the scope that answered in time is applied")
	w.backend:advance(5)
	spec.eq(coins(), 3, "the late result is discarded")
	w.backend:delay("global", 0)
	spec.eq(w.refresh().value, true, "the next refresh applies it")
	spec.eq(coins(), 4, "applied")
end

tests["11 100 a failed refresh returns false keeps values and diagnoses"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.start()
	w.backend:setGlobal("Shop.Coins", 4)
	w.backend:failNext("global", "throw")
	spec.eq(w.refresh().value, false, "throwing refresh")
	spec.eq(coins(), 3, "kept")
	w.backend:failNext("global", "load-failed")
	spec.eq(w.refresh().value, false, "LoadFailed refresh")
	spec.eq(coins(), 3, "kept")
	spec.eq(#w.diagnostics("request (global snapshot)"), 1, "diagnosed once in the generation")
	spec.eq(w.refresh().value, true, "a failed snapshot is refreshed again")
	spec.eq(coins(), 4, "applied")
end

tests["99 a refresh commits every scope in one pass"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	local player = w.backend:join(7)
	w.start()
	w.backend:delay(7, 2)
	w.backend:setGlobal("Shop.Coins", 2)
	w.backend:setForPlayer(7, "Shop.Discount", 3)
	local result = w.refresh()
	w.backend:advance(1)
	spec.eq(`{coins()} {discount(player)}`, "1 0", "the answered global scope waits for the player scope")
	w.backend:advance(1)
	spec.eq(`{coins()} {discount(player)} {result.value}`, "2 3 true", "both applied together")
end

tests["101 a leave between staging and commit drops the staged snapshot"] = function()
	local w = world()
	w.emit.config("Shop.Coins", 1, w.n.f64)
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	w.backend:failNext(7, "throw", 1)
	local player = w.backend:join(7)
	w.backend:setGlobal("Shop.Coins", 2)
	w.backend:delay("global", 2)
	local result = w.refresh()
	w.backend:advance(0.5)
	w.backend:leave(player)
	w.backend:advance(2)
	spec.eq(result.value, true, "the departed scope does not count")
	spec.eq(w.backend.updateListeners, 1, "only the global snapshot is listened to")
	spec.eq(w.server.configReady(player), false, "no slot")
end

tests["100 98 staleness follows UpdateAvailable and Outdated independently"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.start()
	w.backend.outdatedOnPublish = false
	w.backend:setGlobal("Shop.Coins", 2)
	spec.eq(coins(), 1, "the event alone changes nothing")
	spec.eq(w.refresh().value, true, "refreshed on the event")
	spec.eq(coins(), 2, "applied without Outdated")
	w.backend.outdatedOnPublish = true
	w.backend.eventOnPublish = false
	w.backend:setGlobal("Shop.Coins", 3)
	spec.eq(w.refresh().value, true, "refreshed on Outdated")
	spec.eq(coins(), 3, "applied without the event")
end

tests["4 emit forms touch no service and no port"] = function()
	local g = spec.runtime({ game = fakeGame({}) })
	g.module("internal/configs/runtime")
	local game = setmetatable({}, {
		__index = function()
			error("an emit form reached an engine service")
		end,
	})
	local h = spec.runtime({ game = game })
	local emit = h.module("emit")
	local coins = emit.config("Shop.Coins", 1, emit.n.f64)
	local discount = emit.playerConfig("Shop.Discount", 0, emit.n.f64)
	spec.eq(coins(), 1, "readable before boot")
	spec.eq(discount({ UserId = 1 }), 0, "player read before boot")
end

tests["14 96 100 global readiness with a delayed first fetch and the payout fallback"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	w.backend.globals.GlobalEvent = DOUBLE
	w.backend:delay("global", 25)
	w.start()
	local function beforeJob()
		return w.server.refreshConfigs() or w.server.configReady()
	end
	local paid = {}
	local function boost(coins, now)
		if not w.server.configReady() then
			w.server.refreshConfigs()
		end
		if not w.server.configReady() then
			table.insert(paid, coins)
			return
		end
		local e = event()
		local live = e.name ~= "" and now >= e.startsAt and now < e.endsAt
		table.insert(paid, if live then coins * e.multipliers.coins else coins)
	end
	local job = {}
	task.spawn(function()
		job.ok = beforeJob()
		boost(100, DOUBLE.startsAt + 1)
	end)
	w.backend:advance(10)
	spec.eq(job.ok, false, "beforeJob is false until the first fetch succeeds")
	spec.eq(event().name, "", "fallback")
	w.backend:advance(10)
	spec.eq(paid[1], 100, "paid without a multiplier after one more bounded refresh")
	w.backend:advance(5)
	spec.eq(w.server.configReady(), true, "the first fetch lands")
	w.backend:delay("global", 0)
	task.spawn(function()
		job.ok = beforeJob()
		boost(100, DOUBLE.startsAt + 1)
	end)
	spec.eq(job.ok, true, "true once loaded")
	spec.eq(paid[2], 200, "the event multiplier applies")
	spec.eq(w.backend.updateListeners, 1, "late first fetches from the bounded refreshes are not listened to")
end

tests["13 the global event server path refreshes every 60 s and keeps the last good event"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	w.backend.globals.GlobalEvent = DOUBLE
	w.start()
	local seen = {}
	task.spawn(function()
		for _ = 1, 3 do
			w.backend:sleep(60)
			w.server.refreshConfigs()
			table.insert(seen, event().endsAt)
		end
	end)
	local later = table.clone(DOUBLE)
	later.endsAt += 3600
	w.backend:advance(30)
	w.backend:setGlobal("GlobalEvent", later)
	w.backend:advance(30)
	local bad = table.clone(DOUBLE)
	bad.endsAt = "Sunday"
	w.backend:setGlobal("GlobalEvent", bad)
	w.backend:advance(60)
	w.backend:advance(60)
	spec.list(seen, { later.endsAt, later.endsAt, later.endsAt }, "new end time, then the last good one kept")
	spec.eq(#w.diagnostics("GlobalEvent.endsAt"), 1, "invalid end time diagnosed once")
end

tests["15 97 an undeclared field is admitted kept and frozen"] = function()
	local w = world()
	local event = w.emit.config("GlobalEvent", NO_EVENT, eventNode(w.n))
	local value = table.clone(DOUBLE)
	value.banner = { color = "gold", sizes = { 1, 2 } }
	w.backend.globals.GlobalEvent = value
	w.start()
	spec.eq(event().banner.color, "gold", "kept in place")
	spec.eq(event().banner.sizes[2], 2, "nested undeclared array kept")
	spec.eq(rawequal(event().banner, value.banner), false, "a copy, never the engine's table")
	spec.eq(deepFrozen(event()), true, "frozen")
end

tests["92 93 declarations admit the fallback and reads never yield throw or fetch"] = function()
	local w = world()
	throws(function()
		w.emit.config("Bad", "text", w.n.f64)
	end, "Bad has an invalid fallback at Bad", "invalid fallback")
	throws(function()
		w.emit.config("Vec", 1, w.n.vector3)
	end, "cannot be a config value", "non-persistable descriptor")
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	local requests = #w.backend.requests
	local thread = coroutine.create(function()
		return coins(), discount({ UserId = 99 }), discount(nil), w.server.configReady({ UserId = 99 })
	end)
	local ok, a, b, c, ready = coroutine.resume(thread)
	spec.eq(ok, true, "reads do not throw")
	spec.eq(coroutine.status(thread), "dead", "reads do not yield")
	spec.eq(`{a} {b} {c} {ready}`, "1 0 0 false", "fallbacks")
	spec.eq(#w.backend.requests, requests, "reads do not fetch")
end

tests["92 2 3 names follow the grammar, length, duplicate and count limits"] = function()
	local w = world()
	local n = w.n
	throws(function()
		w.emit.config("Crucible.Thing", 1, n.f64)
	end, "Crucible.Thing is invalid", "reserved segment")
	throws(function()
		w.emit.config("Bad-Name", 1, n.f64)
	end, "Bad-Name is invalid", "grammar")
	throws(function()
		w.emit.config("a..b", 1, n.f64)
	end, "invalid", "empty segment")
	throws(function()
		w.emit.config(string.rep("a", 65), 1, n.f64)
	end, "65 bytes; the limit is 64", "length")
	w.emit.config(string.rep("b", 64), 1, n.f64)
	w.emit.config("Twice", 1, n.f64)
	throws(function()
		w.emit.playerConfig("Twice", 1, n.f64)
	end, "Twice is declared twice", "duplicate across kinds")
	for i = 3, 1000 do
		w.emit.config(`C{i}`, 1, n.f64)
	end
	throws(function()
		w.emit.config("Last", 1, n.f64)
	end, "limit of 1000", "count")
end

tests["1 92 untransformed macros throw the transformer message"] = function()
	local w = world()
	throws(function()
		w.server.config("A", 1)
	end, "Crucible configs need the @trioforge/crucible/transformer plugin", "config")
	throws(function()
		w.server.playerConfig("A", 1)
	end, "Crucible configs need the @trioforge/crucible/transformer plugin", "playerConfig")
end

tests["3 104 the registry seals at activation and not on a rolled-back boot"] = function()
	local w = world()
	w.emit.config("Early", 1, w.n.f64)
	local failing = setmetatable({}, { __index = w.g.module("subsystem").Subsystem })
	failing.__index = failing
	function failing.new()
		local self = setmetatable({}, failing)
		w.g.module("subsystem").Subsystem.constructor(self)
		return self
	end
	function failing:awake()
		error("boot fails")
	end
	w.emit.defineClass(failing, { kind = "subsystem", id = "Failing", deps = {}, provides = {} })
	local ok = pcall(w.ignite)
	spec.eq(ok, false, "boot rolled back")
	w.backend:advance(1)
	spec.eq(#w.backend.requests, 0, "a rolled-back boot starts no fetch")
	w.emit.config("AfterRollback", 1, w.n.f64)
	local v = world()
	v.emit.config("Early", 1, v.n.f64)
	v.start()
	throws(function()
		v.emit.config("Late", 1, v.n.f64)
	end, "Late was declared after the config registry sealed", "late declaration")
end

tests["104 activation waits once, fetches the global snapshot and connects players only for player configs"] = function()
	local w = world()
	w.emit.config("Shop.Coins", 1, w.n.f64)
	w.ignite()
	spec.eq(#w.backend.requests, 0, "start waits once before fetching")
	w.backend:advance(0)
	spec.eq(w.backend:count("fetch global"), 1, "global fetch at activation")
	spec.eq(w.backend:playerListeners(), 0, "no player listeners without a player config")
	w.backend:join(7)
	spec.eq(w.backend:count("fetch 7"), 0, "no player fetch without a player config")
	local v = world()
	v.emit.playerConfig("Shop.Discount", 0, v.n.f64)
	v.backend:join(1)
	v.start()
	spec.eq(v.backend:playerListeners(), 2, "PlayerAdded and PlayerRemoving connected")
	v.backend:join(2)
	spec.list(v.backend.requests, { "fetch global", "fetch 1", "fetch 2" }, "present then joining players")
end

tests["102 at most four requests run at once and player fetches queue in join order"] = function()
	local w = world()
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	for id = 1, 10 do
		w.backend:delay(id, 1)
	end
	for id = 1, 10 do
		w.backend:join(id)
	end
	spec.eq(w.backend.inFlight, 4, "four in flight")
	w.backend:advance(3)
	spec.eq(w.backend.maxInFlight, 4, "never more than four")
	local order = {}
	for _, request in w.backend.requests do
		table.insert(order, request)
	end
	spec.list(order, {
		"fetch global",
		"fetch 1",
		"fetch 2",
		"fetch 3",
		"fetch 4",
		"fetch 5",
		"fetch 6",
		"fetch 7",
		"fetch 8",
		"fetch 9",
		"fetch 10",
	}, "fair order")
	for id = 1, 10 do
		spec.eq(w.server.configReady(w.backend.players[id]), true, `player {id} ready`)
	end
end

tests["102 a queued fetch for a departed player releases its turn without a request"] = function()
	local w = world()
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	for id = 1, 5 do
		w.backend:delay(id, 1)
		w.backend:join(id)
	end
	w.backend:leave(w.backend.players[5])
	w.backend:advance(2)
	spec.eq(w.backend:count("fetch 5"), 0, "no request for the departed player")
	spec.eq(w.backend.inFlight, 0, "slots released")
	w.backend:join(6)
	spec.eq(w.backend:count("fetch 6"), 1, "a new join is not starved")
end

tests["107 retirement disconnects listeners and keeps the last values"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.backend.globals["Shop.Discount"] = 5
	local player = w.backend:join(7)
	w.start()
	local state = w.runtimeState()
	w.life.request(state, Want.Destroyed)
	w.life.drain()
	w.backend:setGlobal("Shop.Coins", 4)
	spec.eq(w.backend.updateListeners, 0, "snapshot listeners disconnected")
	w.backend:join(8)
	spec.eq(w.backend:playerListeners(), 0, "player listeners disconnected")
	spec.eq(w.backend:count("fetch 8"), 0, "no fetch after retirement")
	local requests = #w.backend.requests
	spec.eq(w.refresh().value, false, "refresh is false")
	spec.eq(#w.backend.requests, requests, "and makes no request")
	spec.eq(coins(), 3, "last value kept")
	spec.eq(discount(player), 5, "player value kept")
	spec.eq(w.server.configReady(), true, "readiness kept")
end

tests["107 a fetch that lands after retirement is discarded"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.backend:delay("global", 2)
	w.start()
	w.life.request(w.runtimeState(), Want.Destroyed)
	w.life.drain()
	w.backend:advance(2)
	spec.eq(coins(), 1, "not applied")
	spec.eq(w.server.configReady(), false, "not ready")
	spec.eq(w.backend.updateListeners, 0, "not listened to")
end

tests["103 4 configs touch only RunService and emit forms touch no service"] = function()
	local w = world()
	w.emit.config("Shop.Coins", 1, w.n.f64)
	spec.list(w.services, { "RunService" }, "only the server entry's side check before boot")
	w.backend:join(1)
	w.start()
	w.backend:setGlobal("Shop.Coins", 2)
	w.refresh()
	spec.list(w.services, { "RunService" }, "no service after activation and refresh")
end

tests["105 106 the default port uses ConfigService only with live data or outside Studio"] = function()
	local function run(studio, attribute)
		local services = {}
		local requested = {}
		local game = {
			GetService = function(_, name)
				table.insert(services, name)
				local s = {
					IsStudio = function()
						return studio
					end,
					GetAttribute = function(_, key)
						return if key == "CrucibleLiveData" then attribute else nil
					end,
					GetConfigAsync = function()
						table.insert(requested, "global")
						error("probe")
					end,
				}
				return s
			end,
		}
		local g = spec.runtime({ game = game, Random = { new = function() end } })
		local port = g.module("internal/configs/port").configPort()
		local ok, snapshot = pcall(port.global)
		return table.find(services, "ConfigService") ~= nil, ok and snapshot.get("x"), #requested
	end
	local live, value, requests = run(true, nil)
	spec.eq(live, false, "Studio without the attribute never reaches ConfigService")
	spec.eq(value, nil, "the in-memory snapshot is empty")
	spec.eq(requests, 0, "no request")
	live, _, requests = run(true, true)
	spec.eq(live and requests, 1, "Studio with CrucibleLiveData uses ConfigService")
	live, _, requests = run(false, nil)
	spec.eq(live and requests, 1, "a live server uses ConfigService")
end

return tests
