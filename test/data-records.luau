local spec = require("./lib/load")
local D2 = require("./lib/data-d2")
local R = require("./lib/data-records")

local WALL = R.WALL

local function noop(current)
	return current
end

local function payload(fields)
	local value = { entries = {}, tallies = {}, total = 0 }
	for key, entry in fields or {} do
		value[key] = entry
	end
	return value
end

local SCENARIOS = {
	{ key = "format", fields = { format = 2, payload = payload() } },
	{ key = "wrongkind", fields = { kind = "profiles", payload = payload() } },
	{ key = "badring", fields = { operations = { 5 }, payload = payload() } },
	{ key = "badshape", fields = { payload = payload({ total = R.SECRET }) } },
	{ key = "newer", fields = { schemaVersion = 3, payload = payload({ extra = R.SECRET }) } },
}

return {
	["rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works"] = function()
		local backend, realm, store = R.world()
		local fifty = string.rep("k", 50)
		R.seed(backend, fifty, { payload = payload({ total = 4 }) })
		for _, bad in { "", string.rep("k", 51), 7 } do
			local label = tostring(bad)
			local ok, err = R.try(backend, realm, function()
				return store:read(bad)
			end)
			spec.eq(ok, false, `read of key {#label}`)
			spec.has(err, "FD.CommandLog: a key is 1 to 50 bytes", `read of key {#label}`)
			ok, err = R.try(backend, realm, function()
				return store:update(bad, noop)
			end)
			spec.eq(ok, false, `update of key {#label}`)
			spec.has(err, "FD.CommandLog: a key is 1 to 50 bytes", `update of key {#label}`)
		end
		spec.eq(#backend.calls, 0, "a bad key sends nothing")
		local result = R.read(backend, realm, store, fifty)
		spec.eq(result.kind, "ok", "a 50 byte key reads")
		spec.eq(result.value.total, 4, "and finds its record")
		local updated = R.update(backend, realm, store, fifty, noop)
		spec.eq(updated.kind, "ok", "and updates")
	end,
	["rule 86 an absent key and an envelope with no payload read undefined, never defaults"] = function()
		local backend, realm, store = R.world()
		local absent = R.read(backend, realm, store, "absent")
		spec.eq(absent.kind, "ok", "an absent key is ok")
		spec.eq(absent.value, nil, "with no value")
		local marker = R.id(WALL - 5, "m1")
		R.seed(backend, "marked", { operations = { marker } })
		local marked = R.read(backend, realm, store, "marked")
		spec.eq(marked.kind, "ok", "an envelope with only an operation marker is ok")
		spec.eq(marked.value, nil, "with no value")
		R.same(R.stored(backend, "marked"), R.envelope({ operations = { marker } }), "and it is untouched")
		spec.eq(R.updates(backend), 0, "reads write nothing")
	end,
	["rule 86 an unreadable record and a newer record are unavailable and nothing is written"] = function()
		local backend, realm, store = R.world()
		for _, scenario in SCENARIOS do
			R.seed(backend, scenario.key, scenario.fields)
			local result = R.read(backend, realm, store, scenario.key)
			spec.eq(result.kind, "unavailable", scenario.key)
			spec.eq(result.value, nil, `{scenario.key} carries no value`)
			R.same(R.stored(backend, scenario.key), R.envelope(scenario.fields), `{scenario.key} is untouched`)
		end
		spec.eq(R.updates(backend), 0, "no update was sent")
	end,
	["rule 86 an older record reads migrated and admitted and the read writes nothing"] = function()
		local backend, realm, store = R.world()
		local fields = { schemaVersion = 0, payload = { entries = { { id = "a" } }, total = 2 } }
		R.seed(backend, "v0", fields)
		R.seed(backend, "v1", { schemaVersion = 1, payload = { entries = {}, total = 2 } })
		local old = R.read(backend, realm, store, "v0")
		spec.eq(old.kind, "ok", "version 0 reads")
		R.same(
			old.value,
			{ entries = { { id = "a" } }, label = "m3", tallies = {}, total = 3 },
			"both steps ran in order and the default filled tallies"
		)
		local middle = R.read(backend, realm, store, "v1")
		R.same(
			middle.value,
			{ entries = {}, label = "m2", tallies = {}, total = 2 },
			"version 1 runs only the second step"
		)
		R.same(R.stored(backend, "v0"), R.envelope(fields), "version 0 stays as stored")
		spec.eq(R.stored(backend, "v1").schemaVersion, 1, "version 1 stays as stored")
		spec.eq(R.updates(backend), 0, "no update was sent")
	end,
	["rule 86 a read returns a deeply frozen table with unknown keys in place at every depth"] = function()
		local backend, realm, store = R.world()
		R.seed(backend, "deep", {
			payload = {
				entries = { { id = "a", note = "n1" } },
				extra = { deep = { k = 1 } },
				tallies = { x = { crit = 7, hits = 1 } },
				total = 2,
			},
		})
		local result = R.read(backend, realm, store, "deep")
		R.same(result.value, {
			entries = { { id = "a", note = "n1" } },
			extra = { deep = { k = 1 } },
			tallies = { x = { crit = 7, hits = 1 } },
			total = 2,
		}, "every key stays where it was")
		R.frozen(result.value, "the read")
		local again = R.read(backend, realm, store, "deep")
		spec.eq(rawequal(again.value, result.value), false, "each read is its own copy")
		spec.eq(
			pcall(function()
				result.value.total = 9
			end),
			false,
			"mutating a read throws"
		)
	end,

	["rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "a1")
		local seen, runs = nil, 0
		local result = R.update(backend, realm, store, "fresh", function(current)
			runs += 1
			seen = current
			return { entries = { { id = "a" } }, tallies = {}, total = current.total + 5 }
		end, id)
		spec.eq(runs, 1, "change ran once")
		R.same(seen, { entries = {}, tallies = {}, total = 0 }, "change got the defaults")
		R.frozen(seen, "the defaults")
		spec.eq(result.kind, "ok", "ok")
		spec.eq(result.replayed, false, "a new id is not a replay")
		R.same(result.value, { entries = { { id = "a" } }, tallies = {}, total = 5 }, "the result")
		R.frozen(result.value, "the result")
		R.same(
			R.stored(backend, "fresh"),
			R.envelope({
				schemaVersion = 2,
				payload = { entries = { { id = "a" } }, tallies = {}, total = 5 },
				operations = { id },
			}),
			"one envelope with the payload, the version and the id"
		)
		spec.eq(R.updates(backend), 1, "one update")
		spec.eq(R.committed(backend), 1, "one write")
	end,
	["rule 87 the result keeps unknown keys at every depth through a clone and edit"] = function()
		local backend, realm, store = R.world()
		R.seed(backend, "deep", {
			payload = {
				entries = { { id = "a", note = "n1" } },
				extra = { deep = { k = 1 } },
				tallies = { x = { crit = 7, hits = 1 } },
				total = 2,
			},
		})
		local id = R.id(WALL - 5, "u1")
		local result = R.update(backend, realm, store, "deep", function(current)
			local value = table.clone(current)
			value.total = current.total + 1
			local entry = table.clone(current.entries[1])
			entry.id = "z"
			value.entries = { entry }
			local tally = table.clone(current.tallies.x)
			tally.hits = 9
			value.tallies = { x = tally }
			return value
		end, id)
		local expected = {
			entries = { { id = "z", note = "n1" } },
			extra = { deep = { k = 1 } },
			tallies = { x = { crit = 7, hits = 9 } },
			total = 3,
		}
		R.same(result.value, expected, "the result")
		R.frozen(result.value, "the result")
		R.same(
			R.stored(backend, "deep"),
			R.envelope({ payload = expected, operations = { id } }),
			"unknown keys are stored"
		)
	end,
	["rule 87 a result rawequal to the current record stores no payload and records the id, whether passed or made"] = function()
		local backend, realm, store = R.world()
		local current = payload({ entries = { { id = "a" } }, total = 2 })
		R.seed(backend, "same", { payload = current })
		local id = R.id(WALL - 5, "n1")
		local runs = 0
		local function count(value)
			runs += 1
			return value
		end
		local first = R.update(backend, realm, store, "same", count, id)
		spec.eq(first.kind, "ok", "ok")
		spec.eq(first.replayed, false, "a new id is not a replay")
		R.same(
			R.stored(backend, "same"),
			R.envelope({ payload = current, operations = { id } }),
			"the payload is as it was and the id is recorded"
		)
		local retry = R.update(backend, realm, store, "same", count, id)
		spec.eq(retry.kind, "ok", "a retry is ok")
		spec.eq(retry.replayed, true, "a replay")
		spec.eq(runs, 1, "the retry did not run change")

		local wall = D2.wallAt(backend)
		local made = R.update(backend, realm, store, "same", count)
		spec.eq(made.replayed, false, "the made id is new")
		local operations = R.stored(backend, "same").operations
		spec.eq(#operations, 2, "the framework's id is recorded too")
		local second = string.match(operations[2], "^o1%-(%w+)%-%w+$")
		spec.eq(second ~= nil and R.decode36(second), wall, "made at the realm's wall second")
		R.same(R.stored(backend, "same").payload, current, "the payload is still as it was")
	end,
	["rule 87 on an absent key a no-op records the id in an envelope with no payload that reads undefined"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "n2")
		local result = R.update(backend, realm, store, "gone", noop, id)
		spec.eq(result.kind, "ok", "ok")
		R.same(result.value, { entries = {}, tallies = {}, total = 0 }, "the defaults come back")
		R.same(
			R.stored(backend, "gone"),
			R.envelope({ schemaVersion = 0, operations = { id } }),
			"an unopened envelope holding only the id"
		)
		local read = R.read(backend, realm, store, "gone")
		spec.eq(read.kind, "ok", "reads ok")
		spec.eq(read.value, nil, "and undefined")
	end,
	["rule 87 a change that returns something other than a table throws naming the store"] = function()
		local backend, realm, store = R.world()
		local ok, err = R.try(backend, realm, function()
			return store:update("odd", function()
				return 5
			end, R.id(WALL - 5, "v1"))
		end)
		spec.eq(ok, false, "the call throws")
		spec.has(err, "FD.CommandLog: a store's value is a record, got number", "with the store and what it got")
		spec.eq(R.stored(backend, "odd"), nil, "nothing was stored")
	end,
	["rule 87 a throw in change cancels the write, is rethrown at the call and leaves the id unrecorded"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "t1")
		local ok, err = R.try(backend, realm, function()
			return store:update("boom", function()
				error("exploded-4271")
			end, id)
		end)
		spec.eq(ok, false, "the call throws")
		spec.has(err, "exploded-4271", "with the callback's text")
		spec.eq(R.stored(backend, "boom"), nil, "nothing was stored")
		spec.eq(R.committed(backend), 0, "nothing was written")
		local runs = 0
		local retry = R.update(backend, realm, store, "boom", function(current)
			runs += 1
			return current
		end, id)
		spec.eq(retry.replayed, false, "the id was not recorded")
		spec.eq(runs, 1, "so change ran")
	end,
	["rule 87 a change that yields is cancelled and thrown at the caller"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "y1")
		local ok, err = R.try(backend, realm, function()
			return store:update("yield", function(current)
				coroutine.yield()
				return current
			end, id)
		end)
		spec.eq(ok, false, "the call throws")
		spec.has(err, "yield", "naming the yield")
		spec.eq(R.stored(backend, "yield"), nil, "nothing was stored")
		spec.eq(R.committed(backend), 0, "nothing was written")
	end,
	["rule 13 a generic update or a profile write inside change throws and does not run"] = function()
		local backend, realm, store = R.world()
		realm.store = realm.profile(D2.STORE, { coins = 0, gems = 0 })
		local player = D2.join(backend, realm)
		local before = #backend.calls
		local ok, err = R.try(backend, realm, function()
			return store:update("outer", function(current)
				store:update("inner", noop)
				return current
			end)
		end)
		spec.eq(ok, false, "a nested generic update throws")
		spec.has(err, "a write inside a data callback; return the data instead", "with the rule's text")
		ok, err = R.try(backend, realm, function()
			return store:update("outer", function(current)
				realm.store:write(player, noop)
				return current
			end)
		end)
		spec.eq(ok, false, "a profile write throws")
		spec.has(err, "a write inside a data callback; return the data instead", "with the rule's text")
		spec.eq(#backend:callsTo({ key = "inner" }), 0, "the nested update sent nothing")
		spec.eq(R.stored(backend, "outer"), nil, "the outer update stored nothing")
		spec.eq(R.committed(backend), 0, "no record write")
		spec.eq(#backend.calls - before, 2, "only the two outer attempts reached storage")
	end,
	["rule 13 a generic update inside a profile data callback throws and the write is cancelled"] = function()
		local backend, realm, store = R.world()
		realm.store = realm.profile(D2.STORE, { coins = 0, gems = 0 })
		local player = D2.join(backend, realm)
		local ok, err = R.try(backend, realm, function()
			return realm.store:write(player, function(current)
				store:update("inside", noop)
				return current
			end)
		end)
		spec.eq(ok, false, "the profile write throws")
		spec.has(err, "a write inside a data callback; return the data instead", "with the rule's text")
		spec.eq(#backend:callsTo({ key = "inside" }), 0, "the generic update sent nothing")
		local after = R.call(backend, realm, function()
			return realm.store:write(player, function(current)
				local value = table.clone(current)
				value.coins = 7
				return value
			end)
		end)
		spec.eq(after, true, "the session is not left busy")
	end,
	["rule 87 change may run again: a conflict reruns it and exactly one write lands"] = function()
		local backend, realm, store = R.world()
		backend:fail("conflict", { kind = "update", store = R.STORE }, 1)
		local runs = 0
		local id = R.id(WALL - 5, "c1")
		local result = R.update(backend, realm, store, "conflict", function(current)
			runs += 1
			return { entries = current.entries, tallies = current.tallies, total = current.total + 1 }
		end, id)
		spec.eq(result.kind, "ok", "ok")
		spec.eq(runs >= 2, true, "change ran again")
		spec.eq(result.value.total, 1, "the result counts once")
		R.same(
			R.stored(backend, "conflict"),
			R.envelope({ payload = { entries = {}, tallies = {}, total = 1 }, operations = { id } }),
			"one write"
		)
		spec.eq(R.committed(backend), 1, "one commit")
	end,
	["rule 87 an older stored version writes the migrated value even when change returns it unchanged"] = function()
		local backend, realm, store = R.world()
		R.seed(backend, "v0", { schemaVersion = 0, payload = { entries = {}, total = 2 } })
		R.seed(backend, "v1", { schemaVersion = 1, payload = { entries = {}, total = 2 } })
		local id0, id1 = R.id(WALL - 5, "o0"), R.id(WALL - 5, "o1")
		local first = R.update(backend, realm, store, "v0", noop, id0)
		local second = R.update(backend, realm, store, "v1", noop, id1)
		local migrated0 = { entries = {}, label = "m3", tallies = {}, total = 3 }
		local migrated1 = { entries = {}, label = "m2", tallies = {}, total = 2 }
		R.same(first.value, migrated0, "version 0 comes back migrated")
		spec.eq(first.replayed, false, "not a replay")
		R.same(second.value, migrated1, "version 1 comes back migrated")
		R.same(
			R.stored(backend, "v0"),
			R.envelope({ schemaVersion = 2, payload = migrated0, operations = { id0 } }),
			"the migrated payload and the new version are one write"
		)
		R.same(
			R.stored(backend, "v1"),
			R.envelope({ schemaVersion = 2, payload = migrated1, operations = { id1 } }),
			"version 1 advances with its payload"
		)
	end,
	["rule 87 a record at a newer version cancels the update, returns unavailable and is diagnosed once"] = function()
		local backend, realm, store = R.world()
		local fields = { schemaVersion = 3, payload = payload({ extra = R.SECRET }) }
		R.seed(backend, "n1", fields)
		R.seed(backend, "n2", fields)
		local runs = 0
		local first = R.update(backend, realm, store, "n1", function(current)
			runs += 1
			return current
		end, R.id(WALL - 5, "w1"))
		local second = R.update(backend, realm, store, "n2", noop, R.id(WALL - 5, "w2"))
		spec.eq(first.kind, "unavailable", "the update is unavailable")
		spec.eq(second.kind, "unavailable", "also for another key")
		spec.eq(runs, 0, "change never ran")
		spec.eq(R.committed(backend), 0, "nothing was written")
		R.same(R.stored(backend, "n1"), R.envelope(fields), "the record is untouched")
		spec.eq(realm.engine.logged("written by a newer build"), 1, "diagnosed once per store")
		spec.has(R.logText(realm), "FD.CommandLog: n1 was written by a newer build", "naming the store and the key")
		local read = R.read(backend, realm, store, "n1")
		spec.eq(read.kind, "unavailable", "a read is unavailable too")
		spec.eq(realm.engine.logged("written by a newer build"), 1, "and shares the diagnostic")
		spec.eq(string.find(R.logText(realm), R.SECRET, 1, true), nil, "never a value")
	end,
	["rule 87 two realms updating one key both land and both ids are recorded"] = function()
		local backend, a, storeA = R.world()
		local b, storeB = R.second(backend)
		local idA, idB = R.id(WALL - 5, "ra"), R.id(WALL - 5, "rb")
		local seenA, seenB = {}, {}
		local gate = backend:gate("commit", { server = a.id, kind = "update" })
		local first = a.run(function()
			return storeA:update("shared", function(current)
				table.insert(seenA, current.total)
				return { entries = current.entries, tallies = current.tallies, total = current.total + 1 }
			end, idA)
		end)
		backend:advance(0.2)
		local second = R.update(backend, b, storeB, "shared", function(current)
			table.insert(seenB, current.total)
			return { entries = current.entries, tallies = current.tallies, total = current.total + 10 }
		end, idB)
		spec.eq(second.kind, "ok", "the second realm landed first")
		gate.release()
		backend:advance(1)
		spec.eq(first.done, true, "the first realm returned")
		spec.eq(first.value.kind, "ok", "and landed")
		spec.list(seenB, { 0 }, "the second realm saw the empty record")
		spec.list(seenA, { 0, 10 }, "the first reran against the newer record")
		R.same(
			R.stored(backend, "shared"),
			R.envelope({
				payload = { entries = {}, tallies = {}, total = 11 },
				operations = { idB, idA },
			}),
			"both changes and both ids"
		)
	end,

	["rule 88 an explicit id's no-op replays with the current record, which may include later writers' changes"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "p1")
		local runs = 0
		local function count(value)
			runs += 1
			return value
		end
		R.update(backend, realm, store, "later", count, id)
		R.update(backend, realm, store, "later", function(current)
			return { entries = current.entries, tallies = current.tallies, total = 9 }
		end, R.id(WALL - 5, "p2"))
		local replay = R.update(backend, realm, store, "later", count, id)
		spec.eq(replay.kind, "ok", "ok")
		spec.eq(replay.replayed, true, "a replay")
		R.same(replay.value, { entries = {}, tallies = {}, total = 9 }, "with the current stored record")
		spec.eq(runs, 1, "change ran once")
		spec.eq(R.committed(backend), 2, "the replay wrote nothing")
	end,
	["rule 86 and 87 an envelope with no payload is never newer, whatever version it carries"] = function()
		local backend, realm, store = R.world()
		local marker = R.id(WALL - 5, "n1")
		R.seed(backend, "bare", { schemaVersion = 7, operations = { marker } })
		local read = R.read(backend, realm, store, "bare")
		spec.eq(read.kind, "ok", "a read is ok")
		spec.eq(read.value, nil, "with no value")
		local id = R.id(WALL - 5, "n2")
		local updated = R.update(backend, realm, store, "bare", function(current)
			return { entries = current.entries, tallies = current.tallies, total = 4 }
		end, id)
		spec.eq(updated.kind, "ok", "an update runs")
		R.same(updated.value, { entries = {}, tallies = {}, total = 4 }, "from the defaults")
		R.same(
			R.stored(backend, "bare"),
			R.envelope({
				schemaVersion = 2,
				payload = { entries = {}, tallies = {}, total = 4 },
				operations = { marker, id },
			}),
			"and stores the record at this build's version"
		)
		spec.eq(realm.engine.logged("newer build"), 0, "nothing was diagnosed")
	end,
	["rule 88 a replay of an id held by a record stored at an older version returns the record migrated"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "q1")
		local fields = { schemaVersion = 1, payload = { entries = {}, total = 2 }, operations = { id } }
		R.seed(backend, "old", fields)
		local replay = R.update(backend, realm, store, "old", noop, id)
		spec.eq(replay.kind, "ok", "ok")
		spec.eq(replay.replayed, true, "a replay")
		R.same(
			replay.value,
			{ entries = {}, label = "m2", tallies = {}, total = 2 },
			"migrated from version 1 by one step"
		)
		R.same(R.stored(backend, "old"), R.envelope(fields), "and nothing was stored")
		spec.eq(R.committed(backend), 0, "no commit")
	end,
	["rule 88 a replay of an id held by a record that no longer loads is unavailable"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "q2")
		local fields = { schemaVersion = 2, payload = payload({ total = R.SECRET }), operations = { id } }
		R.seed(backend, "bad", fields)
		local replay = R.update(backend, realm, store, "bad", noop, id)
		spec.eq(replay.kind, "unavailable", "unavailable")
		spec.eq(replay.value, nil, "with no value")
		R.same(R.stored(backend, "bad"), R.envelope(fields), "nothing was stored")
		spec.eq(string.find(R.logText(realm), R.SECRET, 1, true), nil, "never a value")
	end,
	["rule 88 a no-op with a default id retried after an ambiguous commit replays once"] = function()
		local backend, realm, store = R.world()
		backend:fail("committed", { kind = "update", store = R.STORE }, 1)
		local runs = 0
		local result = R.update(backend, realm, store, "retry", function(current)
			runs += 1
			return current
		end, nil, 5)
		spec.eq(result.kind, "ok", "ok")
		spec.eq(result.replayed, true, "the retry found its own id")
		R.same(result.value, { entries = {}, tallies = {}, total = 0 }, "the defaults")
		spec.eq(runs, 1, "change ran once")
		spec.eq(R.updates(backend), 2, "two attempts")
		spec.eq(R.committed(backend), 1, "one write")
		local stored = R.stored(backend, "retry")
		spec.eq(#stored.operations, 1, "one id recorded")
		spec.eq(stored.payload, nil, "and no payload")
	end,
	["rule 88 a changing update with a default id retried after an ambiguous commit replays the stored record"] = function()
		local backend, realm, store = R.world()
		backend:fail("committed", { kind = "update", store = R.STORE }, 1)
		local runs = 0
		local result = R.update(backend, realm, store, "retry", function(current)
			runs += 1
			return { entries = current.entries, tallies = current.tallies, total = current.total + 1 }
		end, nil, 5)
		spec.eq(result.replayed, true, "the retry found its own id")
		R.same(result.value, { entries = {}, tallies = {}, total = 1 }, "the record the first attempt stored")
		spec.eq(runs, 1, "change ran once")
		spec.eq(R.stored(backend, "retry").payload.total, 1, "and counted once")
	end,
	["rule 88 an absent key replays defaults with replayed true and a read still returns undefined"] = function()
		local backend, realm, store = R.world()
		local id = R.id(WALL - 5, "s1")
		local first = R.update(backend, realm, store, "absent", noop, id)
		spec.eq(first.replayed, false, "the first is new")
		local second = R.update(backend, realm, store, "absent", noop, id)
		spec.eq(second.kind, "ok", "ok")
		spec.eq(second.replayed, true, "the second is a replay")
		R.same(second.value, { entries = {}, tallies = {}, total = 0 }, "carrying the defaults")
		local read = R.read(backend, realm, store, "absent")
		spec.eq(read.kind, "ok", "a read is ok")
		spec.eq(read.value, nil, "and undefined")
	end,
	["rule 88 the ring keeps the last 256 ids in order and eviction raises the floor to the evicted id's creation time"] = function()
		local backend, realm, store = R.world()
		local ids = {}
		for index = 1, 258 do
			ids[index] = R.id(WALL - 3000 + index, `r{index}`)
		end
		for index = 1, 258 do
			local result = R.update(backend, realm, store, "ring", noop, ids[index], 0.2)
			spec.eq(result.kind, "ok", `update {index}`)
		end
		local expected = {}
		for index = 3, 258 do
			table.insert(expected, ids[index])
		end
		local envelope = R.stored(backend, "ring")
		R.same(
			envelope,
			R.envelope({ schemaVersion = 0, operations = expected, operationFloor = WALL - 3000 + 2 }),
			"the last 256 ids in order and the floor at the second evicted id's creation"
		)

		local committed = R.committed(backend)
		local atFloor = R.update(backend, realm, store, "ring", noop, R.id(WALL - 3000 + 2, "at"))
		spec.eq(atFloor.kind, "expired", "an id created at the floor is expired")
		local evicted = R.update(backend, realm, store, "ring", noop, ids[1])
		spec.eq(evicted.kind, "expired", "an evicted id is expired, not a replay")
		R.same(R.stored(backend, "ring"), envelope, "and nothing changed")
		spec.eq(R.committed(backend), committed, "no write")

		local above = R.update(backend, realm, store, "ring", noop, R.id(WALL - 3000 + 3, "up"))
		spec.eq(above.kind, "ok", "an id created above the floor is accepted")
		spec.eq(R.stored(backend, "ring").operationFloor, WALL - 3000 + 3, "and the next eviction raised the floor")
	end,
	["rule 88 an id older than 3,600 seconds is expired before any backend call and the boundary is exact"] = function()
		local function attempt(offset: number, advance: number)
			local backend, realm, store = R.world()
			backend:advance(advance)
			local result = R.update(backend, realm, store, "age", noop, R.id(WALL - offset, "g1"))
			return result, R.updates(backend)
		end
		local expired, calls = attempt(3601, 0)
		spec.eq(expired.kind, "expired", "3,601 seconds old")
		spec.eq(calls, 0, "refused before any call")
		local edge, edgeCalls = attempt(3600, 0)
		spec.eq(edge.kind, "ok", "exactly 3,600 seconds old is accepted")
		spec.eq(edgeCalls, 1, "and sent")
		local late, lateCalls = attempt(3600, 0.5)
		spec.eq(late.kind, "expired", "3,600.5 seconds old")
		spec.eq(lateCalls, 0, "refused before any call")
		local under, underCalls = attempt(3599, 0.5)
		spec.eq(under.kind, "ok", "3,599.5 seconds old is accepted")
		spec.eq(underCalls, 1, "and sent")
	end,
	["rule 88 an id created five seconds ahead is accepted and six seconds ahead is expired before any call"] = function()
		local ahead, calls = {}, {}
		for _, offset in { 5, 6 } do
			local backend, realm, store = R.world()
			ahead[offset] = R.update(backend, realm, store, "future", noop, R.id(WALL + offset, "f1"))
			calls[offset] = R.updates(backend)
		end
		spec.eq(ahead[5].kind, "ok", "five seconds ahead is within the skew")
		spec.eq(calls[5], 1, "and sent")
		spec.eq(ahead[6].kind, "expired", "six seconds ahead is expired")
		spec.eq(calls[6], 0, "refused before any call")
	end,
	["rule 110 an update and a read of a record that does not load carry the same exact diagnostic"] = function()
		local expected = "FD.CommandLog: bad did not load: FD.CommandLog.total: expected number, got string"
		local fields = { payload = payload({ total = R.SECRET }) }
		local backend, realm, store = R.world()
		R.seed(backend, "bad", fields)
		local updated = R.update(backend, realm, store, "bad", noop, R.id(WALL - 5, "z1"))
		spec.eq(updated.kind, "unavailable", "the update is unavailable")
		spec.eq(R.logText(realm), expected, "the diagnostic")
		local readBackend, readRealm, readStore = R.world()
		R.seed(readBackend, "bad", fields)
		local read = R.read(readBackend, readRealm, readStore, "bad")
		spec.eq(read.kind, "unavailable", "the read is unavailable")
		spec.eq(R.logText(readRealm), expected, "and so is its diagnostic")
	end,
	["rule 13 a worker that ran a data callback is clean when the pool hands it to the next call"] = function()
		local backend, realm, store = R.world()
		R.update(backend, realm, store, "first", noop, R.id(WALL - 5, "w1"))
		local runner = realm.graph.module("internal/runner")
		local outcome = R.call(backend, realm, function()
			local status, value = runner.call(function()
				return store:update("second", noop, R.id(WALL - 5, "w2"))
			end, nil, nil)
			return { status = status, value = value }
		end, 0.1)
		spec.eq(
			string.find(tostring(outcome.value), "a write inside a data callback", 1, true),
			nil,
			"no false callback error"
		)
		spec.eq(outcome.status, 2, "the call yielded: its worker is not a data callback thread")
	end,
	["rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing"] = function()
		local backend, realm, store = R.world()
		local long = R.ring(1, WALL - 5, "n", 65)[1]
		local exact = R.ring(1, WALL - 5, "n", 64)[1]
		spec.eq(#long, 65, "a 65 byte id")
		for _, bad in { long, "plain", "o1-zz", 12, "xo1-5-ab", "o1-5-ab!", "o1-5-ab-c", "o1-5-", "O1-5-ab", "o1-5-AB" } do
			local ok, err = R.try(backend, realm, function()
				return store:update("ids", noop, bad)
			end)
			spec.eq(ok, false, `{tostring(bad)} throws`)
			spec.has(err, "FD.CommandLog: an operation id comes from operationId()", `{tostring(bad)} names the store`)
		end
		spec.eq(#backend.calls, 0, "nothing was sent")
		local old = R.update(backend, realm, store, "ids", noop, "o1-5-ab")
		spec.eq(old.kind, "expired", "the smallest well-formed id is read as an old one")
		local result = R.update(backend, realm, store, "ids", noop, exact)
		spec.eq(result.kind, "ok", "a 64 byte id is accepted")
		spec.list(R.stored(backend, "ids").operations, { exact }, "and recorded exactly")
	end,
	["rule 88 a full ring whose oldest entry is not an operation id evicts it and leaves the floor"] = function()
		local backend, realm, store = R.world()
		local ids = R.ring(256, WALL - 3000, "m")
		ids[1] = "legacy-entry"
		backend:seed(R.STORE, "legacy", R.envelope({ operations = ids, operationFloor = WALL - 4000 }), {}, {})
		local id = R.id(WALL - 5, "m9")
		local result = R.update(backend, realm, store, "legacy", noop, id)
		spec.eq(result.kind, "ok", "the update goes through")
		local expected = {}
		for index = 2, 256 do
			table.insert(expected, ids[index])
		end
		table.insert(expected, id)
		local stored = R.stored(backend, "legacy")
		spec.list(stored.operations, expected, "the entry left and the new id joined")
		spec.eq(stored.operationFloor, WALL - 4000, "the floor did not move")
	end,
	["rule 87 an update keeps the key's user ids and metadata"] = function()
		local backend, realm, store = R.world()
		backend:seed(R.STORE, "keep", R.envelope({}), { 11, 12 }, { tag = "x" })
		local result = R.update(backend, realm, store, "keep", noop, R.id(WALL - 5, "u1"))
		spec.eq(result.kind, "ok", "the update goes through")
		local entry = backend:stored(R.STORE, "keep")
		spec.list(entry.userIds, { 11, 12 }, "the user ids")
		spec.eq(entry.metadata.tag, "x", "the metadata")
	end,
	["rule 90 an attempt after an ambiguous commit that fails for good is unavailable, not the first attempt's result"] = function()
		local backend, realm, store = R.world()
		backend:fail("committed", { kind = "update", store = R.STORE }, 1)
		backend:fail("reject", { kind = "update", store = R.STORE }, 1, { after = 1, code = 403 })
		local result = R.update(backend, realm, store, "stale", noop, R.id(WALL - 5, "r1"), 5)
		spec.eq(result.kind, "unavailable", "unavailable")
		spec.eq(R.updates(backend), 2, "two attempts")
	end,
	["rule 88 a ring head exactly 60 seconds old at the transform is too young"] = function()
		local backend, realm, store = R.world()
		local ids = R.ring(256, WALL, "x")
		local seeded = R.envelope({ schemaVersion = 0, operations = ids, operationFloor = WALL - 500 })
		backend:seed(R.STORE, "edge", seeded, {}, {})
		backend:advance(59.95)
		local fresh = R.id(WALL + 58, "edge")
		local refused = R.update(backend, realm, store, "edge", noop, fresh, 0.1)
		spec.eq(refused.kind, "overloaded", "exactly 60 seconds is refused")
		R.same(R.stored(backend, "edge"), seeded, "and nothing was written")
		local accepted = R.update(backend, realm, store, "edge", noop, fresh, 0.1)
		spec.eq(accepted.kind, "ok", "a moment later it is accepted")
	end,
	["rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older"] = function()
		local backend, realm, store = R.world()
		local ids = R.ring(256, WALL, "q")
		local seeded = R.envelope({ schemaVersion = 0, operations = ids, operationFloor = WALL - 500 })
		backend:seed(R.STORE, "young", seeded, {}, {})
		local replay = R.update(backend, realm, store, "young", noop, ids[2], 0.2)
		spec.eq(replay.replayed, true, "a replay needs no eviction")
		R.same(R.stored(backend, "young"), seeded, "and writes nothing")

		backend:advance(59.65)
		local fresh = R.id(WALL + 58, "new")
		local refused = R.update(backend, realm, store, "young", noop, fresh, 0.2)
		spec.eq(refused.kind, "overloaded", "refused when the oldest is 59.9 seconds old")
		R.same(R.stored(backend, "young"), seeded, "and nothing was written")
		spec.eq(R.committed(backend), 0, "no commit")

		local accepted = R.update(backend, realm, store, "young", noop, fresh, 0.2)
		spec.eq(accepted.kind, "ok", "accepted when the oldest is 60.1 seconds old")
		local expected = {}
		for index = 2, 256 do
			table.insert(expected, ids[index])
		end
		table.insert(expected, fresh)
		R.same(
			R.stored(backend, "young"),
			R.envelope({ schemaVersion = 0, operations = expected, operationFloor = WALL }),
			"the oldest was evicted and the floor is its creation second"
		)
	end,
}
