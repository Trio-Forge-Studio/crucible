local spec = require("./lib/load")
local Backend = require("./lib/data-backend")
local R = require("./lib/data-records")

local WALL = R.WALL
local SEALED = "was declared after the data registry sealed; declare stores in modules that boot loads"
local PLUGIN = "Crucible data needs the @trioforge/crucible/transformer plugin"

local function noop(current)
	return current
end

local function world()
	local backend = Backend.new()
	local realm = backend:realm()
	local n = realm.n
	local w = {
		backend = backend,
		realm = realm,
		n = n,
		emit = realm.graph.module("emit"),
		records = realm.graph.module("internal/data/record-declarations"),
		bindings = realm.graph.module("internal/data/record-bindings"),
		node = n.object({ { "count", n.f64 }, { "label", n.union({ n["nil"], n.string }) } }),
	}
	function w.declare(name: string, defaults, migrations)
		return w.emit.dataStore(name, { defaults = defaults or { count = 0 }, migrations = migrations }, w.node)
	end
	function w.bound(name: string, migrations)
		local handle = w.declare(name, { count = 0 }, migrations)
		w.bindings.bindRecords(realm.data)
		return handle
	end
	return w
end

local function fields(value): { string }
	local list = {}
	for key in value do
		table.insert(list, tostring(key))
	end
	table.sort(list)
	return list
end

local function run(w, fn, seconds: number?)
	local handle = w.realm.run(fn)
	w.backend:advance(seconds or 1)
	return handle
end

local FAILURES = {
	{
		reason = "timeout",
		provoke = function(w, handle)
			w.realm.budget.update = 0
			return run(w, function()
				return handle:update("k", noop, R.id(WALL - 5, "p1"))
			end, 31)
		end,
	},
	{
		reason = "uncertain",
		provoke = function(w, handle)
			w.backend:gate("response", { kind = "update" })
			return run(w, function()
				return handle:update("k", noop, R.id(WALL - 5, "p2"))
			end, 31)
		end,
	},
	{
		reason = "overloaded",
		provoke = function(w, handle)
			w.realm.budget.update = 0
			for index = 1, 32 do
				w.realm.run(function()
					return handle:update("k", noop, R.id(WALL - 5, `q{index}`))
				end)
			end
			return run(w, function()
				return handle:update("k", noop, R.id(WALL - 5, "q33"))
			end, 0)
		end,
	},
	{
		reason = "closing",
		provoke = function(w, handle)
			w.realm.run(function()
				w.realm.shutdown.shutdownData(w.realm.data)
			end)
			w.backend:advance(0.6)
			return run(w, function()
				return handle:update("k", noop, R.id(WALL - 5, "p3"))
			end)
		end,
	},
	{
		reason = "expired",
		provoke = function(w, handle)
			return run(w, function()
				return handle:update("k", noop, R.id(WALL - 4000, "p4"))
			end)
		end,
	},
	{
		reason = "unavailable",
		provoke = function(w, handle)
			w.backend:fail("reject", { kind = "update" }, 1, { code = 403 })
			return run(w, function()
				return handle:update("k", noop, R.id(WALL - 5, "p5"))
			end)
		end,
	},
}

