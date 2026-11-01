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
	local function none() end
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
			network = { open = none, loaded = none, commit = none, close = none },
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
	local wide = table.clone(DOUBLE)
	for i = 1, 20 do
		wide[`pad${i}`] = string.rep("x", 6000)
	end
	w.backend:setGlobal("GlobalEvent", wide)
	w.refresh()
	spec.eq(#w.diagnostics("over 100000 bytes"), 2, "a decoded table over the byte limit refused")
	spec.eq(event().name, "Text", "kept")
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
	spec.eq(try({ a = {}, m = {}, t = { 1, "x", 3 }, u = 1 }), false, "tuple too long")
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

tests["104 a game without configs never builds the port or fetches"] = function()
	local w = world()
	w.start()
	w.backend:advance(60)
	spec.eq(#w.backend.requests, 0, "no request")
	spec.eq(#w.backend.timers, 0, "start did not wait on the config port")
	spec.eq(w.server.configReady(), false, "nothing to be ready")
	spec.eq(w.refresh().value, false, "nothing loaded")
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

tests["107 a refresh in flight at retirement applies nothing it staged"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	local player = w.backend:join(7)
	w.start()
	w.backend:setGlobal("Shop.Coins", 4)
	w.backend:setForPlayer(7, "Shop.Discount", 6)
	w.backend:delay("global", 2)
	local result = w.refresh()
	w.backend:advance(1)
	w.life.request(w.runtimeState(), Want.Destroyed)
	w.life.drain()
	w.backend:advance(1)
	spec.eq(result.value, false, "false")
	spec.eq(discount(player), 0, "the player scope staged before retirement is not applied")
	spec.eq(coins(), 3, "global kept")
end

tests["103 4 configs touch only RunService and emit forms touch no service"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	spec.list(w.services, { "RunService" }, "only the server entry's side check before boot")
	w.backend:join(1)
	w.start()
	w.backend:setGlobal("Shop.Coins", 2)
	w.refresh()
	spec.eq(coins(), 2, "a read")
	spec.eq(w.server.configReady(), true, "a readiness check")
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

tests["105 the Roblox adapter maps each snapshot to its own ConfigService call"] = function()
	local calls = {}
	local listeners = {}
	local function snapshot(tag, err, outdated)
		return {
			Outdated = outdated,
			Error = err,
			GetValue = function(_, key)
				return `{tag}:{key}`
			end,
			Refresh = function()
				table.insert(calls, `refresh {tag}`)
			end,
			UpdateAvailable = {
				Connect = function(_, listener)
					listeners[tag] = listener
					return {
						Disconnect = function()
							table.insert(calls, `disconnect {tag}`)
						end,
					}
				end,
			},
		}
	end
	local service = {
		GetConfigAsync = function()
			table.insert(calls, "global")
			return snapshot("G", "None", false)
		end,
		GetConfigForPlayerAsync = function(_, player)
			table.insert(calls, `player {player.UserId}`)
			return snapshot(`P{player.UserId}`, if player.UserId == 5 then "LF" else "None", player.UserId == 5)
		end,
	}
	local game = {
		GetService = function(_, name)
			if name == "ConfigService" then
				return service
			end
			return {
				IsStudio = function()
					return false
				end,
			}
		end,
	}
	local g = spec.runtime({
		game = game,
		Random = { new = function() end },
		Enum = { ConfigSnapshotErrorState = { LoadFailed = "LF", None = "None" } },
	})
	local port = g.module("internal/configs/port").configPort()
	local global = port.global()
	local five = port.forPlayer({ UserId = 5 })
	local six = port.forPlayer({ UserId = 6 })
	spec.list(calls, { "global", "player 5", "player 6" }, "the global snapshot and each player's own are requested")
	spec.eq(global.get("k"), "G:k", "the global snapshot's value")
	spec.eq(five.get("k"), "P5:k", "player 5 reads their own snapshot")
	spec.eq(six.get("k"), "P6:k", "player 6 reads their own snapshot")
	spec.eq(global.failed(), false, "None is not a failed load")
	spec.eq(five.failed(), true, "LoadFailed is a failed load")
	spec.eq(six.failed(), false, "None is not a failed load for a player")
	spec.eq(global.outdated(), false, "a current global snapshot")
	spec.eq(five.outdated(), true, "an outdated player snapshot")
	spec.eq(six.outdated(), false, "a current player snapshot")
	five.refresh()
	spec.eq(calls[#calls], "refresh P5", "refresh goes to that snapshot")
	local fired = 0
	local disconnect = six.onUpdateAvailable(function()
		fired += 1
	end)
	listeners.P6()
	spec.eq(fired, 1, "UpdateAvailable reaches the listener")
	disconnect()
	spec.eq(calls[#calls], "disconnect P6", "the disconnect closes that connection")
end

tests["102 requests that arrive while a queue drains never exceed four"] = function()
	local w = world()
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	for id = 1, 6 do
		w.backend:delay(id, 10)
		w.backend:join(id)
	end
	w.backend:advance(10)
	for id = 7, 10 do
		w.backend:delay(id, 100)
		w.backend:join(id)
	end
	spec.eq(w.backend.inFlight, 4, "four in flight")
	spec.eq(w.backend.maxInFlight, 4, "never more than four")
end

tests["102 four departed queued fetches do not stall later fetches"] = function()
	local w = world()
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	for id = 1, 8 do
		w.backend:delay(id, 1)
		w.backend:join(id)
	end
	for _ = 5, 8 do
		w.backend:leave(w.backend.players[5])
	end
	w.backend:advance(2)
	spec.eq(w.backend.inFlight, 0, "every slot is back")
	for id = 9, 12 do
		w.backend:delay(id, 100)
		w.backend:join(id)
	end
	spec.eq(w.backend.inFlight, 4, "four later fetches all run")
end

tests["5 97 numeric ranges bound f32 u16 and u32"] = function()
	local w = world()
	local n = w.n
	local num =
		w.emit.config("Num", { f = 0, h = 0, w = 0 }, n.object({ { "f", n.f32 }, { "h", n.u16 }, { "w", n.u32 } }))
	w.start()
	local function admits(value)
		local before = #w.diagnostics("Num")
		w.backend:setGlobal("Num", value)
		w.refresh()
		return #w.diagnostics("Num") == before
	end
	spec.eq(admits({ f = 3.4028234663852886e38, h = 65535, w = 4294967295 }), true, "every maximum is admitted")
	spec.eq(num().w, 4294967295, "the u32 maximum is stored")
	spec.eq(admits({ f = -3.4028234663852886e38, h = 0, w = 0 }), true, "the f32 minimum is admitted")
	spec.eq(admits({ f = 3.4028236e38, h = 0, w = 0 }), false, "f32 over its range")
	spec.eq(admits({ f = -3.4028236e38, h = 0, w = 0 }), false, "f32 under its range")
	spec.eq(admits({ f = 0, h = 65536, w = 0 }), false, "u16 over its ceiling")
	spec.eq(admits({ f = 0, h = 0, w = 4294967296 }), false, "u32 over its ceiling")
	spec.eq(admits({ f = 0, h = 0, w = -1 }), false, "u32 below zero")
	spec.eq(admits({ f = 0, h = 0, w = 1.5 }), false, "u32 fraction")
	spec.eq(num().f, -3.4028234663852886e38, "the last good value survived every refusal")
end

tests["5 97 variant unions and bare literals select by tag and value"] = function()
	local w = world()
	local n = w.n
	local tagged = w.emit.config(
		"Tagged",
		{ kind = "a", x = 1 },
		n.union({
			n.variant("kind", "a", n.object({ { "x", n.f64 } })),
			n.variant("kind", "b", n.object({ { "y", n.string } })),
		})
	)
	local mode = w.emit.config("Mode", { mode = "on" }, n.object({ { "mode", n.literal("on") } }))
	w.start()
	local function admits(name, value)
		local before = #w.diagnostics(name)
		w.backend:setGlobal(name, value)
		w.refresh()
		return #w.diagnostics(name) == before
	end
	spec.eq(admits("Tagged", { kind = "b", y = "z" }), true, "the second variant")
	spec.eq(tagged().y, "z", "the second variant's body is stored")
	spec.eq(admits("Tagged", { kind = "b", y = 5 }), false, "a variant body is checked")
	spec.eq(admits("Tagged", { kind = "c" }), false, "an unknown tag")
	spec.eq(admits("Tagged", { kind = "a", x = 2 }), true, "the first variant")
	spec.eq(tagged().x, 2, "the first variant's body is stored")
	spec.eq(admits("Mode", { mode = "off" }), false, "a bare literal must match")
	spec.eq(mode().mode, "on", "the literal value survived")
end

tests["5 97 map keys and string sizes are checked"] = function()
	local w = world()
	local n = w.n
	local keyed = w.emit.config("Keyed", {}, n.map(n.union({ n.literal("a"), n.literal("b") }), n.f64))
	local text = w.emit.config("Text", "", n.string)
	w.start()
	local function admits(name, value)
		local before = #w.diagnostics(name)
		w.backend:setGlobal(name, value)
		w.refresh()
		return #w.diagnostics(name) == before
	end
	spec.eq(admits("Keyed", { a = 1, b = 2 }), true, "declared keys")
	spec.eq(keyed().b, 2, "stored")
	spec.eq(admits("Keyed", { a = 1, c = 2 }), false, "a key outside the union")
	spec.eq(keyed().a, 1, "the last good value survived")
	spec.eq(admits("Text", string.rep("a", 65536)), true, "a string at the byte cap")
	spec.eq(#text(), 65536, "stored")
	spec.eq(admits("Text", string.rep("a", 65537)), false, "a string over the byte cap")
	spec.eq(#text(), 65536, "the last good value survived")
end

tests["100 a discarded refresh leaves an outdated-only snapshot stale"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.start()
	w.backend.eventOnPublish = false
	w.backend:setGlobal("Shop.Coins", 5)
	w.backend:delay("global", 15)
	local late = w.refresh()
	w.backend:advance(10)
	spec.eq(late.value, false, "false at the bound")
	spec.eq(coins(), 3, "the value is unchanged")
	w.backend:advance(6)
	w.backend:delay("global", 0)
	local next = w.refresh()
	spec.eq(next.value, true, "the next call refreshes")
	spec.eq(coins(), 5, "and applies the value the discarded refresh fetched")
	spec.eq(w.backend:count("refresh global"), 2, "it asked again")
	spec.eq(w.refresh().value, true, "now current")
	spec.eq(w.backend:count("refresh global"), 2, "and asks no more")
end

tests["100 an UpdateAvailable during a refresh keeps the snapshot stale"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.start()
	w.backend:setGlobal("Shop.Coins", 5)
	w.backend:delay("global", 2)
	local first = w.refresh()
	w.backend:advance(1)
	w.backend:setGlobal("Shop.Coins", 7)
	w.backend:advance(1)
	spec.eq(first.value, false, "an update that arrived mid-refresh leaves the snapshot stale")
	w.backend:delay("global", 0)
	spec.eq(w.refresh().value, true, "the next refresh")
	spec.eq(coins(), 7, "applies the later value")
end

tests["10 100 a joiner shares a false result"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.start()
	w.backend:setGlobal("Shop.Coins", 4)
	w.backend:delay("global", 1)
	w.backend:failNext("global", "throw")
	local first = w.refresh()
	local second = w.refresh()
	w.backend:advance(1)
	spec.eq(first.value, false, "the caller")
	spec.eq(second.done and second.value, false, "the joiner")
	spec.eq(coins(), 3, "values kept")
end

tests["7 94 each retry wait is a fresh random draw of the capped delay"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.backend:failNext("global", "throw", 3)
	w.backend.jitter = 0.5
	w.start()
	spec.eq(w.backend:count("fetch global"), 1, "first attempt")
	w.backend:advance(0.25)
	spec.eq(w.backend:count("fetch global"), 1, "0.5 of the 1 s delay is not over yet")
	w.backend.jitter = 0.25
	w.backend:advance(0.25)
	spec.eq(w.backend:count("fetch global"), 2, "retry after 0.5 s")
	w.backend:advance(0.25)
	spec.eq(w.backend:count("fetch global"), 2, "0.25 of the 2 s delay is not over yet")
	w.backend:advance(0.25)
	spec.eq(w.backend:count("fetch global"), 3, "retry after 0.5 s")
	w.backend:advance(0.75)
	spec.eq(w.backend:count("fetch global"), 3, "0.25 of the 4 s delay is not over yet")
	w.backend:advance(0.25)
	spec.eq(w.backend:count("fetch global"), 4, "retry after 1 s")
	spec.eq(coins(), 3, "applied")
end

tests["7 94 a retry loop stops once its scope is loaded or gone"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.backend:failNext("global", "throw", 1)
	w.start()
	local refreshed = w.refresh()
	spec.eq(refreshed.value, true, "a refresh fetches the unloaded scope")
	spec.eq(coins(), 3, "loaded")
	w.backend:advance(60)
	spec.eq(w.backend:count("fetch global"), 2, "the loop saw it loaded and made no third request")
	w.backend:failNext(7, "throw", 1)
	local player = w.backend:join(7)
	w.backend:leave(player)
	w.backend:advance(100)
	spec.eq(w.backend:count("fetch 7"), 1, "no retry for a departed player")
	spec.eq(#w.backend.timers, 0, "and no loop left sleeping")
end

tests["97 a throwing GetValue keeps the last good value and diagnoses once"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	local label = w.emit.config("Shop.Label", "none", w.n.string)
	w.backend.globals["Shop.Coins"] = 3
	w.backend.globals["Shop.Label"] = "a"
	w.start()
	w.backend:failGet("Shop.Coins")
	w.backend:setGlobal("Shop.Coins", 4)
	w.backend:setGlobal("Shop.Label", "b")
	spec.eq(w.refresh().value, true, "the refresh itself succeeds")
	spec.eq(coins(), 3, "the throwing key keeps its last good value")
	spec.eq(label(), "b", "the other keys still apply")
	spec.eq(#w.diagnostics("GetValue failed"), 1, "diagnosed once")
end

tests["8 97 a player's invalid first targeted value keeps the global value"] = function()
	local w = world()
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	local bonus = w.emit.playerConfig("Shop.Bonus", 0, w.n.f64)
	w.backend.globals["Shop.Discount"] = 5
	w.backend.targeted[7] = { ["Shop.Discount"] = "nine", ["Shop.Bonus"] = "ten" }
	w.start()
	local player = w.backend:join(7)
	spec.eq(w.server.configReady(player), true, "the snapshot was applied")
	spec.eq(discount(player), 5, "the global value stays")
	spec.eq(bonus(player), 0, "the fallback when no global value exists")
	spec.eq(#w.diagnostics("player 7"), 2, "each invalid key diagnosed once")
	w.backend:setForPlayer(7, "Shop.Discount", 9)
	spec.eq(w.refresh().value, true, "refresh")
	spec.eq(discount(player), 9, "a later valid targeted value replaces it")
end

tests["97 a later invalid targeted value keeps that player's last good targeted value"] = function()
	local w = world()
	local discount = w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend.globals["Shop.Discount"] = 5
	w.backend.targeted[7] = { ["Shop.Discount"] = 9 }
	w.start()
	local player = w.backend:join(7)
	local other = w.backend:join(8)
	spec.eq(discount(player), 9, "the targeted value")
	spec.eq(discount(other), 5, "an untargeted player reads the global value")
	w.backend:setForPlayer(7, "Shop.Discount", "nine")
	spec.eq(w.refresh().value, true, "refresh")
	spec.eq(discount(player), 9, "the player's own last good value, not the global one")
	spec.eq(discount(other), 5, "the other player is unchanged")
	spec.eq(#w.diagnostics("player 7"), 1, "the invalid value is diagnosed once")
end

tests["97 an unchanged player table value keeps its identity across a refresh"] = function()
	local w = world()
	local box = w.emit.playerConfig("Shop.Box", { a = 1 }, w.n.object({ { "a", w.n.f64 } }))
	w.backend.globals["Shop.Box"] = { a = 5 }
	w.backend.targeted[7] = { ["Shop.Box"] = { a = 9 } }
	w.start()
	local player = w.backend:join(7)
	local before = box(player)
	spec.eq(before.a, 9, "the targeted table")
	w.backend:setForPlayer(7, "Shop.Other", 1)
	spec.eq(w.refresh().value, true, "refresh")
	spec.eq(rawequal(before, box(player)), true, "the same table")
end

tests["100 a refresh that throws after a discarded one reports false on stale values"] = function()
	local w = world()
	local coins = w.emit.config("Shop.Coins", 1, w.n.f64)
	w.backend.globals["Shop.Coins"] = 3
	w.start()
	w.backend.eventOnPublish = false
	w.backend:setGlobal("Shop.Coins", 5)
	w.backend:delay("global", 15)
	local late = w.refresh()
	w.backend:advance(10)
	spec.eq(late.value, false, "false at the bound")
	w.backend:advance(6)
	w.backend:delay("global", 0)
	w.backend:failNext("global", "throw", 1)
	spec.eq(w.refresh().value, false, "the throwing refresh reports false")
	spec.eq(coins(), 3, "the value is still the old one")
	spec.eq(w.refresh().value, true, "the next refresh recovers")
	spec.eq(coins(), 5, "and applies the fetched value")
end

tests["3 104 a declaration between ignite and the first advance throws"] = function()
	local w = world()
	w.emit.config("A", 1, w.n.f64)
	w.ignite()
	throws(function()
		w.emit.config("B", 1, w.n.f64)
	end, "B was declared after the config registry sealed", "declaration inside the boot yield")
end

tests["3 104 a declaration after a configless boot throws and a rolled-back boot leaves the registry open"] = function()
	local w = world()
	w.start()
	w.backend:advance(5)
	throws(function()
		w.emit.config("Late", 1, w.n.f64)
	end, "Late was declared after the config registry sealed", "late declaration")
	local v = world()
	local failing = setmetatable({}, { __index = v.g.module("subsystem").Subsystem })
	failing.__index = failing
	function failing.new()
		local self = setmetatable({}, failing)
		v.g.module("subsystem").Subsystem.constructor(self)
		return self
	end
	function failing:start()
		error("boot fails")
	end
	v.emit.defineClass(failing, { kind = "subsystem", id = "Failing", deps = {}, provides = {} })
	local ok = pcall(v.ignite)
	spec.eq(ok, false, "boot rolled back")
	v.emit.config("AfterRollback", 1, v.n.f64)
end

tests["104 a duplicate PlayerAdded for a present player makes one scope"] = function()
	local w = world()
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	local player = w.backend:join(7)
	w.start()
	w.backend.joined.fire(player)
	spec.eq(w.backend:count("fetch 7"), 1, "one fetch")
	spec.eq(w.backend.updateListeners, 2, "one listener each for the global snapshot and the player's")
end

tests["107 retirement disconnects at once, before any event or read"] = function()
	local w = world()
	w.emit.config("Shop.Coins", 1, w.n.f64)
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.backend:join(7)
	w.start()
	spec.eq(w.backend.updateListeners, 2, "listening before retirement")
	w.life.request(w.runtimeState(), Want.Destroyed)
	w.life.drain()
	spec.eq(w.backend.updateListeners, 0, "snapshot listeners")
	spec.eq(w.backend:playerListeners(), 0, "player listeners")
end

tests["107 a queued fetch is dropped at retirement without a request"] = function()
	local w = world()
	w.emit.playerConfig("Shop.Discount", 0, w.n.f64)
	w.start()
	for id = 1, 5 do
		w.backend:delay(id, 10)
		w.backend:join(id)
	end
	w.life.request(w.runtimeState(), Want.Destroyed)
	w.life.drain()
	w.backend:advance(10)
	spec.eq(w.backend:count("fetch 5"), 0, "the queued fetch makes no request")
	spec.eq(w.backend.inFlight, 0, "nothing left in flight")
end

tests["100 a duplicate first result is discarded and its listener disconnected"] = function()
	local loopFirst = world()
	local coins = loopFirst.emit.config("Shop.Coins", 1, loopFirst.n.f64)
	loopFirst.backend.globals["Shop.Coins"] = 3
	loopFirst.backend:delay("global", 5)
	loopFirst.start()
	loopFirst.backend:advance(1)
	local refreshed = loopFirst.refresh()
	loopFirst.backend:advance(4)
	spec.eq(coins(), 3, "the loop's fetch applied")
	spec.eq(loopFirst.backend.updateListeners, 1, "one listener")
	loopFirst.backend:advance(1)
	spec.eq(refreshed.value, true, "the refresh's own fetch found the scope loaded")
	spec.eq(loopFirst.backend.updateListeners, 1, "the later duplicate does not listen")

	local refreshFirst = world()
	refreshFirst.emit.config("Shop.Coins", 1, refreshFirst.n.f64)
	refreshFirst.backend:delay("global", 5)
	refreshFirst.start()
	refreshFirst.backend:advance(1)
	refreshFirst.backend:delay("global", 1)
	local result = refreshFirst.refresh()
	refreshFirst.backend:advance(1)
	spec.eq(result.value, true, "the refresh applied the first snapshot")
	spec.eq(refreshFirst.backend.updateListeners, 1, "one listener")
	refreshFirst.backend:advance(3)
	spec.eq(refreshFirst.backend.updateListeners, 1, "the loop's late duplicate does not listen")
end

return tests
