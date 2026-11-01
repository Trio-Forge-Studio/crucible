local spec = require("./lib/load")
local Backend = require("./lib/data-backend")
local D2 = require("./lib/data-d2")
local DM = require("./lib/data-migrations")

local n = DM.n
local STORE = DM.STORE
local USER = DM.USER

local JOB_N = DM.object({ coins = n.f64, id = n.string })
local JOB_N1 = DM.object({ coins = n.f64, id = n.string, rating = DM.opt(n.f64) })

local function root(job)
	return DM.object({
		career = DM.object({ jobs = n.array(job), stats = DM.object({ hp = n.f64 }) }),
		coins = n.f64,
		extra = n.union({ n["nil"], n.string, DM.object({ n = n.f64 }) }),
		ledger = n.map(n.string, DM.object({ amount = n.f64 })),
		mode = n.union({
			n.variant("kind", "A", DM.object({ a = n.f64 })),
			n.variant("kind", "B", DM.object({ b = n.f64 })),
		}),
		profile = DM.opt(DM.object({ bio = n.string })),
	})
end

local function defaults()
	return {
		career = { jobs = {}, stats = { hp = 1 } },
		coins = 0,
		ledger = {},
		mode = { kind = "A", a = 1 },
	}
end

-- A record build N+1 saved, with an unknown key at every kind of position.
local function stored()
	return {
		coins = 10,
		rootExtra = "R",
		career = {
			jobs = { { id = "j1", coins = 1 }, { id = "j3", coins = 40, rating = 5 } },
			stats = { hp = 3, buff = "B" },
			note = "C",
		},
		ledger = { a = { amount = 2, flag = "L" }, b = { amount = 3 } },
		mode = { kind = "A", a = 1, xtra = "X" },
		profile = { bio = "p", mood = "M" },
		extra = { n = 1, unk = "U" },
	}
end

local function world(builds)
	local backend = Backend.new()
	local realms = {}
	for index, build in builds do
		local realm = backend:realm()
		realm.store = realm.profile(STORE, build.defaults, build.shape)
		realms[index] = realm
	end
	return backend, table.unpack(realms)
end

local function builds()
	return { shape = { node = root(JOB_N) }, defaults = defaults() }, {
		shape = { node = root(JOB_N1) },
		defaults = defaults(),
	}
end

local function seed(backend, payload, fields)
	local envelope = { schemaVersion = 0, payload = payload or stored(), epoch = 5 }
	for key, value in fields or {} do
		envelope[key] = value
	end
	D2.seed(backend, nil, envelope)
end

local function loadedWorld(payload)
	local backend, a, b = world({ builds() })
	seed(backend, payload)
	local pa = D2.join(backend, a)
	return backend, a, pa, b
end

-- The value after a write by game code that compiled TypeScript would emit.
local function after(change, payload)
	local backend, a, pa = loadedWorld(payload)
	local result = DM.write(a, pa, change)
	spec.eq(result.ok, true, `written: {result.err}`)
	return DM.session(a, pa).value, backend, a, pa
end

local function clone(value)
	return table.clone(value)
end

local function editJob(old, index: number, edit)
	local value = clone(old)
	value.career = clone(old.career)
	local jobs = clone(old.career.jobs)
	jobs[index] = clone(old.career.jobs[index])
	edit(jobs[index])
	value.career.jobs = jobs
	return value
end