return {
	["rule 4 dataStore returns a frozen handle with read and update and no other field"] = function()
		local w = world()
		local handle = w.declare("Decl.Handle")
		spec.eq(table.isfrozen(handle), true, "frozen")
		spec.list(fields(handle), { "read", "update" }, "its fields")
		spec.eq(type(handle.read), "function", "read")
		spec.eq(type(handle.update), "function", "update")
		spec.eq(getmetatable(handle), nil, "no metatable")
	end,
	["rule 4 defaults are admitted against the descriptor and frozen at evaluation"] = function()
		local w = world()
		local passed = { count = 5, kept = { deep = 1 } }
		w.declare("Decl.Defaults", passed)
		local defaults = w.records.declaredRecords()["Decl.Defaults"].schema.defaults
		R.same(defaults, { count = 5, kept = { deep = 1 } }, "admitted with unknown keys in place")
		R.frozen(defaults, "the defaults")
		spec.eq(rawequal(defaults, passed), false, "a copy of what the caller passed")
		spec.eq(table.isfrozen(passed), false, "the caller's table is left alone")
	end,
	["rule 4 change gets frozen defaults for a missing key and mutating them fails"] = function()
		local w = world()
		local handle = w.bound("Decl.Frozen")
		local seen, mutated
		local result = run(w, function()
			return handle:update("k", function(current)
				seen = current
				mutated = pcall(function()
					current.count = 9
				end)
				return current
			end, R.id(WALL - 5, "d1"))
		end)
		spec.eq(result.value.ok, true, "ok")
		R.same(seen, { count = 0 }, "the defaults")
		R.frozen(seen, "the defaults")
		spec.eq(mutated, false, "mutating them throws")
		R.same(
			R.stored(w.backend, "k", "Decl.Frozen"),
			R.envelope({ schemaVersion = 0, operations = { R.id(WALL - 5, "d1") } }),
			"a no-op stores the id only"
		)
	end,
	["rule 4 migrations set the schema version a changed record is stored at"] = function()
		local w = world()
		local steps = {
			function()
				return {}
			end,
			function()
				return {}
			end,
			function()
				return {}
			end,
		}
		local handle = w.bound("Decl.Versioned", steps)
		local id = R.id(WALL - 5, "v1")
		run(w, function()
			return handle:update("k", function()
				return { count = 4 }
			end, id)
		end)
		R.same(
			R.stored(w.backend, "k", "Decl.Versioned"),
			R.envelope({ schemaVersion = 3, payload = { count = 4 }, operations = { id } }),
			"stored at the step count"
		)
	end,
	["rule 4 defaults that violate the descriptor throw naming the store and path, and the name stays free"] = function()
		local w = world()
		local ok, err = pcall(w.declare, "Decl.Bad", { count = R.SECRET })
		spec.eq(ok, false, "a wrong type throws")
		spec.has(err, "Decl.Bad.count", "naming the store and path")
		spec.eq(string.find(err, R.SECRET, 1, true), nil, "never the value")
		ok, err = pcall(w.declare, "Decl.Bad", {})
		spec.eq(ok, false, "a missing required field throws")
		spec.has(err, "Decl.Bad.count", "naming the store and path")
		local good = pcall(w.declare, "Decl.Bad", { count = 1 })
		spec.eq(good, true, "a failed declaration did not register the name")
	end,
	["rule 3 a duplicate name throws naming it"] = function()
		local w = world()
		w.declare("Dup.Store")
		local ok, err = pcall(w.declare, "Dup.Store")
		spec.eq(ok, false, "throws")
		spec.has(err, "Dup.Store", "naming the name")
		spec.eq(pcall(w.declare, "Dup.Other"), true, "another name is fine")
	end,
	["rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first"] = function()
		local w = world()
		local bad = { "", "1a", "a..b", ".a", "a.", "a b", "a-b", "a.1b", "a.b!", "Crucible", "Crucible.Data" }
		for _, name in bad do
			local ok, err = pcall(w.declare, name)
			spec.eq(ok, false, `{name} is refused`)
			spec.has(err, name, `{name} is named`)
		end
		for _, name in { "A", "a_b.C1", "X.Y.Z", "FD.Players2", "Crucibles.A", "A.Crucible" } do
			spec.eq(pcall(w.declare, name), true, `{name} is accepted`)
		end
	end,
	["rule 2 a store name is at most 50 bytes"] = function()
		local w = world()
		local fifty = "A" .. string.rep("b", 49)
		local long = fifty .. "c"
		spec.eq(pcall(w.declare, fifty), true, "50 bytes is accepted")
		local ok, err = pcall(w.declare, long)
		spec.eq(ok, false, "51 bytes is refused")
		spec.has(err, long, "naming the name")
	end,
	["rule 3 a realm holds at most 256 data declarations"] = function()
		local w = world()
		for index = 1, 256 do
			w.declare(`Cap.S{index}`)
		end
		local ok, err = pcall(w.declare, "Cap.S257")
		spec.eq(ok, false, "the 257th throws")
		spec.has(err, "Cap.S257", "naming it")
		local other = world()
		spec.eq(pcall(other.declare, "Cap.S1"), true, "another realm has its own registry")
	end,
	["rule 3 a declaration after the registry seals throws and sealing off allows it again"] = function()
		local w = world()
		local early = w.declare("Seal.Early")
		w.records.sealDeclarations(true)
		local ok, err = pcall(w.declare, "Seal.Late")
		spec.eq(ok, false, "throws once sealed")
		spec.has(err, `Seal.Late {SEALED}`, "with the rule's text")
		spec.eq(table.isfrozen(early), true, "the earlier handle is unaffected")
		w.records.sealDeclarations(false)
		spec.eq(pcall(w.declare, "Seal.Late"), true, "unsealed, the declaration is accepted")
	end,

	["rule 104 a call before binding waits for activation and returns timeout at the 30 second deadline"] = function()
		local w = world()
		local handle = w.declare("Wait.Store")
		local runs = 0
		local read = w.realm.run(function()
			return handle:read("k")
		end)
		local update = w.realm.run(function()
			return handle:update("k", function(current)
				runs += 1
				return current
			end)
		end)
		w.backend:advance(29.9)
		spec.eq(read.done or update.done, false, "still waiting before the deadline")
		w.backend:advance(0.2)
		spec.eq(read.done and update.done, true, "both returned at the deadline")
		R.same(read.value, { ok = false, reason = "timeout" }, "read")
		R.same(update.value, { ok = false, reason = "timeout" }, "update")
		spec.eq(runs, 0, "change never ran")
		spec.eq(#w.backend.calls, 0, "nothing was sent")
	end,
	["rule 104 a call before binding completes normally when binding happens within its deadline"] = function()
		local w = world()
		local handle = w.declare("Wait.Bound")
		local id = R.id(WALL - 5, "w1")
		local update = w.realm.run(function()
			return handle:update("k", function()
				return { count = 3 }
			end, id)
		end)
		w.backend:advance(10)
		spec.eq(update.done, false, "waiting at 10 seconds")
		w.bindings.bindRecords(w.realm.data)
		w.backend:advance(1)
		spec.eq(update.done, true, "returned after binding")
		R.same(update.value, { ok = true, value = { count = 3 }, replayed = false }, "the public update shape")
		R.same(R.stored(w.backend, "k", "Wait.Bound").payload, { count = 3 }, "and it wrote")
	end,
	["rule 104 after binding read and update return the public shapes"] = function()
		local w = world()
		local handle = w.bound("Shape.Store")
		local absent = run(w, function()
			return handle:read("k")
		end)
		spec.list(fields(absent.value), { "ok" }, "an absent key reads ok with no value")
		spec.eq(absent.value.ok, true, "ok")
		local id = R.id(WALL - 5, "h1")
		local update = run(w, function()
			return handle:update("k", function()
				return { count = 2 }
			end, id)
		end)
		spec.list(fields(update.value), { "ok", "replayed", "value" }, "an update's fields")
		R.same(update.value, { ok = true, value = { count = 2 }, replayed = false }, "an update")
		local replay = run(w, function()
			return handle:update("k", noop, id)
		end)
		R.same(replay.value, { ok = true, value = { count = 2 }, replayed = true }, "a replay")
		local read = run(w, function()
			return handle:read("k")
		end)
		spec.list(fields(read.value), { "ok", "value" }, "a read's fields")
		R.same(read.value, { ok = true, value = { count = 2 } }, "a read")
	end,
	["rule 90 every failure kind maps to its reason string in the public shape"] = function()
		for _, failure in FAILURES do
			local w = world()
			local handle = w.bound("Fail.Store")
			local result = failure.provoke(w, handle)
			spec.eq(result.done, true, `{failure.reason} returned`)
			spec.list(fields(result.value), { "ok", "reason" }, `{failure.reason} fields`)
			spec.eq(result.value.ok, false, `{failure.reason} is not ok`)
			spec.eq(result.value.reason, failure.reason, "the reason string")
		end
		local w = world()
		local handle = w.bound("Fail.Read")
		R.seed(w.backend, "bad", { kind = "profiles" }, "Fail.Read")
		local read = run(w, function()
			return handle:read("bad")
		end)
		R.same(read.value, { ok = false, reason = "unavailable" }, "a read of an unreadable record")
	end,

	["rule 1 the untransformed public dataStore throws the plugin message"] = function()
		local w = world()
		local server = w.realm.graph.module("server/data")
		local ok, err = pcall(server.dataStore, "Plain.Store", { defaults = {} })
		spec.eq(ok, false, "throws")
		spec.has(err, PLUGIN, "with the rule's text")
		spec.eq(w.records.declaredRecords()["Plain.Store"], nil, "and registers nothing")
	end,
	["rule 66 operationId makes o1, the base 36 wall second and an alphanumeric token within 64 bytes"] = function()
		local w = world()
		local server = w.realm.graph.module("server/data")
		w.backend:advance(12.7)
		local first, second = server.operationId(), server.operationId()
		local seconds, token = string.match(first, "^o1%-(%w+)%-(%w+)$")
		spec.eq(seconds ~= nil, true, `{first} has the id format`)
		spec.eq(R.decode36(seconds), WALL + 12, "the prefix is the realm's wall second")
		spec.eq(token, "server1token1", "the token is the platform GUID without its dashes")
		spec.eq(#first <= 64, true, "within 64 bytes")
		spec.eq(first ~= second, true, "two calls differ")
		spec.eq(string.match(second, "^o1%-%w+%-(%w+)$"), "server1token2", "the next token")
		spec.eq(string.lower(first), first, "lower case")
		local handle = w.bound("Id.Store")
		local result = run(w, function()
			return handle:update("k", noop, first)
		end)
		spec.eq(result.value.ok, true, "the store accepts it")
		spec.eq(result.value.replayed, false, "as a new id")
	end,
}
