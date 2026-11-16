local spec = require("../packages/crucible/lune")
local D2 = require("./lib/data-d2")
local DM = require("./lib/data-migrations")

local n = DM.n
local STORE = DM.STORE
local USER = DM.USER
local SECRET = "SECRET-payload-value"

local rootNode
local function recordNode()
	rootNode = rootNode or DM.object({ coins = n.f64, gems = n.f64, label = DM.opt(n.string) })
	return rootNode
end

-- Two steps whose order shows in the result: doubling then adding 5 gives 19 from 7, adding then doubling gives 24.
local function twoSteps(log)
	return {
		node = recordNode(),
		migrations = {
			function(old)
				table.insert(log, { step = 0, coins = old.coins })
				return { coins = old.coins * 2 }
			end,
			function(old)
				table.insert(log, { step = 1, coins = old.coins })
				return { coins = old.coins + 5, label = "m2" }
			end,
		},
	}
end

local OLD = { coins = 7, gems = 3 }
local MIGRATED = { coins = 19, gems = 3, label = "m2" }

local function seedOld(backend, fields)
	local seed = { schemaVersion = 0, payload = OLD, epoch = 5 }
	for key, value in fields or {} do
		seed[key] = value
	end
	D2.seed(backend, nil, seed)
	return seed
end

local function steps(log)
	local order = {}
	for _, entry in log do
		table.insert(order, entry.step)
	end
	return order
end

local function untouchedSeed(payload)
	return { schemaVersion = 0, payload = payload, epoch = 5 }
end

-- A step shape whose step `failing` of two fails in `mode`.
local function failingShape(mode: string, failing: number, holder, ran)
	local function fail()
		if mode == "throws" then
			error("secret-boom")
		elseif mode == "yields" then
			coroutine.yield()
		elseif mode == "waits" then
			holder.realm.wait(1)
		elseif mode == "string" then
			return "not a record"
		elseif mode == "number" then
			return 42
		elseif mode == "array" then
			return { 1, 2 }
		end
		return nil
	end
	local migrations = {}
	for index = 0, 1 do
		migrations[index + 1] = function(old)
			table.insert(ran, index)
			if index == failing then
				return fail()
			end
			return { coins = old.coins + 1 }
		end
	end
	return { migrations = migrations }
end

local function nest(value, depth)
	for _ = 1, depth do
		value = { child = value }
	end
	return value
end

local function deepest(value)
	while value.child do
		value = value.child
	end
	return value
end

-- `levels` tables, each holding the next under both `left` and `right`, so a tree copy would hold 2^levels leaves.
local function shared(levels, leaf)
	local graph = leaf
	for _ = 1, levels do
		graph = { left = graph, right = graph }
	end
	return graph
end

-- One step, the last, returning `make(api)` over the stored record `{ coins = 1 }`, timed from the load.
local function lastStep(make, node)
	local backend, a = D2.world(1)
	local patch = make(a.graph.module(""))
	a.store = a.profile(STORE, { coins = 0 }, { node = node, migrations = {
		function()
			return patch
		end,
	} })
	D2.seed(backend, nil, { payload = { coins = 1 } })
	local started = os.clock()
	local player, handle = DM.load(backend, a)
	return backend, a, player, handle, os.clock() - started
end

local function diagnostic(path, reason)
	return `{STORE}{path}: {reason} (after migrating from version 0)`
end