return {
	["rule 78 and 79 an unknown key survives an edit by copy at the root, in a nested object, an array element and a map value, and a newer build sees both"] = function()
		local backend, a, pa, b = loadedWorld()
		local value = DM.session(a, pa).value
		spec.eq(value.career.jobs[2].rating, 5, "build N keeps the unknown rating in memory")
		spec.eq(value.rootExtra, "R", "the root key")
		spec.eq(value.career.stats.buff, "B", "the nested key")
		spec.eq(value.ledger.a.flag, "L", "the map value's key")
		spec.eq(value.mode.xtra, "X", "the variant's key")
		spec.eq(table.isfrozen(value.career.jobs[2]), true, "frozen with its unknown key")

		local edits = {
			function(old)
				return editJob(old, 2, function(job)
					job.coins = 43
				end)
			end,
			function(old)
				local next = clone(old)
				next.ledger = clone(old.ledger)
				next.ledger.a = clone(old.ledger.a)
				next.ledger.a.amount = 9
				return next
			end,
			function(old)
				local next = clone(old)
				next.career = clone(old.career)
				next.career.stats = clone(old.career.stats)
				next.career.stats.hp = 4
				return next
			end,
			function(old)
				local next = clone(old)
				next.coins = 11
				return next
			end,
		}
		for index, edit in edits do
			local result = DM.write(a, pa, edit)
			spec.eq(result.ok, true, `edit {index}: {result.err}`)
		end
		local expected = {
			coins = 11,
			rootExtra = "R",
			career = {
				jobs = { { id = "j1", coins = 1 }, { id = "j3", coins = 43, rating = 5 } },
				stats = { hp = 4, buff = "B" },
				note = "C",
			},
			ledger = { a = { amount = 9, flag = "L" }, b = { amount = 3 } },
			mode = { kind = "A", a = 1, xtra = "X" },
			profile = { bio = "p", mood = "M" },
			extra = { n = 1, unk = "U" },
		}
		DM.deepEq(DM.session(a, pa).value, expected, "the live value")
		local saved = DM.save(a, pa)
		backend:advance(1)
		spec.eq(saved.value, "ok", "saved")
		DM.deepEq(DM.stored(backend).payload, expected, "the stored record holds every unknown key and every edit")
		spec.eq(DM.stored(backend).schemaVersion, 0, "at the same version")
		DM.leave(backend, a, pa)

		local pb = D2.join(backend, b)
		local seen = DM.session(b, pb).value
		DM.deepEq(seen, expected, "build N+1 reads both")
		spec.eq(seen.career.jobs[2].rating, 5, "its declared optional field has the unknown key's value")
		spec.eq(seen.career.jobs[2].coins, 43, "and the new declared value")
	end,
	["rule 79 a table game code builds fresh holds only what it was given"] = function()
		local value = after(function(old)
			return editJob(old, 2, function(job)
				job.coins = 41
			end)
		end)
		spec.eq(value.career.jobs[2].rating, 5, "control: a copy keeps the unknown key")

		local rebuilt = after(function(old)
			local next = clone(old)
			next.career = clone(old.career)
			local jobs = clone(old.career.jobs)
			local job = old.career.jobs[2]
			jobs[2] = { id = job.id, coins = job.coins + 1 }
			next.career.jobs = jobs
			return next
		end)
		DM.deepEq(rebuilt.career.jobs[2], { id = "j3", coins = 41 }, "a rebuilt element drops its unknown key")
		spec.eq(rebuilt.career.jobs[1].id, "j1", "the other element")
		spec.eq(rebuilt.career.note, "C", "siblings of the copy path keep theirs")
		spec.eq(rebuilt.career.stats.buff, "B", "so do other subtrees")

		local fresh = after(function(old)
			return { coins = 1, career = old.career, ledger = old.ledger, mode = old.mode }
		end)
		spec.eq(fresh.rootExtra, nil, "a rebuilt root drops the root's unknown key")
		spec.eq(fresh.career.note, "C", "but the tables it reuses keep theirs")
		spec.eq(fresh.profile, nil, "and the keys it lacks are gone")

		local swapped = after(function(old)
			local next = clone(old)
			next.mode = { kind = "B", b = 2 }
			return next
		end)
		DM.deepEq(swapped.mode, { kind = "B", b = 2 }, "replacing the union variant drops the old variant's key")
		local same = after(function(old)
			local next = clone(old)
			next.mode = clone(old.mode)
			next.mode.a = 5
			return next
		end)
		DM.deepEq(same.mode, { kind = "A", a = 5, xtra = "X" }, "editing the same variant by copy keeps it")

		local absent = after(function(old)
			local next = clone(old)
			next.profile = nil
			return next
		end)
		spec.eq(absent.profile, nil, "an optional object set absent takes its unknown key along")
		local recreated = after(function(old)
			local next = clone(old)
			next.profile = { bio = "q" }
			return next
		end)
		DM.deepEq(recreated.profile, { bio = "q" }, "a fresh optional object has none")

		local back, backend, a, pa = after(function(old)
			local next = clone(old)
			next.extra = "text"
			return next
		end)
		spec.eq(back.extra, "text", "a value replaced by another kind")
		local again = DM.write(a, pa, function(old)
			local next = clone(old)
			next.extra = { n = 2 }
			return next
		end)
		spec.eq(again.ok, true, "and replaced back")
		DM.deepEq(DM.session(a, pa).value.extra, { n = 2 }, "the old object's key did not come back")
		spec.eq(backend ~= nil, true, "world")

		local dropped = after(function(old)
			local next = clone(old)
			next.career = clone(old.career)
			next.career.jobs = { old.career.jobs[1] }
			next.ledger = { b = old.ledger.b }
			return next
		end)
		spec.eq(#dropped.career.jobs, 1, "a dropped array element")
		spec.eq(dropped.ledger.a, nil, "a dropped map entry")
		spec.eq(dropped.ledger.b.amount, 3, "the kept entry")
	end,
	["rule 78 an element that is kept while the array is reordered, filtered or capped keeps its unknown keys"] = function()
		local value = after(function(old)
			local next = clone(old)
			next.career = clone(old.career)
			next.career.jobs = { old.career.jobs[2], old.career.jobs[1] }
			return next
		end)
		spec.eq(value.career.jobs[1].id, "j3", "reordered")
		spec.eq(value.career.jobs[1].rating, 5, "the element carries its key to its new index")
		local capped = after(function(old)
			local next = clone(old)
			next.career = clone(old.career)
			next.career.jobs = { old.career.jobs[2] }
			return next
		end)
		spec.eq(#capped.career.jobs, 1, "filtered")
		spec.eq(capped.career.jobs[1].rating, 5, "the survivor keeps its key")
	end,
	["rule 78 write admission passes undeclared keys through at every depth and still validates declared fields"] = function()
		local value, backend, a, pa = after(function(old)
			local next = editJob(old, 1, function(job)
				job.fresh = "F3"
			end)
			next.fresh = "F0"
			next.career.fresh = "F1"
			next.ledger = clone(old.ledger)
			next.ledger.a = clone(old.ledger.a)
			next.ledger.a.fresh = { deep = { list = { 1, 2, 3 } } }
			next.mode = clone(old.mode)
			next.mode.fresh = "F5"
			return next
		end)
		spec.eq(value.fresh, "F0", "at the root")
		spec.eq(value.career.fresh, "F1", "nested")
		spec.eq(value.career.jobs[1].fresh, "F3", "in an element")
		spec.eq(value.ledger.a.fresh.deep.list[3], 3, "in a map value, a subtree")
		spec.eq(value.mode.fresh, "F5", "in a variant")
		spec.eq(table.isfrozen(value.ledger.a.fresh.deep), true, "frozen")
		local saved = DM.save(a, pa)
		backend:advance(1)
		spec.eq(saved.value, "ok", "saved")
		DM.deepEq(DM.stored(backend).payload.ledger.a.fresh, { deep = { list = { 1, 2, 3 } } }, "stored untouched")
		spec.eq(DM.stored(backend).payload.career.jobs[1].fresh, "F3", "in the element")

		local bad = DM.write(a, pa, function(old)
			return editJob(old, 2, function(job)
				job.coins = "SECRET-coins"
			end)
		end)
		spec.eq(bad.ok, false, "a wrong declared field in an element throws")
		spec.has(bad.err, "FD.Players.career.jobs[1].coins: expected number", "with the element's path")
		spec.eq(string.find(bad.err, "SECRET-coins", 1, true), nil, "never the value")
	end,
	["rule 76 and 78 undeclared keys count toward the payload cap, depth and visit limits and must be JSON"] = function()
		local backend, a, pa = loadedWorld()
		local session = DM.session(a, pa)
		local current = session.value
		local function tryJunk(junk)
			return DM.write(a, pa, function(old)
				return editJob(old, 1, function(job)
					job.junk = junk
				end)
			end)
		end
		local huge = {}
		for index = 1, 5 do
			huge[`s{index}`] = string.rep("Zq", 30000)
		end
		local capped = tryJunk(huge)
		spec.eq(capped.ok, false, "300,000 bytes of undeclared strings")
		spec.has(capped.err, "FD.Players: 4120's profile is", "refused by the payload cap")
		local fn = tryJunk(function() end)
		spec.eq(fn.ok, false, "an undeclared function")
		spec.has(
			fn.err,
			`{STORE}.career.jobs[0].junk: expected JSON, got function`,
			"is refused as non-JSON, at its path"
		)
		local nan = tryJunk(0 / 0)
		spec.eq(nan.ok, false, "an undeclared NaN")
		spec.has(nan.err, `{STORE}.career.jobs[0].junk: expected a finite number`, "is refused")
		local deepest = tryJunk(DM.nest(24))
		spec.eq(deepest.ok, true, `24 tables from depth 5 end at depth 28: {deepest.err}`)
		local base = DM.session(a, pa).value
		local deeper = tryJunk(DM.nest(25))
		spec.eq(deeper.ok, false, "25 tables end at depth 29")
		spec.has(deeper.err, "deeper than 28", "refused for depth")
		spec.eq(rawequal(DM.session(a, pa).value, base), true, "nothing changed")
		local visits = tryJunk(table.create(100000, 1))
		spec.eq(visits.ok, false, "100,000 undeclared numbers")
		spec.has(visits.err, "more than 100000 values", "exceed the visit limit")
		local fewer = tryJunk(table.create(50000, 1))
		spec.eq(fewer.ok, true, `50,000 are accepted: {fewer.err}`)
		spec.eq(rawequal(current, base), false, "the accepted writes took effect")
		backend:advance(0)
	end,
	["rule 80 a claim that keeps the map entries it does not know leaves a newer build's data, one that rebuilds from its catalog drops it"] = function()
		local catalog = { "u1", "u2" }
		local function run(keep: boolean)
			local backend = Backend.new()
			local a = backend:realm()
			local seen
			a.store = a.data:profile(
				a.schema(STORE, { upgrades = {} }, { node = DM.object({ upgrades = n.map(n.string, n.f64) }) }),
				{
					onLoad = function(_, value)
						seen = value.upgrades.future
						local upgrades = if keep then table.clone(value.upgrades) else {}
						for _, id in catalog do
							local level = value.upgrades[id]
							if level ~= nil then
								upgrades[id] = math.min(level, 10)
							end
						end
						local next = table.clone(value)
						next.upgrades = upgrades
						return next
					end,
					onRelease = function(_, value)
						return value
					end,
					publish = function() end,
					delivered = function() end,
				}
			)
			D2.seed(backend, nil, { payload = { upgrades = { u1 = 5, u2 = 50, future = 7 } }, epoch = 5 })
			local pa = D2.join(backend, a)
			spec.eq(seen, 7, "admission kept the entry the catalog does not know")
			spec.eq(DM.session(a, pa).revision, 1, "the claim's change is a revision")
			backend:advance(1)
			return DM.stored(backend).payload.upgrades
		end
		DM.deepEq(run(true), { u1 = 5, u2 = 10, future = 7 }, "the keeping claim stores the unknown entry")
		DM.deepEq(run(false), { u1 = 5, u2 = 10 }, "the rebuilding claim stores only the catalog")
	end,
	["rule 81 a required field added with a default at a root path needs no step: older records take it and an older build keeps it"] = function()
		local shapeN = { node = DM.object({ coins = n.f64 }) }
		local shapeN1 = { node = DM.object({ bonus = n.f64, coins = n.f64 }) }
		local backend, a, b = world({
			{ shape = shapeN, defaults = { coins = 0 } },
			{ shape = shapeN1, defaults = { bonus = 5, coins = 0 } },
		})
		seed(backend, { coins = 4 })
		local pb = D2.join(backend, b)
		local value = DM.session(b, pb).value
		spec.eq(value.bonus, 5, "an older record takes the default on N+1")
		spec.eq(value.coins, 4, "and keeps its data")
		DM.leave(backend, b, pb)

		seed(backend, { coins = 4, bonus = 9 }, { epoch = 8 })
		local pa, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "build N reads N+1's record without error")
		spec.eq(DM.session(a, pa).value.bonus, 9, "the field is an unknown key there")
		local edited = DM.write(a, pa, function(old)
			local next = clone(old)
			next.coins = 5
			return next
		end)
		spec.eq(edited.ok, true, "N writes it")
		local saved = DM.save(a, pa)
		backend:advance(1)
		spec.eq(saved.value, "ok", "saves")
		DM.deepEq(DM.stored(backend).payload, { coins = 5, bonus = 9 }, "and the field is kept")
		spec.eq(DM.stored(backend).schemaVersion, 0, "at the shared version")
	end,
	["rule 81 a field removed from a root path stays as an unknown key that an older build still reads"] = function()
		local shapeN = { node = DM.object({ coins = n.f64, legacy = n.f64 }) }
		local shapeN1 = { node = DM.object({ coins = n.f64 }) }
		local backend, a, b = world({
			{ shape = shapeN, defaults = { coins = 0, legacy = 0 } },
			{ shape = shapeN1, defaults = { coins = 0 } },
		})
		seed(backend, { coins = 4, legacy = 3 })
		local pb = D2.join(backend, b)
		spec.eq(DM.session(b, pb).value.legacy, 3, "N+1 keeps the removed field in place")
		local edited = DM.write(b, pb, function(old)
			local next = clone(old)
			next.coins = 6
			return next
		end)
		spec.eq(edited.ok, true, "N+1 writes")
		local saved = DM.save(b, pb)
		backend:advance(1)
		spec.eq(saved.value, "ok", "saves")
		DM.leave(backend, b, pb)
		DM.deepEq(DM.stored(backend).payload, { coins = 6, legacy = 3 }, "the removed field is still stored")
		local pa = D2.join(backend, a)
		spec.eq(DM.session(a, pa).value.legacy, 3, "the older build reads that value")
		DM.leave(backend, a, pa)

		seed(backend, { coins = 8 }, { epoch = 20 })
		local fresh = D2.join(backend, a)
		spec.eq(DM.session(a, fresh).value.legacy, 0, "a record without it takes the older build's default")
	end,
	["rule 82 and 83 a per-version fixture with every collection and variant loads through a declaration with later steps"] = function()
		local JOB = DM.object({ coins = n.f64, id = n.string, tips = n.f64 })
		local node = DM.object({
			career = DM.object({ jobs = n.array(JOB) }),
			ledger = n.map(n.string, DM.object({ amount = n.f64 })),
			mode = n.union({
				n.variant("kind", "A", DM.object({ a = n.f64 })),
				n.variant("kind", "B", DM.object({ b = n.f64 })),
			}),
			name = DM.opt(n.string),
			profile = DM.opt(DM.object({ bio = n.string })),
			title = n.string,
		})
		local migrations = {
			function(old)
				local jobs = {}
				for index, job in old.career.jobs do
					local copy = table.clone(job)
					copy.tips = 0
					jobs[index] = copy
				end
				return { career = { jobs = jobs } }
			end,
			function(old)
				return { title = old.name }
			end,
		}
		local fixtures = {}
		for _, variant in { { kind = "A", a = 1 }, { kind = "B", b = 2 } } do
			for _, profile in { { bio = "p" }, false } do
				table.insert(fixtures, {
					name = "n",
					career = { jobs = { { id = "j1", coins = 1 } } },
					ledger = { a = { amount = 2 } },
					mode = variant,
					profile = profile or nil,
				})
			end
		end
		for index, fixture in fixtures do
			local backend, a = D2.world(1, {
				defaults = { career = { jobs = {} }, ledger = {}, mode = { kind = "A", a = 0 }, title = "" },
				version = { node = node, migrations = migrations },
			})
			D2.seed(backend, nil, { schemaVersion = 0, epoch = 3, payload = fixture })
			local pa, handle = DM.begin(a)
			backend:advance(1)
			spec.eq(handle.value, true, `fixture {index} loads`)
			local value = DM.session(a, pa).value
			spec.eq(value.career.jobs[1].tips, 0, `fixture {index}: the element step ran on the populated array`)
			spec.eq(value.title, "n", `fixture {index}: the second step ran`)
			spec.eq(value.mode.kind, fixture.mode.kind, `fixture {index}: its variant`)
			spec.eq(value.ledger.a.amount, 2, `fixture {index}: its map entry`)
			spec.eq((value.profile ~= nil), (fixture.profile ~= nil), `fixture {index}: optional object as stored`)
			spec.eq(DM.stored(backend).schemaVersion, 2, `fixture {index}: saved at version 2`)
		end
	end,
	["rule 83 a widening without a step makes an older build fail the load as unreadable, and with a step refuse it as newer"] = function()
		local narrow = { node = DM.object({ level = n.f64 }) }
		local widened = { node = DM.object({ level = n.union({ n.f64, n.string }) }) }
		local backend, old, new = world({
			{ shape = narrow, defaults = { level = 0 } },
			{ shape = widened, defaults = { level = 0 } },
		})
		D2.seed(backend, nil, { payload = { level = 3 }, epoch = 1 })
		local pn = D2.join(backend, new)
		local set = DM.write(new, pn, function()
			return { level = "SECRET-max" }
		end)
		spec.eq(set.ok, true, "the widened build stores a string")
		local saved = DM.save(new, pn)
		backend:advance(1)
		spec.eq(saved.value, "ok", "saved")
		DM.leave(backend, new, pn)
		local before = DM.stored(backend)
		DM.deepEq(before.payload, { level = "SECRET-max" }, "the widened value is stored at version 0")
		spec.eq(before.schemaVersion, 0, "with no step")
		local po, handle = DM.begin(old)
		backend:advance(1)
		DM.assertRefused(backend, old, po, handle, old.limits.LOAD_FAILED_MESSAGE, nil, "unreadable, not newer", before)
		spec.has(DM.logged(old, "did not load")[1], `{STORE}.level: expected number, got string`, "at its path")
		spec.eq(old.engine.logged("SECRET-max"), 0, "without the value")

		local stepped = { node = widened.node, migrations = {
			function()
				return {}
			end,
		} }
		local second, older, newer = world({
			{ shape = narrow, defaults = { level = 0 } },
			{ shape = stepped, defaults = { level = 0 } },
		})
		D2.seed(second, nil, { schemaVersion = 1, payload = { level = "max" }, epoch = 2 })
		local pe, refused = DM.begin(older)
		second:advance(1)
		spec.eq(refused.value, false, "refused")
		spec.eq(DM.kick(older, pe).message, older.limits.SERVER_UPDATING_MESSAGE, "as a newer record")
		backend:advance(0)
		spec.eq(newer ~= nil, true, "the stepped build exists")
	end,
	["rule 84 stored data is admitted and frozen at the engine boundary, with its unknown keys kept"] = function()
		local backend, a = loadedWorld()
		local inspected = DM.inspect(backend, a, USER + 1)
		spec.eq(inspected.kind, "ok", "an absent key reads as ok")
		seed(backend, stored(), { epoch = 9 })
		local result = a.run(function()
			return a.store:inspect(USER)
		end)
		backend:advance(1)
		local value = result.value.value
		spec.eq(value.career.jobs[2].rating, 5, "inspect keeps the unknown key an admission dropping them would lose")
		spec.eq(value.rootExtra, "R", "at the root")
		for name, entry in
			{ value, value.career, value.career.jobs, value.career.jobs[2], value.ledger.a, value.profile }
		do
			spec.eq(table.isfrozen(entry), true, `{name} is frozen`)
		end
		local loaded = a.admission.loadRecord(a.store.schema, stored(), 0)
		spec.eq(loaded.kind, "ok", "loadRecord")
		spec.eq(loaded.migrated, false, "unmigrated")
		spec.eq(table.isfrozen(loaded.value.career.stats), true, "frozen")
		spec.eq(loaded.value.career.stats.buff, "B", "unknown keys kept")
	end,
}
