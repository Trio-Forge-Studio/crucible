local spec = require("./lib/load")
local D2 = require("./lib/data-d2")
local DM = require("./lib/data-migrations")

local n = DM.n
local STORE = DM.STORE
local USER = DM.USER
local SECRET = "SECRET-payload-value"

local JOB = DM.object({ id = n.string, tips = n.f64 })
local ROOT = DM.object({
	bonus = n.map(n.string, DM.object({ n = n.f64 })),
	career = DM.object({ jobs = n.array(JOB), rank = n.f64 }),
	coins = n.f64,
	mode = n.union({
		n.variant("kind", "A", DM.object({ a = n.f64, shared = n.f64 })),
		n.variant("kind", "B", DM.object({ b = n.f64, shared = n.f64 })),
	}),
	nickname = DM.opt(n.string),
	profile = DM.opt(DM.object({ bio = n.string, level = n.f64 })),
	settings = DM.object({ vibration = n.bool, volume = n.f64 }),
})

local function defaults()
	return {
		bonus = { k = { n = 1 } },
		career = { jobs = { { id = "d", tips = 0 } }, rank = 3 },
		coins = 5,
		mode = { kind = "A", a = 1, shared = 9 },
		nickname = "def",
		profile = { bio = "d", level = 2 },
		settings = { vibration = true, volume = 0.5 },
	}
end

local function valid()
	return {
		bonus = { k = { n = 7 } },
		career = { jobs = { { id = "j1", tips = 1 }, { id = "j2", tips = 2 } }, rank = 4 },
		coins = 9,
		mode = { kind = "B", b = 3, shared = 4 },
		nickname = "nick",
		profile = { bio = "b", level = 6 },
		settings = { vibration = false, volume = 0.25 },
	}
end

local function seeded(count: number?, fields, shape, base)
	local backend, a, b = D2.world(count or 1, { defaults = base or defaults(), version = shape or { node = ROOT } })
	local seed = { schemaVersion = 0, payload = fields, epoch = 5 }
	D2.seed(backend, nil, seed)
	return backend, a, b, seed
end