return {
	["rule 75 nil replacements throw in every position"] = function()
		for _, position in { "object", "root", "earlier-scalar", "last" } do
			local backend, a = D2.world(1)
			local api = a.graph.module("")
			local caught
			local later = false
			local function step()
				local ok, reason = pcall(api.migrationReplace, nil)
				spec.eq(ok, false, position)
				assert(string.find(tostring(reason), "migrationReplace requires a JSON value, got nil", 1, true))
				caught = true
				if position == "root" then
					return api.migrationReplace(nil)
				end
				return { field = api.migrationReplace(nil) }
			end
			a.store = a.profile(STORE, { coins = 0 }, {
				migrations = if position == "last"
					then { step }
					else {
						step,
						function()
							later = true
							return {}
						end,
					},
			})
			D2.seed(backend, nil, { payload = { coins = 1, field = if position == "earlier-scalar" then 7 else {} } })
			local _, handle = DM.load(backend, a)
			spec.eq(caught, true, "guard was exercised")
			spec.eq(handle.value, false, "nil replacement refuses migration")
			spec.eq(later, false, "next step never runs")
		end
	end,
	["rule 75 deletion marker cannot be mutated"] = function()
		local _, a, _, handle = lastStep(function(api)
			local ok = pcall(function()
				api.migrationDelete[1] = true
			end)
			spec.eq(ok, false, "delete marker is frozen")
			return api.migrationDelete
		end)
		spec.eq(handle.value, false, "root deletion refuses")
		spec.eq(#DM.logged(a, "returned migrationDelete, not a record"), 1, "delete diagnostic")
	end,
	["rule 75 depth takes priority over invalid deep keys"] = function()
		local _, a, _, handle = lastStep(function()
			return nest({ [{}] = 1 }, 28)
		end)
		spec.eq(handle.value, false, "deep key refused")
		spec.eq(#DM.logged(a, diagnostic(string.rep(".child", 28), "deeper than 28")), 1, "depth diagnostic")
	end,
	["rule 75 final cycles keep admission paths and declared field priority"] = function()
		for _, mode in { "array", "root", "declared" } do
			local _, a, _, handle = lastStep(function()
				local cycle = {}
				if mode == "array" then
					cycle[1] = cycle
					return { added = cycle }
				end
				cycle.left = cycle
				if mode == "root" then
					return cycle
				end
				return { coins = "x", added = cycle }
			end, DM.object({ coins = n.f64 }))
			spec.eq(handle.value, false, "cycle refused")
			local path = if mode == "array" then ".added[0]" elseif mode == "root" then ".left.left" else ".coins"
			local reason = if mode == "declared" then "expected number, got string" else "a cycle"
			spec.eq(#DM.logged(a, diagnostic(path, reason)), 1, mode)
		end
	end,
	["rule 75 a final cycle containing operations still reaches admission"] = function()
		local _, a, _, handle = lastStep(function(api)
			local cycle = { gone = api.migrationDelete }
			cycle.self = cycle
			return { added = cycle }
		end)
		spec.eq(handle.value, false, "marked cycle refused")
		spec.eq(#DM.logged(a, diagnostic(".added.self", "a cycle")), 1, "cycle path")
	end,

	["rule 75 temporary deep patches preserve merging and consume operations before the next step"] = function()
		for _, depth in { 27, 28, 29, 40 } do
			for _, mode in { "plain", "delete", "replace", "scalar" } do
				local expected = if mode == "plain"
					then { a = 3, b = 2 }
					elseif mode == "scalar" then { a = 3, b = 4 }
					else { a = 3 }
				local backend, a = D2.world(1)
				local api = a.graph.module("")
				a.store = a.profile(STORE, { coins = 0 }, {
					migrations = {
						function()
							return nest({ a = 1, b = 2 }, depth)
						end,
						function()
							local patch = { a = 3 }
							if mode == "delete" then
								patch.b = api.migrationDelete
							elseif mode == "scalar" then
								patch.b = api.migrationReplace(4)
							elseif mode == "replace" then
								patch = api.migrationReplace({ a = 3 })
							end
							return nest(patch, depth)
						end,
						function(old)
							local result = deepest(old)
							DM.deepEq(result, expected, "next step")
							return api.migrationReplace({ coins = 0, result = result })
						end,
					},
				})
				D2.seed(backend, nil, { payload = { coins = 1 } })
				local player, handle = DM.load(backend, a)
				spec.eq(handle.value, true, `{mode} at {depth} loads after flattening`)
				DM.deepEq(DM.session(a, player).value.result, expected, "saved result")
			end
		end
	end,
	["rule 75 markers in literals and a root deletion refuse the record before another step"] = function()
		for _, mode in { "root", "array", "replacement", "replacement-array", "metatable", "table-key" } do
			local backend, a = D2.world(1)
			local api = a.graph.module("")
			local later = false
			a.store = a.profile(STORE, { coins = 0 }, {
				migrations = {
					function()
						if mode == "root" then
							return api.migrationDelete
						elseif mode == "array" then
							return { items = { api.migrationDelete } }
						elseif mode == "metatable" then
							return { added = setmetatable({ gone = api.migrationDelete }, {}) }
						elseif mode == "table-key" then
							return { added = { [api.migrationDelete] = 1 } }
						elseif mode == "replacement" then
							return { items = api.migrationReplace({ nested = api.migrationReplace(1) }) }
						end
						return api.migrationReplace({ items = { api.migrationDelete } })
					end,
					function()
						later = true
						return {}
					end,
				},
			})
			D2.seed(backend, nil, { payload = { coins = 1 } })
			local _, handle = DM.load(backend, a)
			spec.eq(handle.value, false, `{mode} refused`)
			spec.eq(later, false, "no marker reaches another step")
			spec.eq(
				#DM.logged(
					a,
					if mode == "root"
						then "returned migrationDelete, not a record"
						elseif mode == "table-key" then "a table key"
						else "a table with a metatable"
				),
				1,
				"diagnostic"
			)
			DM.deepEq(DM.stored(backend).payload, { coins = 1 }, "stored data unchanged")
		end
	end,
	["rule 75 operations beneath scalars and arrays produce clean objects"] = function()
		for _, previous in { 7, { 1, 2 } } do
			local backend, a = D2.world(1)
			local api = a.graph.module("")
			a.store = a.profile(STORE, { coins = 0 }, {
				migrations = {
					function()
						return { field = { gone = api.migrationDelete, kept = api.migrationReplace({ 3 }) } }
					end,
					function(old)
						DM.deepEq(old.field, { kept = { 3 } }, "operations consumed")
						return {}
					end,
				},
			})
			D2.seed(backend, nil, { payload = { coins = 1, field = previous } })
			local player, handle = DM.load(backend, a)
			spec.eq(handle.value, true, "loaded")
			DM.deepEq(DM.session(a, player).value.field, { kept = { 3 } }, "saved object")
		end
	end,
	["rule 75 a shared added graph is walked once per table and refused with the diagnostic a tree gets"] = function()
		for _, case in
			{
				{
					levels = 20,
					marked = false,
					path = string.rep(".left", 20) .. ".bad",
					reason = "expected a finite number",
				},
				{ levels = 20, marked = true },
				{ levels = 40, marked = false, path = string.rep(".left", 27), reason = "deeper than 28" },
			}
		do
			local _, a, _, handle, seconds = lastStep(function(api)
				return {
					added = shared(case.levels, { bad = 0 / 0, gone = case.marked and api.migrationDelete or nil }),
				}
			end)
			local label = `{case.levels} levels{if case.marked then " with an operation" else ""}`
			assert(seconds < 0.5, `{label}: refused in {seconds} s, not after expanding the graph`)
			spec.eq(handle.value, false, `{label}: refused`)
			if case.path then
				spec.eq(#DM.logged(a, diagnostic(".added" .. case.path, case.reason)), 1, `{label}: diagnostic`)
			end
		end
	end,
	["rule 75 a table without operations is attached as the step returned it"] = function()
		local frozen = table.freeze({ value = 1 })
		local backend, a, player, handle = lastStep(function()
			return { added = shared(3, { value = 1 }), kept = frozen }
		end)
		spec.eq(handle.value, true, "loaded")
		assert(rawequal(DM.session(a, player).value.kept, frozen), "a frozen table is admitted in place")
		local tree = { value = 1 }
		for _ = 1, 3 do
			tree = { left = table.clone(tree), right = table.clone(tree) }
		end
		DM.deepEq(DM.stored(backend).payload, { coins = 1, added = tree, kept = { value = 1 } }, "saved as a tree")
	end,
	["rule 75 a last step's patch deeper than the stack is refused with admission's depth diagnostic"] = function()
		for _, marked in { false, true } do
			local _, a, _, handle = lastStep(function(api)
				local chain = if marked then { gone = api.migrationDelete } else { value = 1 }
				for _ = 1, 30000 do
					chain = { child = chain }
				end
				return { added = chain }
			end)
			local label = if marked then "with an operation at the bottom" else "plain"
			spec.eq(handle.value, false, `{label}: refused, not thrown`)
			spec.eq(
				#DM.logged(a, diagnostic(".added" .. string.rep(".child", 27), "deeper than 28")),
				1,
				`{label}: diagnostic`
			)
		end
	end,
	["rule 75 the last step applies operations down to the persistence limit"] = function()
		local _, a, player, handle = lastStep(function(api)
			return nest({ a = 1, b = api.migrationDelete }, 27)
		end)
		spec.eq(handle.value, true, "an operation at depth 29 applies")
		DM.deepEq(deepest(DM.session(a, player).value), { a = 1 }, "the key is removed")
		local _, deeper, _, refused = lastStep(function(api)
			return nest({ a = 1, b = api.migrationDelete }, 28)
		end)
		spec.eq(refused.value, false, "a table at depth 29 is refused")
		spec.eq(#DM.logged(deeper, diagnostic(string.rep(".child", 28), "deeper than 28")), 1, "by admission")
	end,
	["rule 75 only migrationReplace makes a replacement, and its metatable is hidden and locked"] = function()
		local backend, a, _, handle = lastStep(function(api)
			local real = api.migrationReplace(5)
			spec.eq(typeof(getmetatable(real)), "string", "the metatable is hidden")
			spec.eq((pcall(setmetatable, real, nil)), false, "and cannot be replaced")
			spec.eq((pcall(setmetatable, api.migrationDelete, nil)), false, "nor can the deletion's")
			return { coins = setmetatable({ value = 5 }, { __metatable = getmetatable(real) }) }
		end)
		spec.eq(handle.value, false, "a forged replacement is refused, not honoured")
		spec.eq(#DM.logged(a, diagnostic(".coins", "a table with a metatable")), 1, "as a table with a metatable")
		DM.deepEq(DM.stored(backend).payload, { coins = 1 }, "stored data unchanged")
	end,
	["rule 75 a table key is refused even when a later step overwrites it"] = function()
		local cyclic = {}
		cyclic.self = cyclic
		for _, key in { cyclic, {} } do
			local later = false
			local backend, a = D2.world(1, {
				version = {
					migrations = {
						function()
							return { temporary = { [key] = 1 } }
						end,
						function()
							later = true
							return { temporary = 0 }
						end,
					},
				},
			})
			D2.seed(backend, nil, { payload = { coins = 1 } })
			local _, handle = DM.load(backend, a)
			spec.eq(handle.value, false, "refused")
			spec.eq(later, false, "before the next step")
			spec.eq(#DM.logged(a, diagnostic(".temporary", "a table key")), 1, "diagnostic")
			DM.deepEq(DM.stored(backend).payload, { coins = 1 }, "stored data unchanged")
		end
	end,
	["rule 75 cyclic additions are unreadable and leave stored data untouched"] = function()
		local cycle = {}
		cycle.left, cycle.right = cycle, cycle
		for _, earlier in { false, true } do
			local later = false
			local migrations = {
				function()
					return { added = cycle }
				end,
			}
			if earlier then
				table.insert(migrations, function()
					later = true
					return { added = 0 }
				end)
			end
			local backend, a = D2.world(1, { version = { migrations = migrations } })
			D2.seed(backend, nil, { payload = { coins = 1, gems = 5 } })
			local label = if earlier then "in an earlier step" else "in the last step"
			local started = os.clock()
			local _, handle = DM.load(backend, a)
			assert(os.clock() - started < 1, `{label}: a branching cycle is refused promptly`)
			spec.eq(#DM.logged(a, "a cycle"), 1, `{label}: the cycle diagnostic`)
			spec.eq(handle.value, false, `{label}: a cycle fails admission`)
			spec.eq(later, false, `{label}: before another step`)
			DM.deepEq(DM.stored(backend).payload, { coins = 1, gems = 5 }, `{label}: old data`)
		end
	end,
	["rule 75 an earlier step's operations apply in a new subtree below the persistence limit"] = function()
		local backend, a = D2.world(1)
		local api = a.graph.module("")
		a.store = a.profile(STORE, { coins = 0 }, {
			migrations = {
				function()
					return { added = nest({ a = 1, b = api.migrationDelete, c = api.migrationReplace({ 2 }) }, 40) }
				end,
				function(old)
					DM.deepEq(deepest(old.added), { a = 1, c = { 2 } }, "applied before the next step")
					return { added = api.migrationDelete }
				end,
			},
		})
		D2.seed(backend, nil, { payload = { coins = 1 } })
		local player, handle = DM.load(backend, a)
		spec.eq(handle.value, true, "loaded")
		DM.deepEq(DM.session(a, player).value, { coins = 1 }, "the temporary subtree is gone")
	end,
	["rule 75 replacement literals still fail schema admission without saving"] = function()
		for _, value in { "bad", { coins = "bad" } } do
			local backend, a = D2.world(1)
			local api = a.graph.module("")
			a.store = a.profile(STORE, { coins = 0, gems = 0 }, {
				node = recordNode(),
				migrations = {
					function()
						return api.migrationReplace(value)
					end,
				},
			})
			D2.seed(backend, nil, { payload = { coins = 1, gems = 5 } })
			local _, handle = DM.load(backend, a)
			spec.eq(handle.value, false, "invalid replacement is unreadable")
			DM.deepEq(DM.stored(backend).payload, { coins = 1, gems = 5 }, "old data stays stored")
		end
	end,
	["rule 75 explicit operations rename maps and delete keys before admission and persist once"] = function()
		local backend, a, b = D2.world(2)
		local api = a.graph.module("")
		local ran = 0
		local shape = {
			node = DM.object({ coins = n.f64, gems = n.f64, discoveries = n.map(n.string, n.f64) }),
			migrations = {
				function(old)
					ran += 1
					return {
						discoveries = api.migrationReplace({ renamed = old.discoveries.legacy }),
						obsolete = api.migrationDelete,
						nested = { removed = api.migrationDelete },
						newMap = { absent = api.migrationDelete, present = api.migrationReplace({ value = 7 }) },
						empty = api.migrationReplace({}),
					}
				end,
				function(old)
					ran += 1
					spec.eq(old.obsolete, nil, "removed before the next step")
					spec.eq(old.discoveries.legacy, nil, "renamed before the next step")
					return { coins = old.discoveries.renamed }
				end,
			},
		}
		a.store = a.profile(STORE, { coins = 0, gems = 0, discoveries = {} }, shape)
		b.store = b.profile(STORE, { coins = 0, gems = 0, discoveries = {} }, shape)
		local payload = {
			coins = 1,
			gems = 3,
			discoveries = { legacy = 8 },
			obsolete = 9,
			nested = { removed = 1, kept = 2 },
			empty = { stale = 1 },
			literal = { kind = "replace", value = { kept = true } },
		}
		D2.seed(backend, nil, { payload = payload })
		local player, handle = DM.load(backend, a)
		spec.eq(handle.value, true, "loads without onLoad cleanup")
		local expected = {
			coins = 8,
			gems = 3,
			discoveries = { renamed = 8 },
			nested = { kept = 2 },
			empty = {},
			newMap = { present = { value = 7 } },
			literal = { kind = "replace", value = { kept = true } },
		}
		DM.deepEq(DM.session(a, player).value, expected, "live data")
		DM.deepEq(DM.stored(backend).payload, expected, "saved data contains no operations")
		DM.leave(backend, a, player)
		local second = D2.join(backend, b)
		DM.deepEq(DM.session(b, second).value, expected, "reload")
		spec.eq(ran, 2, "no migration reruns")
	end,
	["rule 75 a whole record replacement removes omitted keys and still admits defaults"] = function()
		local backend, a = D2.world(1)
		local api = a.graph.module("")
		a.store = a.profile(STORE, { coins = 0, gems = 0 }, {
			node = recordNode(),
			migrations = {
				function()
					return api.migrationReplace({ coins = 9 })
				end,
			},
		})
		D2.seed(backend, nil, { payload = { coins = 1, gems = 5, obsolete = 10 } })
		local player, handle = DM.load(backend, a)
		spec.eq(handle.value, true, "loaded")
		DM.deepEq(DM.session(a, player).value, { coins = 9, gems = 0 }, "omitted field uses defaults")
		DM.deepEq(DM.stored(backend).payload, { coins = 9, gems = 0 }, "saved")
	end,
	["rule 74 a version 0 record with two steps migrates, counts one revision and saves schemaVersion 2 with the payload"] = function()
		local log = {}
		local backend, a, b = D2.world(2, { version = twoSteps(log) })
		seedOld(backend)
		local pa, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "loaded")
		local session = DM.session(a, pa)
		spec.eq(session.revision, 1, "the migration is revision 1")
		DM.deepEq(session.value, MIGRATED, "the live value is the migrated record")
		spec.list(steps(log), { 0, 1 }, "both steps ran once, in order")
		spec.eq(log[1].coins, 7, "step 0 saw the stored record")
		spec.eq(log[2].coins, 14, "step 1 saw step 0's merged result")
		local calls = backend:callsTo({ server = a.id })
		spec.eq(#calls, 2, "the acquisition and one save, already requested")
		assert(calls[2].at < 0.3, `the save was requested at once, not at the renewal (t = {calls[2].at})`)
		spec.eq(calls[2].committed, true, "the save committed")
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 2, "version committed with the payload")
		DM.deepEq(envelope.payload, MIGRATED, "migrated payload")
		spec.eq(envelope.epoch, 6, "one acquisition")
		spec.eq(envelope.commit.revision, 1, "the migration's revision")
		spec.eq(envelope.commit.token, session.token, "under the session's token")
		spec.eq(envelope.lease.serverId, a.id, "the lease is the session's")
		spec.eq(session.confirmed, 1, "confirmed")

		DM.leave(backend, a, pa)
		local pb = D2.join(backend, b)
		spec.eq(#log, 2, "a second realm of the same build runs no step")
		local second = DM.session(b, pb)
		DM.deepEq(second.value, MIGRATED, "the same value")
		spec.eq(second.revision, 0, "nothing migrated, so revision 0")
		spec.eq(#backend:callsTo({ server = b.id }), 1, "and no save is requested")
		spec.eq(DM.stored(backend).schemaVersion, 2, "still version 2")
	end,
	["rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2"] = function()
		local log = {}
		local backend, a = D2.world(1, { version = twoSteps(log) })
		seedOld(backend)
		backend:fail("committed", { server = a.id }, 1, { after = 1 })
		local beforeCommit = backend:gate("commit", { index = 2 })
		local afterCommit = backend:gate("committed", { index = 2 })
		local pa, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "loaded")
		spec.eq(backend.calls[2].parked, "commit", "the save is parked before its commit")
		local parked = DM.stored(backend)
		spec.eq(parked.schemaVersion, 0, "the stored version is still 0")
		DM.deepEq(parked.payload, OLD, "with the old payload")

		beforeCommit.release()
		spec.eq(backend.calls[2].parked, "committed", "committed, response held")
		local committed = DM.stored(backend)
		spec.eq(committed.schemaVersion, 2, "version 2 landed in the same write")
		DM.deepEq(committed.payload, MIGRATED, "as the migrated payload")

		afterCommit.release()
		for _ = 1, 120 do
			backend:advance(0.05)
			local envelope = DM.stored(backend)
			local pair = (envelope.schemaVersion == 0 and envelope.payload.coins == 7)
				or (envelope.schemaVersion == 2 and envelope.payload.coins == 19)
			assert(pair, `a torn pair: version {envelope.schemaVersion} with coins {envelope.payload.coins}`)
		end
		spec.eq(backend.calls[2].plan, "committed", "the first save committed and answered with an error")
		spec.eq(backend.calls[3].committed, nil, "the retry found its own commit and wrote nothing")
		spec.eq(#backend:callsTo({ server = a.id }), 3, "no other call yet")
		local final = DM.stored(backend)
		spec.eq(final.schemaVersion, 2, "ends at version 2")
		DM.deepEq(final.payload, MIGRATED, "with the migrated payload")
		spec.eq(DM.session(a, pa).confirmed, 1, "the retry confirmed what it found")
		spec.eq(#log, 2, "the steps ran once")
	end,
	["rule 74 a record at version v of N runs exactly migrations v to N-1, in order"] = function()
		local expected = {
			[0] = { order = { 0, 1, 2 }, value = 1012 },
			[1] = { order = { 1, 2 }, value = 112 },
			[2] = { order = { 2 }, value = 12 },
			[3] = { order = {}, value = 1 },
		}
		for version = 0, 3 do
			local order = {}
			local migrations = {}
			for index = 0, 2 do
				migrations[index + 1] = function(old)
					table.insert(order, index)
					return { n = old.n * 10 + index }
				end
			end
			local backend, a = D2.world(1, { version = { migrations = migrations } })
			D2.seed(backend, nil, { schemaVersion = version, payload = { n = 1 }, epoch = 2 })
			local pa, handle = DM.load(backend, a)
			spec.eq(handle.value, true, `v{version}: loaded`)
			spec.list(order, expected[version].order, `v{version}: steps run`)
			local session = DM.session(a, pa)
			spec.eq(session.value.n, expected[version].value, `v{version}: value`)
			spec.eq(session.revision, if version < 3 then 1 else 0, `v{version}: revision`)
			spec.eq(DM.stored(backend).schemaVersion, 3, `v{version}: stored version`)
			spec.eq(DM.stored(backend).payload.n, expected[version].value, `v{version}: stored value`)
		end
	end,
	["rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step"] = function()
		local log = {}
		local backend, a = D2.world(1, { version = twoSteps(log) })
		local pa, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "an empty key loads")
		local session = DM.session(a, pa)
		DM.deepEq(session.value, { coins = 0, gems = 0 }, "the defaults")
		spec.eq(session.revision, 0, "no revision")
		spec.eq(#log, 0, "no step ran")
		spec.eq(#backend:callsTo({ server = a.id }), 1, "and no save was requested")
		spec.eq(DM.stored(backend).payload, nil, "nothing stored yet")
		backend:advance(25)
		local renewed = DM.stored(backend)
		spec.eq(renewed.schemaVersion, 2, "the first renewal stores the current version")
		DM.deepEq(renewed.payload, { coins = 0, gems = 0 }, "with the defaults")

		local second, b = D2.world(1, { version = twoSteps(log) })
		local id = D2.id(D2.WALL, "m1")
		D2.seed(second, nil, { unopened = true, epoch = 3, inbox = { D2.item(id, "grant", { amount = 1 }) } })
		local pb, loaded = DM.begin(b)
		second:advance(1)
		spec.eq(loaded.value, true, "an inbox-only envelope loads")
		DM.deepEq(DM.session(b, pb).value, { coins = 0, gems = 0 }, "the defaults, with schemaVersion 0 stored below N")
		spec.eq(#log, 0, "no step ran for it either")
		spec.eq(DM.session(b, pb).revision, 0, "no revision")
		spec.eq(DM.stored(second).inbox[1].id, id, "its inbox entry is kept")
	end,
	["rule 74 a migration rerun after a crash reaches the same value and stores version N"] = function()
		local log = {}
		local backend, a, b = D2.world(2, { version = twoSteps(log) })
		seedOld(backend)
		backend:gate("commit", { index = 2 })
		local _, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "A migrated in memory")
		spec.eq(#log, 2, "A ran both steps")
		a.crash()
		backend:advance(1)
		local stuck = DM.stored(backend)
		spec.eq(stuck.schemaVersion, 0, "the stored record is still the old version")
		DM.deepEq(stuck.payload, OLD, "with the old payload")
		local pb = D2.joinSlow(backend, b)
		backend:advance(1)
		spec.eq(#log, 4, "B reran both steps")
		spec.eq(log[3].step, 0, "from step 0")
		spec.eq(log[3].coins, 7, "on the same input")
		spec.eq(log[4].coins, 14, "and the same intermediate")
		DM.deepEq(DM.session(b, pb).value, MIGRATED, "the same value")
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 2, "B's save stores version N")
		DM.deepEq(envelope.payload, MIGRATED, "with the migrated payload")
		spec.eq(envelope.epoch, 7, "two acquisitions since the seed's 5")
	end,
	["rule 49 inspect of an old-version record returns the migrated value and writes nothing"] = function()
		local log = {}
		local backend, a = D2.world(1, { version = twoSteps(log) })
		local seed = seedOld(backend)
		local before = backend:stored(STORE, tostring(USER))
		local result = DM.inspect(backend, a)
		spec.eq(result.kind, "ok", "readable")
		DM.deepEq(result.value, MIGRATED, "the migrated, admitted value")
		spec.eq(table.isfrozen(result.value), true, "frozen")
		spec.eq(#log, 2, "the steps ran in memory")
		local calls = backend:callsTo({ server = a.id })
		spec.eq(#calls, 1, "one call")
		spec.eq(calls[1].kind, "read", "a GetAsync")
		spec.eq(calls[1].committed, nil, "nothing committed")
		DM.deepEq(
			backend:stored(STORE, tostring(USER)),
			before,
			"the stored entry, version counter included, is unchanged"
		)
		spec.eq(DM.stored(backend).epoch, seed.epoch, "no epoch step")
		spec.eq(DM.stored(backend).lease, nil, "inspect opened nothing")
	end,
	["rule 75 a step gets a private copy and the next step sees the merged state, not its sibling's edits"] = function()
		local seen = {}
		local migrations = {
			function(old)
				old.coins = 999
				old.nested.x = 1
				old.added = true
				return {}
			end,
			function(old)
				table.insert(seen, { coins = old.coins, x = old.nested.x, added = old.added })
				return {}
			end,
		}
		local backend, a = D2.world(1, { version = { migrations = migrations } })
		D2.seed(backend, nil, { payload = { coins = 7, nested = { x = 0 } } })
		local pa = D2.join(backend, a)
		local session = DM.session(a, pa)
		spec.eq(session.value.coins, 7, "the loaded value is unchanged")
		spec.eq(session.value.nested.x, 0, "also nested")
		spec.eq(session.value.added, nil, "and no key was added")
		spec.eq(seen[1].coins, 7, "step 1 saw the merged state")
		spec.eq(seen[1].x, 0, "nested too")
		spec.eq(seen[1].added, nil, "without step 0's edits")
		backend:advance(1)
		DM.deepEq(DM.stored(backend).payload, { coins = 7, nested = { x = 0 } }, "and so is the stored record")
		spec.eq(DM.stored(backend).schemaVersion, 2, "the empty patches only marked the version")
	end,
	["rule 75 a patch merges objects, replaces everything else verbatim and keeps omitted keys"] = function()
		local migrations = {
			function(old)
				return {
					coins = 6,
					renamed = old.gone,
					profile = { stats = { hp = 9 }, tags = { "q" }, scalar = { deep = true } },
					list = {},
					grid = { 7 },
				}
			end,
			function()
				return { profile = { stats = 5 }, keep = {} }
			end,
		}
		local backend, a = D2.world(1, { version = { migrations = migrations } })
		D2.seed(backend, nil, {
			payload = {
				coins = 5,
				keep = "kept",
				gone = 1,
				profile = { name = "a", tags = { "x", "y", "z" }, stats = { hp = 1, mp = 2 }, scalar = 4 },
				list = { 1, 2, 3 },
				grid = { x = 1 },
			},
		})
		local pa = D2.join(backend, a)
		DM.deepEq(DM.session(a, pa).value, {
			coins = 6,
			keep = {},
			gone = 1,
			renamed = 1,
			profile = { name = "a", tags = { "q" }, stats = 5, scalar = { deep = true } },
			list = {},
			grid = { 7 },
		}, "the merged record")
		backend:advance(1)
		spec.eq(DM.stored(backend).payload.gone, 1, "a step cannot delete a key: the old name of a rename stays")
		spec.eq(DM.stored(backend).payload.renamed, 1, "the new name is written")
		spec.eq(DM.stored(backend).payload.profile.name, "a", "an omitted nested key is kept")
		spec.list(DM.stored(backend).payload.profile.tags, { "q" }, "the array is replaced, not merged")
	end,
	["rule 75 an empty patch only marks the version"] = function()
		local backend, a = D2.world(1, { version = { migrations = {
			function()
				return {}
			end,
		} } })
		D2.seed(backend, nil, { payload = { coins = 4, extra = { 1, 2 } } })
		local pa = D2.join(backend, a)
		spec.eq(DM.session(a, pa).revision, 1, "still a revision")
		backend:advance(1)
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 1, "the version moved")
		DM.deepEq(envelope.payload, { coins = 4, extra = { 1, 2 } }, "the payload did not")
	end,
	["rule 75 a step that throws, yields or returns a non-record fails the load at once and leaves the data untouched"] = function()
		local payload = { coins = 7, gems = 3, secret = SECRET }
		local tails = {
			throws = "threw",
			yields = "yielded",
			waits = "yielded",
			string = "returned string, not a record",
			number = "returned number, not a record",
			array = "returned table, not a record",
			["nil"] = "returned nil, not a record",
		}
		for _, mode in { "throws", "yields", "waits", "string", "number", "array", "nil" } do
			for failing = 0, 1 do
				local label = `{mode} at step {failing}`
				local holder, ran = {}, {}
				local backend, a = D2.world(1, { version = failingShape(mode, failing, holder, ran) })
				holder.realm = a
				D2.seed(backend, nil, untouchedSeed(payload))
				local pa, handle = DM.begin(a)
				backend:advance(1)
				DM.assertRefused(backend, a, pa, handle, a.limits.LOAD_FAILED_MESSAGE, untouchedSeed(payload), label)
				spec.list(ran, if failing == 0 then { 0 } else { 0, 1 }, `{label}: steps that ran`)
				local messages = DM.logged(a, "migrations[")
				spec.eq(#messages, 1, `{label}: one diagnostic`)
				spec.eq(
					messages[1],
					`{STORE}: {USER}'s record did not load: {STORE}: migrations[{failing}] {tails[mode]}`,
					`{label}: the diagnostic`
				)
				spec.has(messages[1], STORE, `{label}: names the store`)
				spec.has(messages[1], tostring(USER), `{label}: names the user`)
				spec.eq(a.engine.logged(`migrations[{1 - failing}]`), 0, `{label}: and not the other step`)
				spec.eq(a.engine.logged(SECRET), 0, `{label}: no data value in any diagnostic`)
				spec.eq(a.engine.logged("secret-boom"), 0, `{label}: nor the step's error text`)
			end
		end
	end,
	["rule 76 a default fills a field missing from a frozen table a migration step returned"] = function()
		local settings = DM.object({ vibration = n.bool, volume = n.f64 })
		local shape = {
			node = DM.object({ coins = n.f64, settings = settings }),
			migrations = {
				function()
					return { settings = table.freeze({ vibration = false }) }
				end,
			},
		}
		local backend, a =
			D2.world(1, { version = shape, defaults = { coins = 0, settings = { vibration = true, volume = 0.5 } } })
		D2.seed(backend, nil, untouchedSeed({ coins = 1 }))
		local player = DM.load(backend, a)
		local value = DM.session(a, player).value
		spec.eq(value.settings.vibration, false, "the step's field")
		spec.eq(value.settings.volume, 0.5, "the default fills the field the step left out")
		spec.eq(value.coins, 1, "and the rest is kept")
	end,
	["rule 110 a failed migration and a failed admission are diagnosed under separate codes"] = function()
		local shape = {
			node = recordNode(),
			migrations = {
				function(old)
					if old.coins == 13 then
						error("secret-boom")
					elseif old.coins == 14 then
						coroutine.yield()
					elseif old.coins == 15 then
						return "not a record"
					end
					return { coins = old.coins + 1 }
				end,
			},
		}
		local backend, a = D2.world(1, { version = shape })
		D2.seed(backend, 4120, untouchedSeed({ coins = 13, gems = 1 }))
		D2.seed(backend, 4123, untouchedSeed({ coins = 14, gems = 1 }))
		D2.seed(backend, 4124, untouchedSeed({ coins = 15, gems = 1 }))
		D2.seed(backend, 4121, untouchedSeed({ coins = 2, gems = SECRET }))
		D2.seed(backend, 4122, { schemaVersion = 1, payload = { coins = 2, gems = SECRET }, epoch = 5 })
		for _, user in { 4120, 4123, 4124, 4121, 4122 } do
			local player = DM.begin(a, user)
			backend:advance(1)
			spec.eq(a.kicked(player), a.limits.LOAD_FAILED_MESSAGE, `kick of {user}`)
		end
		local migration = DM.logged(a, "did not load: FD.Players: migrations[0] threw")
		spec.eq(#migration, 1, "the thrown step is diagnosed")
		spec.has(migration[1], "4120", "for the first user")
		spec.eq(
			#DM.logged(a, "migrations[0] yielded"),
			0,
			"and a yielding step shares its code, so it is not logged again"
		)
		spec.eq(#DM.logged(a, "not a record"), 0, "as does a step that returns a non-record")
		local admission = DM.logged(a, "did not load: FD.Players.gems: expected number, got string")
		spec.eq(#admission, 1, "and the failed admissions are diagnosed once under their own code")
		spec.has(admission[1], "4121", "for the user whose record was migrated")
		spec.has(admission[1], "(after migrating from version 0)", "saying so")
		local unmigrated = DM.logged(a, "4122")
		spec.eq(#unmigrated, 0, "the user whose record is current shares that code and is not logged again")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 110 an unreadable envelope is diagnosed once per user with the store and the user, and kicks at once"] = function()
		local backend, a = D2.world(1, { version = { node = recordNode() } })
		for _, user in { 4120, 4121 } do
			D2.seed(backend, user, { format = 2, payload = { coins = 3, gems = SECRET } })
		end
		local first = DM.begin(a, 4120)
		backend:advance(1)
		spec.eq(a.kicked(first), a.limits.LOAD_FAILED_MESSAGE, "the first user is kicked as failed")
		a.leave(first)
		local again = DM.begin(a, 4120)
		backend:advance(1)
		spec.eq(a.kicked(again), a.limits.LOAD_FAILED_MESSAGE, "and again on a rejoin")
		local second = DM.begin(a, 4121)
		backend:advance(1)
		spec.eq(a.kicked(second), a.limits.LOAD_FAILED_MESSAGE, "the second user is kicked as failed")
		local messages = DM.logged(a, "record is unreadable: ")
		spec.eq(#messages, 2, "one diagnostic per user, not per attempt")
		local one, two = "FD.Players: 4120's record is unreadable: ", "FD.Players: 4121's record is unreadable: "
		spec.eq(string.sub(messages[1], 1, #one), one, "the first")
		spec.eq(string.sub(messages[2], 1, #two), two, "the second")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 110 an unreadable envelope names the bad field and never echoes the stored kind or format"] = function()
		for field, stored in { kind = "ZZSECRETKIND", format = "ZZSECRETFORMAT" } do
			local backend, a = D2.world(1, { version = { node = recordNode() } })
			D2.seed(backend, nil, { [field] = stored, payload = { coins = 3, gems = 4 } })
			local player = DM.begin(a)
			backend:advance(1)
			spec.eq(a.kicked(player), a.limits.LOAD_FAILED_MESSAGE, `a bad {field} fails the load`)
			local messages = DM.logged(a, "record is unreadable: ")
			spec.eq(#messages, 1, "one diagnostic")
			spec.has(messages[1], `record is unreadable: bad {field}`, "naming the field")
			spec.eq(a.engine.logged(stored), 0, `never the stored {field}`)
		end
	end,
	["rule 110 A18 an unreadable envelope never echoes a non-table envelope, a payload or a counter"] = function()
		local cases = {
			{ label = "a non-table envelope", stored = "ZZSECRETENVELOPE", reason = "not a table" },
			{ label = "a string payload", fields = { payload = "ZZSECRETPAYLOAD" }, reason = "bad payload" },
			{ label = "schemaVersion", fields = { schemaVersion = "ZZSECRETVERSION" }, reason = "bad schemaVersion" },
			{ label = "epoch", fields = { epoch = "ZZSECRETEPOCH" }, reason = "bad epoch" },
			{ label = "seenFloor", fields = { seenFloor = "ZZSECRETFLOOR" }, reason = "bad seenFloor" },
		}
		for _, case in cases do
			local backend, a = D2.world(1, { version = { node = recordNode() } })
			if case.stored then
				backend:seed("FD.Players", "4120", case.stored)
			else
				local fields = table.clone(case.fields)
				fields.payload = fields.payload or { coins = 3, gems = 4 }
				D2.seed(backend, nil, fields)
			end
			local player = DM.begin(a)
			backend:advance(1)
			spec.eq(a.kicked(player), a.limits.LOAD_FAILED_MESSAGE, `{case.label} fails the load`)
			local messages = DM.logged(a, "record is unreadable: ")
			spec.eq(#messages, 1, `{case.label}: one diagnostic`)
			spec.has(messages[1], `record is unreadable: {case.reason}`, `{case.label}: naming the field`)
			spec.eq(a.engine.logged("ZZSECRET"), 0, `{case.label}: never the stored value`)
		end
	end,
	["rule 110 a failed admission of a record at the current version names no migration"] = function()
		local shape = {
			node = recordNode(),
			migrations = {
				function(old)
					return { coins = old.coins }
				end,
			},
		}
		local backend, a = D2.world(1, { version = shape })
		D2.seed(backend, nil, { schemaVersion = 1, payload = { coins = 2, gems = SECRET }, epoch = 5 })
		DM.begin(a)
		backend:advance(1)
		local messages = DM.logged(a, "did not load")
		spec.eq(#messages, 1, "one diagnostic")
		spec.has(messages[1], "FD.Players.gems: expected number, got string", "the path and reason")
		spec.eq(string.find(messages[1], "migrating", 1, true), nil, "and no mention of a migration")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step"] = function()
		local holder, ran = {}, {}
		local backend, a = D2.world(1, { version = failingShape("throws", 1, holder, ran) })
		D2.seed(backend, nil, untouchedSeed({ coins = 7, secret = SECRET }))
		local first, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, false, "refused")
		a.leave(first)
		for _ = 1, 2 do
			local again, retry = DM.begin(a)
			backend:advance(1)
			spec.eq(retry.value, false, "refused again")
			spec.eq(a.kicked(again), a.limits.LOAD_FAILED_MESSAGE, "kicked again")
			a.leave(again)
		end
		spec.eq(#ran, 6, "the step ran on every attempt")
		local messages = DM.logged(a, "migrations[1]")
		spec.eq(#messages, 1, "one diagnostic for three attempts")
		spec.has(messages[1], `{STORE}`, "the store")
		spec.has(messages[1], `{USER}`, "the user")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 110 a failed migration is diagnosed once per store and code, not once per user"] = function()
		local holder, ran = {}, {}
		local backend, a = D2.world(1, { version = failingShape("throws", 1, holder, ran) })
		for _, user in { 4120, 4121, 4122 } do
			D2.seed(backend, user, untouchedSeed({ coins = 7, secret = SECRET }))
			local player, handle = DM.begin(a, user)
			backend:advance(1)
			spec.eq(handle.value, false, `refused {user}`)
			spec.eq(a.kicked(player), a.limits.LOAD_FAILED_MESSAGE, `kicked {user}`)
		end
		spec.eq(#DM.logged(a, "migrations[1]"), 1, "one diagnostic for three users")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 77 and durability sequence an older build meets a newer version"] = function()
		local fills = 0
		local function fillTips(old)
			fills += 1
			local jobs = {}
			for index, job in old.career.jobs do
				local copy = table.clone(job)
				copy.tips = 0
				jobs[index] = copy
			end
			return { career = { jobs = jobs } }
		end
		local function root(job)
			return DM.object({ career = DM.object({ jobs = n.array(job) }), coins = n.f64 })
		end
		local jobN1 = DM.object({ coins = n.f64, id = n.string, rating = DM.opt(n.f64) })
		local jobN2 = DM.object({ coins = n.f64, id = n.string, rating = DM.opt(n.f64), tips = n.f64 })
		local shapeN2 = { node = root(jobN2), migrations = { fillTips } }
		local shapeN1 = { node = root(jobN1), version = 0 }
		local defaults = { career = { jobs = {} }, coins = 0 }
		local backend, d, e, f = D2.world(3, { versions = { shapeN2, shapeN1, shapeN2 }, defaults = defaults })
		local record = {
			career = { jobs = { { id = "j1", coins = 10 }, { id = "j3", coins = 40, rating = 5 } } },
			coins = 100,
		}
		D2.seed(backend, nil, { schemaVersion = 0, epoch = 8, payload = record })

		local pd = DM.load(backend, d, nil, 0.5)
		spec.eq(fills, 1, "D ran fillTips once")
		local migrated = DM.stored(backend)
		spec.eq(migrated.schemaVersion, 1, "version 1 committed with the payload")
		spec.eq(migrated.epoch, 9, "epoch 9")
		spec.eq(migrated.lease.serverId, d.id, "lease D")
		local expectedJobs = { { id = "j1", coins = 10, tips = 0 }, { id = "j3", coins = 40, rating = 5, tips = 0 } }
		DM.deepEq(
			migrated.payload,
			{ career = { jobs = expectedJobs }, coins = 100 },
			"each element copied whole, rating kept"
		)

		backend:advance(300 - backend.now)
		DM.leave(backend, d, pd)
		local released = DM.stored(backend)
		spec.eq(released.lease, nil, "D released")
		spec.eq(released.schemaVersion, 1, "version 1")
		backend:advance(330 - backend.now)
		local before = backend:stored(STORE, tostring(USER))

		local pe, loading = DM.begin(e)
		backend:advance(1)
		spec.eq(loading.value, false, "E's load fails")
		local kick = DM.kick(e, pe)
		spec.eq(kick.message, e.limits.SERVER_UPDATING_MESSAGE, "kicked as updating")
		assert(kick.at - 330 < 0.5, `kicked at once (t = {kick.at})`)
		DM.deepEq(
			backend:stored(STORE, tostring(USER)),
			before,
			"the stored entry is unchanged: lease, epoch, version counter"
		)
		local calls = backend:callsTo({ server = e.id })
		spec.eq(#calls, 1, "E made one call: the acquisition")
		spec.eq(calls[1].committed, nil, "it wrote nothing")

		local inspected = DM.inspect(backend, e)
		spec.eq(inspected.kind, "unavailable", "inspect on E is unavailable")
		local resumed = e.run(function()
			return e.store:resume(pe)
		end)
		backend:advance(1)
		spec.eq(resumed.value, false, "resume on E does what a load does")
		local kicks = 0
		for _, entry in e.kicks do
			if entry.player == pe then
				kicks += 1
				spec.eq(entry.message, e.limits.SERVER_UPDATING_MESSAGE, "every kick is the updating message")
			end
		end
		spec.eq(kicks, 2, "one for the load and one for the resume")
		for _, call in backend:callsTo({ server = e.id }) do
			spec.eq(call.committed, nil, `E's call {call.index} ({call.kind}) wrote nothing`)
		end
		DM.deepEq(backend:stored(STORE, tostring(USER)), before, "still unchanged")

		local decl = D2.message("grant")
		local id = D2.id(D2.wallAt(backend), "m1")
		local sent = D2.send(e, decl, { amount = 1 }, id)
		backend:advance(1)
		spec.eq(sent.value.kind, "ok", "a send from E appends")
		local appended = DM.stored(backend)
		spec.eq(#appended.inbox, 1, "one item queued")
		spec.eq(appended.inbox[1].version, 0, "stamped with E's version")
		spec.eq(appended.schemaVersion, 1, "the stored version is untouched")
		spec.eq(appended.epoch, 9, "and the epoch")
		DM.deepEq(appended.payload, migrated.payload, "and the payload")

		backend:advance(345 - backend.now)
		local received = D2.receiver(f, decl)
		local pf, floaded = DM.begin(f)
		backend:advance(1)
		spec.eq(floaded.value, true, "F loads version 1 normally")
		spec.eq(fills, 1, "and runs no step")
		local after = DM.stored(backend)
		spec.eq(after.epoch, 10, "epoch 10")
		spec.eq(after.lease.serverId, f.id, "lease F")
		spec.eq(#received.calls, 1, "F applied E's message")
		spec.eq(after.payload.coins, 101, "to the payload")
		spec.list(after.seen, { id }, "and acknowledged it")
		spec.eq(after.schemaVersion > 0, true, "the stored version stays above E's")
		spec.eq(DM.session(f, pf).value.career.jobs[2].tips, 0, "F's value holds the filled field")
	end,
	["rule 77 a save never overwrites a record that became newer while the session was live"] = function()
		local backend, a = D2.world(1, { version = { version = 1 } })
		D2.seed(backend, nil, { schemaVersion = 1, payload = { coins = 7, gems = 3 }, epoch = 5 })
		local player, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "loaded")
		local entry = backend:entry(DM.STORE, tostring(DM.USER))
		entry.value.schemaVersion = 9
		entry.value.payload = { coins = 777, gems = 3 }
		DM.write(a, player, function(current)
			return { coins = current.coins + 1, gems = current.gems }
		end)
		backend:advance(25)
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 9, "the newer version is untouched")
		spec.eq(envelope.payload.coins, 777, "and the newer payload is not overwritten")
	end,
	["rule 77 clearQuarantined and send work on a newer record without touching its payload or lease"] = function()
		local backend, e = D2.world(1, { version = { version = 2 } })
		local id = D2.id(D2.WALL - 20, "q1")
		local lease = { token = "foreign", serverId = "x", jobId = "y", expiresAt = D2.WALL + 1000 }
		D2.seed(backend, nil, {
			schemaVersion = 9,
			epoch = 4,
			lease = lease,
			payload = { coins = 7, gems = 0 },
			quarantine = { D2.item(id, "grant", { amount = 6 }, { reason = "attempts", attempts = 5, generation = 5 }) },
		})
		local cleared = e.run(function()
			return e.store.inbox:clearQuarantined(USER, { id }, "retry")
		end)
		backend:advance(1)
		spec.eq(cleared.value.kind, "ok", "clearQuarantined works")
		spec.eq(cleared.value.value, 1, "one item moved")
		local envelope = DM.stored(backend)
		spec.eq(#envelope.quarantine, 0, "it left the quarantine")
		spec.eq(#envelope.inbox, 1, "and is live again")
		spec.eq(envelope.inbox[1].generation, 6, "at the next generation")
		spec.eq(envelope.schemaVersion, 9, "the version is untouched")
		spec.eq(envelope.epoch, 4, "so is the epoch")
		DM.deepEq(envelope.lease, lease, "and the lease")
		DM.deepEq(envelope.payload, { coins = 7, gems = 0 }, "and the payload")
	end,
}