-- Loads a seeded record and expects it to be refused with the unreadable-record message and the data untouched.
local function refused(label: string, backend, a, seed, path: string, reason: string, leak: string?)
	local player, handle = DM.begin(a)
	backend:advance(1)
	DM.assertRefused(backend, a, player, handle, a.limits.LOAD_FAILED_MESSAGE, seed, label)
	local messages = DM.logged(a, "did not load")
	spec.eq(#messages, 1, `{label}: one diagnostic`)
	spec.has(messages[1], `{STORE}{path}`, `{label}: path`)
	spec.has(messages[1], reason, `{label}: reason`)
	spec.has(messages[1], tostring(USER), `{label}: names the user`)
	spec.eq(a.engine.logged(SECRET), 0, `{label}: no record value in any diagnostic`)
	if leak then
		spec.eq(a.engine.logged(leak), 0, `{label}: nor the offending value`)
	end
end

-- One field `x` under `node`, defaulting to `default`, holding `stored`.
local function single(node, default, stored, extra)
	local payload = { x = stored, secret = SECRET }
	for key, value in extra or {} do
		payload[key] = value
	end
	return seeded(1, payload, { node = DM.object({ x = node, secret = n.string }) }, { x = default, secret = "" })
end

local function accepted(label: string, node, default, stored)
	local backend, a = single(node, default, stored)
	local player, handle = DM.begin(a)
	backend:advance(1)
	spec.eq(handle.value, true, `{label}: loads`)
	return DM.session(a, player).value.x, a, player
end

local function starts(text: string, prefix: string): boolean
	return string.sub(text, 1, #prefix) == prefix
end

local function arrayChain(levels: number)
	local root = {}
	local cursor = root
	for _ = 2, levels do
		local child = {}
		cursor[1] = child
		cursor = child
	end
	return root
end

local REJECTED = {
	{ "a boolean holding a string", n.bool, true, "hunter2", ".x", "expected boolean, got string", "hunter2" },
	{ "a number holding a string", n.f64, 0, "12abc", ".x", "expected number, got string", "12abc" },
	{ "NaN", n.f64, 0, 0 / 0, ".x", "expected a finite number" },
	{ "infinity", n.f64, 0, math.huge, ".x", "expected a finite number" },
	{ "negative infinity", n.f64, 0, -math.huge, ".x", "expected a finite number" },
	{ "a u8 over 255", n.u8, 0, 256, ".x", "expected an integer in [0, 255]" },
	{ "a negative u8", n.u8, 0, -1, ".x", "expected an integer in [0, 255]" },
	{ "a fractional u8", n.u8, 0, 1.5, ".x", "expected an integer in [0, 255]" },
	{ "a u16 over 65535", n.u16, 0, 65536, ".x", "expected an integer in [0, 65535]" },
	{ "a negative u16", n.u16, 0, -1, ".x", "expected an integer in [0, 65535]" },
	{ "a fractional u16", n.u16, 0, 0.5, ".x", "expected an integer in [0, 65535]" },
	{ "a u32 over 4294967295", n.u32, 0, 4294967296, ".x", "expected an integer in [0, 4294967295]" },
	{ "a negative u32", n.u32, 0, -1, ".x", "expected an integer in [0, 4294967295]" },
	{ "an f32 over its range", n.f32, 0, 3.5e38, ".x", "out of f32 range" },
	{ "an f32 under its range", n.f32, 0, -3.5e38, ".x", "out of f32 range" },
	{ "a string over 65536 bytes", n.string, "", string.rep("a", 65537), ".x", "a string over 65536 bytes" },
	{ "invalid UTF-8", n.string, "", "ab" .. string.char(255), ".x", "invalid UTF-8" },
	{ "a truncated UTF-8 sequence", n.string, "", "ab" .. string.char(195), ".x", "invalid UTF-8" },
	{ "an array with a hole", n.array(n.f64), {}, { [1] = 1, [3] = 3 }, ".x", "expected an array" },
	{ "an object where an array is declared", n.array(n.f64), {}, { key = 1 }, ".x", "expected an array" },
	{
		"an array where an object is declared",
		DM.object({ y = n.f64 }),
		{ y = 0 },
		{ 1, 2 },
		".x",
		"expected an object",
	},
	{ "a string element in a number array", n.array(n.f64), {}, { 1, 2, "x" }, ".x[2]", "expected number, got string" },
}

return {
	["rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched"] = function()
		for _, case in REJECTED do
			local name, node, default, stored, path, reason, leak = table.unpack(case)
			local backend, a, _, seed = single(node, default, stored)
			refused(name, backend, a, seed, path, reason, leak)
		end
	end,
	["rule 76 the width and length boundaries themselves are accepted, and nothing is coerced"] = function()
		spec.eq((accepted("u8 255", n.u8, 0, 255)), 255, "u8 255")
		spec.eq((accepted("u8 0", n.u8, 1, 0)), 0, "u8 0")
		spec.eq((accepted("u16 65535", n.u16, 0, 65535)), 65535, "u16 65535")
		spec.eq((accepted("u32 4294967295", n.u32, 0, 4294967295)), 4294967295, "u32 4294967295")
		spec.eq((accepted("f32 max", n.f32, 0, 3.4028234663852886e38)), 3.4028234663852886e38, "f32 max")
		spec.eq((accepted("f32 min", n.f32, 0, -3.4028234663852886e38)), -3.4028234663852886e38, "f32 min")
		spec.eq(#accepted("string of 65536", n.string, "", string.rep("a", 65536)), 65536, "string of 65536 bytes")
		local greeting = utf8.char(104, 233, 108, 108, 111, 32, 10003)
		spec.eq((accepted("utf-8", n.string, "", greeting)), greeting, "valid UTF-8")
		spec.eq((accepted("zero is not reset", n.f64, 5, 0)), 0, "a present zero stays zero, not the default")
		spec.eq((accepted("false is not reset", n.bool, true, false)), false, "a present false stays false")
		spec.eq((accepted("empty string is not reset", n.string, "d", "")), "", "a present empty string stays empty")
	end,
	["rule 76 a mixed array and object table, and a hole in an undeclared array, are refused at their path"] = function()
		local mixed = { 1, 2 }
		mixed.name = "x"
		local payload = { extra = mixed, secret = SECRET }
		local backend, a, _, seed = seeded(1, payload, { node = DM.object({ secret = n.string }) }, { secret = "" })
		refused("mixed", backend, a, seed, ".extra", "a table mixing array and object keys", "name")
		local holed = { nest = { list = { [1] = 1, [3] = 3 } }, secret = SECRET }
		local second, b, _, seed2 = seeded(1, holed, { node = DM.object({ secret = n.string }) }, { secret = "" })
		refused("hole", second, b, seed2, ".nest.list", "a table mixing array and object keys")
	end,
	["rule 76 depth 28 loads and depth 29 does not, in objects and in arrays"] = function()
		for _, build in { "objects", "arrays" } do
			local make = if build == "objects" then DM.nest else arrayChain
			local ok = { secret = SECRET, chain = make(27) }
			local backend, a = seeded(1, ok, { node = DM.object({ secret = n.string }) }, { secret = "" })
			local player, handle = DM.begin(a)
			backend:advance(1)
			spec.eq(handle.value, true, `{build}: 28 tables deep, the root included, load`)
			spec.eq(DM.session(a, player).value.secret, SECRET, `{build}: loaded`)

			local deep = { secret = SECRET, chain = make(28) }
			local second, b, _, seed = seeded(1, deep, { node = DM.object({ secret = n.string }) }, { secret = "" })
			refused(
				`{build}: 29 deep`,
				second,
				b,
				seed,
				".chain" .. string.rep(if build == "objects" then ".deep" else "[0]", 27),
				"deeper than 28"
			)
		end
	end,
	["rule 76 100000 values load and 100001 do not, counting the root and every table"] = function()
		local ok = { secret = SECRET, list = table.create(99997, 1) }
		local backend, a = seeded(1, ok, { node = DM.object({ secret = n.string }) }, { secret = "" })
		local player, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "root, secret, list and 99,997 numbers is exactly 100,000 values")
		spec.eq(#DM.session(a, player).value.list, 99997, "loaded whole")

		local over = { secret = SECRET, list = table.create(99998, 1) }
		local second, b, _, seed = seeded(1, over, { node = DM.object({ secret = n.string }) }, { secret = "" })
		refused("one over", second, b, seed, ".list[", "more than 100000 values")
	end,
	["rule 76 a table with a metatable and a cycle are refused at the engine boundary"] = function()
		local backend, a = seeded(1, valid(), nil)
		local store = a.store
		local meta = a.admission.loadRecord(store.schema, setmetatable({}, { __index = {} }), 0)
		spec.eq(meta.kind, "unreadable", "a metatable")
		spec.has(meta.reason, "a table with a metatable", "named")
		spec.has(meta.reason, STORE, "with the store")
		local loop = { name = SECRET }
		loop.self = loop
		local cycle = a.admission.loadRecord(store.schema, loop, 0)
		spec.eq(cycle.kind, "unreadable", "a cycle")
		spec.has(cycle.reason, "a cycle", "named")
		spec.has(cycle.reason, `{STORE}.self`, "at its path")
		spec.eq(string.find(cycle.reason, SECRET, 1, true), nil, "without the value")
		backend:advance(0)
	end,
	["rule 76 defaults fill a missing required key along object fields from the root, shared by identity"] = function()
		local payload = valid()
		payload.coins = nil
		payload.career = nil
		payload.settings = { vibration = false }
		local backend, a = seeded(1, payload)
		local player, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "loads")
		local session = DM.session(a, player)
		local value = session.value
		spec.eq(value.coins, 5, "a missing root key takes its default")
		spec.eq(
			rawequal(value.career, a.store.schema.defaults.career),
			true,
			"a missing object is the defaults' own table"
		)
		spec.eq(value.career.rank, 3, "with its contents")
		spec.eq(value.settings.vibration, false, "a stored value beside a missing one is kept")
		spec.eq(value.settings.volume, 0.5, "the missing key two objects deep takes the default at that path")
		spec.eq(session.revision, 0, "filling a default is not a migration")
	end,
	["rule 76 inside a union only the default's own variant fills a missing key"] = function()
		local own = valid()
		own.mode = { kind = "A", shared = 4 }
		local backend, a = seeded(1, own)
		local player, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "the default's variant loads")
		spec.eq(DM.session(a, player).value.mode.a, 1, "and takes the default's key")

		local other = valid()
		other.mode = { kind = "B", b = 3 }
		local second, b, _, seed = seeded(1, other)
		refused("the other variant", second, b, seed, ".mode.shared", "migration step")
	end,
	["rule 76 maps and arrays never invent entries and defaults never reach their elements"] = function()
		local empty = valid()
		empty.bonus = {}
		empty.career.jobs = {}
		local backend, a = seeded(1, empty)
		local player, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "loads")
		local value = DM.session(a, player).value
		spec.eq(next(value.bonus), nil, "the map stays empty though defaults hold k")
		spec.eq(#value.career.jobs, 0, "the array stays empty though defaults hold an element")

		local element = valid()
		element.career.jobs = { { id = "j1", tips = 1 }, { id = "j2" } }
		local second, b, _, seed = seeded(1, element)
		refused("array element", second, b, seed, ".career.jobs[1].tips", "migration step")
		local entry = valid()
		entry.bonus = { k = {} }
		local third, c, _, seed3 = seeded(1, entry)
		refused("map value", third, c, seed3, ".bonus.k.n", "migration step")
	end,
	["rule 76 R5 an absent optional object stays absent and a stored one takes the default at its path only when defaults hold one"] = function()
		local absent = valid()
		absent.profile = nil
		local backend, a = seeded(1, absent)
		local player, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "loads")
		spec.eq(DM.session(a, player).value.profile, nil, "absent although defaults hold a profile")

		local partial = valid()
		partial.profile = { bio = "b" }
		local second, b = seeded(1, partial)
		local pb, loaded = DM.begin(b)
		second:advance(1)
		spec.eq(loaded.value, true, "a stored optional object loads")
		spec.eq(DM.session(b, pb).value.profile.level, 2, "its missing child takes the default at that path")
		spec.eq(DM.session(b, pb).value.profile.bio, "b", "beside the stored child")

		local bare = defaults()
		bare.profile = nil
		local third, c, _, seed = seeded(1, partial, nil, bare)
		refused("no default at that path", third, c, seed, ".profile.level", "migration step")
	end,
	["rule 76 and 82 a required field missing in an array element is unreadable, then a step that copies each element fills it"] = function()
		local payload = valid()
		payload.career.jobs = { { id = "j1", tips = 1, rating = 4 }, { id = "j2", rating = 5 } }
		local backend, a, _, seed = seeded(1, payload)
		refused("no step", backend, a, seed, ".career.jobs[1].tips", "migration step")
		spec.eq(#DM.logged(a, "needs a migration step"), 1, "the diagnostic says the field needed a step")

		local function fill(old)
			local jobs = {}
			for index, job in old.career.jobs do
				local copy = table.clone(job)
				if copy.tips == nil then
					copy.tips = 0
				end
				jobs[index] = copy
			end
			return { career = { jobs = jobs } }
		end
		local second, b = seeded(1, payload, { node = ROOT, migrations = { fill } })
		local pb, handle = DM.begin(b)
		second:advance(1)
		spec.eq(handle.value, true, "with the step it loads")
		local jobs = DM.session(b, pb).value.career.jobs
		spec.eq(jobs[1].tips, 1, "a present value is kept")
		spec.eq(jobs[2].tips, 0, "the missing one is filled")
		spec.eq(jobs[1].rating, 4, "each element keeps its unknown keys")
		spec.eq(jobs[2].rating, 5, "the filled one too")
		local envelope = DM.stored(second)
		spec.eq(envelope.schemaVersion, 1, "the save advanced the version")
		spec.eq(envelope.payload.career.jobs[2].rating, 5, "and stored the unknown key")
		spec.eq(envelope.payload.career.jobs[2].tips, 0, "with the filled field")
	end,
	["rule 76 and 82 a step fills a required key missing in a map value or a non-default variant"] = function()
		local function fill(old)
			local bonus = {}
			for key, entry in old.bonus do
				bonus[key] = table.clone(entry)
				bonus[key].n = bonus[key].n or 0
			end
			return { bonus = bonus, mode = { b = old.mode.b or 8, shared = old.mode.shared or 6 } }
		end
		local payload = valid()
		payload.bonus = { k = { extra = "E" } }
		payload.mode = { kind = "B" }
		local backend, a = seeded(1, payload, { node = ROOT, migrations = { fill } })
		local pa, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "the step makes both readable")
		local value = DM.session(a, pa).value
		spec.eq(value.bonus.k.n, 0, "the map value is filled")
		spec.eq(value.bonus.k.extra, "E", "and keeps its unknown key")
		spec.eq(value.mode.b, 8, "the non-default variant is filled")
		spec.eq(value.mode.shared, 6, "in every field")
	end,
	["rule 76 a missing optional key stays absent: clearing it through a write is durable across a reload"] = function()
		local backend, a, b = seeded(2, valid())
		local pa = D2.join(backend, a)
		spec.eq(DM.session(a, pa).value.nickname, "nick", "stored")
		local cleared = DM.write(a, pa, function(current)
			local value = table.clone(current)
			value.nickname = nil
			return value
		end)
		spec.eq(cleared.ok, true, `cleared: {cleared.err}`)
		local saved = DM.save(a, pa)
		backend:advance(1)
		spec.eq(saved.value, "ok", "saved")
		spec.eq(DM.stored(backend).payload.nickname, nil, "gone from storage")
		DM.leave(backend, a, pa)
		local pb = D2.join(backend, b)
		spec.eq(DM.session(b, pb).value.nickname, nil, "still cleared though defaults hold a nickname")

		local lacking = valid()
		lacking.nickname = nil
		local second, c = seeded(1, lacking)
		local pc = D2.join(second, c)
		spec.eq(DM.session(c, pc).value.nickname, nil, "a record that lacks it does not take the default")
	end,
	["rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing"] = function()
		local backend, a = seeded(1, valid())
		local pa = D2.join(backend, a)
		local session = DM.session(a, pa)
		local before = session.value
		local published = a.store.published[pa]
		local result = DM.write(a, pa, function(current)
			local value = table.clone(current)
			value.settings = table.clone(current.settings)
			value.settings.vibration = "SECRET-bool"
			return value
		end)
		spec.eq(result.ok, false, "threw")
		spec.eq(
			starts(result.err, "FD.Players.settings.vibration: expected boolean"),
			true,
			"store, path and reason lead the text"
		)
		spec.eq(string.find(result.err, "SECRET-bool", 1, true), nil, "never the value")
		spec.eq(rawequal(session.value, before), true, "the slot value is as before")
		spec.eq(session.revision, 0, "no revision")
		spec.eq(session.confirmed, -1, "nothing confirmed")
		spec.eq(rawequal(a.store.published[pa], published), true, "nothing published")

		local missing = DM.write(a, pa, function(current)
			local value = table.clone(current)
			value.settings = { vibration = true }
			return value
		end)
		spec.eq(missing.ok, false, "admission is without defaults: a missing required field throws")
		spec.eq(starts(missing.err, "FD.Players.settings.volume: "), true, "at its path")
		spec.eq(session.revision, 0, "still no revision")

		local notRecord = DM.write(a, pa, function()
			return "SECRET-string"
		end)
		spec.eq(notRecord.ok, false, "a non-record result throws")
		spec.eq(starts(notRecord.err, "FD.Players: "), true, "naming the store")
		spec.eq(string.find(notRecord.err, "SECRET-string", 1, true), nil, "never the value")
		spec.eq(session.revision, 0, "still no revision")

		local ok = DM.write(a, pa, function(current)
			local value = table.clone(current)
			value.coins = 12
			return value
		end)
		spec.eq(ok.ok, true, "a valid write after the refusals")
		spec.eq(session.revision, 1, "is revision 1")
		spec.eq(session.value.coins, 12, "stored in the slot")
	end,
	["rule 15 every admitted table is deeply frozen: the loaded value, declared and undeclared subtrees, and defaults"] = function()
		local payload = valid()
		payload.unknown = { inner = { z = 1 }, list = { 1, 2 } }
		local backend, a = seeded(1, payload)
		local pa = D2.join(backend, a)
		local value = DM.session(a, pa).value
		local tables = {
			["root"] = value,
			["a declared object"] = value.career,
			["a declared array"] = value.career.jobs,
			["an element"] = value.career.jobs[1],
			["a declared map"] = value.bonus,
			["a map value"] = value.bonus.k,
			["a union variant"] = value.mode,
			["an optional object"] = value.profile,
			["an undeclared subtree"] = value.unknown,
			["a table inside it"] = value.unknown.inner,
			["an undeclared array"] = value.unknown.list,
			["the defaults"] = a.store.schema.defaults,
			["a default subtree"] = a.store.schema.defaults.career,
			["a default element"] = a.store.schema.defaults.career.jobs[1],
		}
		for name, entry in tables do
			spec.eq(table.isfrozen(entry), true, `{name} is frozen`)
			local ok = pcall(function()
				entry.injected = 1
			end)
			spec.eq(ok, false, `{name}: mutation throws`)
			spec.eq(entry.injected, nil, `{name}: unchanged`)
		end
		local replaced = pcall(function()
			value.coins = 1
		end)
		spec.eq(replaced, false, "an existing field cannot be replaced")
		spec.eq(value.coins, 9, "and still holds its value")
	end,
	["rule 15 a frozen subtree of the current value is reused by identity when written back"] = function()
		local payload = valid()
		payload.unknown = { inner = { z = 1 } }
		local backend, a = seeded(1, payload)
		local pa = D2.join(backend, a)
		local session = DM.session(a, pa)
		local current = session.value
		local result = DM.write(a, pa, function(old)
			local value = table.clone(old)
			value.coins = 12
			value.settings = { vibration = old.settings.vibration, volume = 0.75 }
			return value
		end)
		spec.eq(result.ok, true, `written: {result.err}`)
		local value = session.value
		spec.eq(rawequal(value, current), false, "a new root")
		for _, key in { "career", "bonus", "mode", "profile", "unknown" } do
			spec.eq(rawequal(value[key], current[key]), true, `{key} is the very same table`)
		end
		spec.eq(rawequal(value.career.jobs, current.career.jobs), true, "so is a table inside it")
		spec.eq(rawequal(value.settings, current.settings), false, "a rebuilt table is a new one")
		spec.eq(table.isfrozen(value.settings), true, "frozen")
		spec.eq(value.settings.volume, 0.75, "with the written value")
	end,
	["rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused"] = function()
		for _, wrap in { 8, 9 } do
			local backend, a = seeded(1, { a = DM.nest(19) }, { version = nil }, {})
			local pa = D2.join(backend, a)
			local session = DM.session(a, pa)
			local current = session.value
			local result = DM.write(a, pa, function(old)
				local value = table.clone(old)
				local holder = { a = old.a }
				for _ = 2, wrap do
					holder = { deep = holder }
				end
				value.wrap = holder
				return value
			end)
			if wrap == 8 then
				spec.eq(
					result.ok,
					true,
					`a wrapper of {wrap} puts the subtree's deepest table at depth 28: {result.err}`
				)
				spec.eq(rawequal(session.value.a, current.a), true, "the cached subtree is reused")
			else
				spec.eq(result.ok, false, `a wrapper of {wrap} puts it at depth 29`)
				spec.eq(starts(result.err, "FD.Players"), true, "naming the store")
				spec.has(result.err, "deeper than 28", "and the reason")
				spec.eq(rawequal(session.value, current), true, "nothing changed")
				spec.eq(session.revision, 0, "no revision")
			end
		end
	end,
	["rule 15 a cached array counts its full visit cost at every use"] = function()
		local backend, a = seeded(1, { big = table.create(60000, 1) }, { version = nil }, {})
		local pa = D2.join(backend, a)
		local session = DM.session(a, pa)
		local current = session.value
		spec.eq(#current.big, 60000, "loaded")
		local twice = DM.write(a, pa, function(old)
			local value = table.clone(old)
			value.copy = old.big
			return value
		end)
		spec.eq(twice.ok, false, "60,001 visits twice, with the root, exceed 100,000")
		spec.has(twice.err, "more than 100000 values", "named")
		spec.eq(starts(twice.err, STORE), true, "naming the store")
		spec.eq(rawequal(session.value, current), true, "nothing changed")
		local once = DM.write(a, pa, function(old)
			local value = table.clone(old)
			value.flag = true
			return value
		end)
		spec.eq(once.ok, true, `used once it is accepted: {once.err}`)
		spec.eq(rawequal(session.value.big, current.big), true, "and reused")
		spec.eq(session.revision, 1, "one revision")
	end,
	["rule 14 and 76 a write over PAYLOAD_MAX_BYTES throws naming the store and user, a single string over 65536 is refused first"] = function()
		local backend, a = seeded(1, valid())
		local pa = D2.join(backend, a)
		local session = DM.session(a, pa)
		local current = session.value
		local function fill(count: number)
			return DM.write(a, pa, function(old)
				local value = table.clone(old)
				for index = 1, count do
					value[`blob{index}`] = string.rep("Zq", 30000)
				end
				return value
			end)
		end
		local over = fill(5)
		spec.eq(over.ok, false, "300,000 bytes in five strings of 60,000")
		spec.eq(starts(over.err, "FD.Players: 4120's profile is"), true, "names the store and the user")
		spec.has(over.err, "over 262144", "and the cap")
		spec.eq(string.find(over.err, "ZqZq", 1, true), nil, "never the content")
		spec.eq(rawequal(session.value, current), true, "nothing changed")
		spec.eq(session.revision, 0, "no revision")
		local under = fill(4)
		spec.eq(under.ok, true, `240,000 bytes are accepted: {under.err}`)
		local long = DM.write(a, pa, function(old)
			local value = table.clone(old)
			value.blob = string.rep("a", 65537)
			return value
		end)
		spec.eq(long.ok, false, "one string over 65,536 bytes")
		spec.has(
			long.err,
			`{STORE}.blob: a string over 65536 bytes`,
			"is refused by rule 76 with its path, before the size check"
		)
	end,
	["rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once"] = function()
		local payload = valid()
		payload.settings.vibration = "SECRET-diag"
		payload.secret = SECRET
		local backend, a = seeded(1, payload)
		local first, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, false, "refused")
		local messages = DM.logged(a, "did not load")
		spec.eq(#messages, 1, "one diagnostic")
		spec.has(messages[1], STORE, "the store")
		spec.has(messages[1], tostring(USER), "the user id")
		spec.has(messages[1], ".settings.vibration", "the path")
		spec.has(messages[1], "expected boolean", "the reason")
		spec.eq(a.engine.logged("SECRET-diag"), 0, "not the offending value")
		spec.eq(a.engine.logged(SECRET), 0, "not any other value")
		a.leave(first)
		for _ = 1, 2 do
			local again, retry = DM.begin(a)
			backend:advance(1)
			spec.eq(retry.value, false, "refused again")
			spec.eq(a.kicked(again), a.limits.LOAD_FAILED_MESSAGE, "kicked again")
			a.leave(again)
		end
		spec.eq(#DM.logged(a, "did not load"), 1, "rejoining twice logged nothing more")
	end,
}
