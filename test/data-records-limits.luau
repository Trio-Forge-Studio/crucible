local spec = require("./lib/load")
local D2 = require("./lib/data-d2")
local R = require("./lib/data-records")

local WALL = R.WALL
local MAX_ENVELOPE_BYTES = 1048576
local RECORD_SECTIONS_BYTES = 4096 + 1000000 + 18432

local function noop(current)
	return current
end

local function chunked(chunks)
	return function()
		return { chunks = chunks }
	end
end

local function bulkEnvelope(fields)
	local envelope = R.envelope(fields)
	envelope.schemaVersion = 0
	return envelope
end

-- The envelope the engine writes when `extra` is appended to a seeded bulk record: a spec-owned model of rule 38.
local function inboxEnvelope(base, extra, id, list)
	local chunks = table.clone(base)
	table.insert(chunks, extra)
	return bulkEnvelope({ payload = { chunks = chunks }, inbox = list, operations = { id } })
end

local function items(count: number, size: number)
	local list = {}
	for index = 1, count do
		table.insert(list, D2.item(R.id(WALL - 50, `i{index}`), "Msg", { text = string.rep("x", size) }))
	end
	return list
end

-- The length of the appended chunk that makes the final envelope `target` encoded bytes.
local function fitChunk(base, id, list, target: number): number
	local length = 20000
	for _ = 1, 3 do
		local size = R.encodedBytes(inboxEnvelope(base, string.rep("a", length), id, list))
		length += target - size
	end
	return length
end

return {
	["rule 87 a record between 262,144 and 1,000,000 bytes is accepted"] = function()
		local backend, realm, store = R.bulkWorld()
		local chunks = R.chunks(500000)
		local id = R.id(WALL - 5, "s1")
		local result = R.update(backend, realm, store, "mid", chunked(chunks), id)
		spec.eq(result.kind, "ok", "500,000 bytes is accepted")
		R.same(
			R.stored(backend, "mid", "FD.Bulk"),
			bulkEnvelope({ payload = { chunks = chunks }, operations = { id } }),
			"stored"
		)
		spec.eq(R.committed(backend, "FD.Bulk"), 1, "one write")
	end,
	["rule 87 a record of exactly 1,000,000 bytes with a full ring of 64 byte ids is written within the record sections sum"] = function()
		local backend, realm, store = R.bulkWorld()
		local old = R.ring(256, WALL - 3000, "o", 64)
		backend:seed("FD.Bulk", "cap", bulkEnvelope({ operations = old }), {}, {})
		local chunks = R.chunks(1000000)
		local id = R.ring(1, WALL - 10, "n", 64)[1]
		spec.eq(#id, 64, "a 64 byte id")
		local result = R.update(backend, realm, store, "cap", chunked(chunks), id)
		spec.eq(result.kind, "ok", "accepted at the cap")
		local expected = {}
		for index = 2, 256 do
			table.insert(expected, old[index])
		end
		table.insert(expected, id)
		local stored = R.stored(backend, "cap", "FD.Bulk")
		R.same(stored.operations, expected, "the ring holds 256 ids with the oldest evicted")
		spec.eq(#stored.operations, 256, "256 markers")
		R.same(stored.payload, { chunks = chunks }, "the payload")
		local bytes = R.encodedBytes(stored)
		spec.eq(bytes <= RECORD_SECTIONS_BYTES, true, `{bytes} bytes is within the record sections sum`)
		spec.eq(bytes <= MAX_ENVELOPE_BYTES, true, "and the envelope cap")
		spec.eq(
			realm.limits.RECORD_SECTIONS_BYTES,
			RECORD_SECTIONS_BYTES,
			"the engine's sum is the table's sum, 1,022,528"
		)
		spec.eq(RECORD_SECTIONS_BYTES, 1022528, "which is 4,096 + 1,000,000 + 18,432")
		spec.eq(RECORD_SECTIONS_BYTES < MAX_ENVELOPE_BYTES, true, "below the envelope cap")
	end,
	["rule 87 a record over 1,000,000 bytes throws at the call naming the store and key, stores nothing and records no id"] = function()
		local backend, realm, store = R.bulkWorld()
		local chunks = R.chunks(1000001, R.SECRET)
		local id = R.id(WALL - 5, "s2")
		local ok, err = R.try(backend, realm, function()
			return store:update("big", chunked(chunks), id)
		end)
		spec.eq(ok, false, "the call throws")
		spec.has(err, "FD.Bulk: big", "naming the store and key")
		spec.has(err, "1000001", "and the size")
		spec.eq(string.find(err, R.SECRET, 1, true), nil, "never a value")
		spec.eq(R.stored(backend, "big", "FD.Bulk"), nil, "nothing was stored")
		spec.eq(R.committed(backend, "FD.Bulk"), 0, "nothing was written")
		local runs = 0
		local retry = R.update(backend, realm, store, "big", function(current)
			runs += 1
			return current
		end, id)
		spec.eq(retry.replayed, false, "the id was not recorded")
		spec.eq(runs, 1, "so change ran")
	end,
	["rule 39 a string over 65,536 bytes is refused by admission at the call"] = function()
		local backend, realm, store = R.bulkWorld()
		local ok, err = R.try(backend, realm, function()
			return store:update("long", chunked({ string.rep("a", 65537) }), R.id(WALL - 5, "s3"))
		end)
		spec.eq(ok, false, "the call throws")
		spec.has(err, "FD.Bulk.chunks[0]", "naming the store and path")
		spec.eq(R.stored(backend, "long", "FD.Bulk"), nil, "nothing was stored")
	end,
	["rule 39 other sections push the final envelope over the cap: the update cancels unavailable, and the cap itself is exact"] = function()
		local base = R.chunks(500000)
		local list = items(128, 3900)
		local id = R.id(WALL - 5, "z1")
		local length = fitChunk(base, id, list, MAX_ENVELOPE_BYTES)
		local function attempt(extra: number, key: string)
			local backend, realm, store = R.bulkWorld()
			local seeded = bulkEnvelope({ payload = { chunks = base }, inbox = list })
			backend:seed("FD.Bulk", key, seeded, {}, {})
			local result = R.update(backend, realm, store, key, function(current)
				local chunks = table.clone(current.chunks)
				table.insert(chunks, string.rep("a", extra))
				return { chunks = chunks }
			end, id)
			return result, backend, realm, seeded
		end
		local exact, backendExact = attempt(length, "exact")
		spec.eq(
			R.encodedBytes(R.stored(backendExact, "exact", "FD.Bulk")),
			MAX_ENVELOPE_BYTES,
			"the model hits the cap exactly"
		)
		spec.eq(exact.kind, "ok", "an envelope of exactly 1,048,576 bytes is accepted")
		local over, backendOver, realm, seeded = attempt(length + 1, "over")
		spec.eq(over.kind, "unavailable", "one byte over is unavailable")
		R.same(R.stored(backendOver, "over", "FD.Bulk"), seeded, "the record is untouched")
		spec.eq(R.committed(backendOver, "FD.Bulk"), 0, "no commit")
		spec.has(R.logText(realm), "FD.Bulk: over", "diagnosed with the store and key")
		spec.has(R.logText(realm), tostring(MAX_ENVELOPE_BYTES + 1), "and the size")
	end,
	["rule 110 an envelope over the cap is diagnosed once per store without a value"] = function()
		local backend, realm, store = R.bulkWorld()
		local base = R.chunks(500000, R.SECRET)
		local list = items(128, 3900)
		local seeded = bulkEnvelope({ payload = { chunks = base }, inbox = list })
		backend:seed("FD.Bulk", "d1", seeded, {}, {})
		backend:seed("FD.Bulk", "d2", seeded, {}, {})
		local grow = function(current)
			local chunks = table.clone(current.chunks)
			table.insert(chunks, string.rep("a", 60000))
			return { chunks = chunks }
		end
		local first = R.update(backend, realm, store, "d1", grow, R.id(WALL - 5, "d1"))
		local second = R.update(backend, realm, store, "d2", grow, R.id(WALL - 5, "d2"))
		spec.eq(first.kind, "unavailable", "the first is unavailable")
		spec.eq(second.kind, "unavailable", "and so is the second")
		spec.eq(realm.engine.logged("would make the envelope"), 1, "diagnosed once per store")
		spec.has(R.logText(realm), "FD.Bulk: d1", "naming the first key")
		spec.eq(string.find(R.logText(realm), R.SECRET, 1, true), nil, "never a value")
	end,

	["rule 90 a request that was never dispatched settles timeout at the 30 second deadline"] = function()
		local backend, realm, store = R.world()
		realm.budget.update = 0
		realm.budget.read = 0
		local update = realm.run(function()
			return store:update("slow", noop, R.id(WALL - 5, "t1"))
		end)
		local read = realm.run(function()
			return store:read("slow")
		end)
		backend:advance(29)
		spec.eq(update.done or read.done, false, "still waiting before the deadline")
		backend:advance(2)
		spec.eq(update.value.kind, "timeout", "an update times out")
		spec.eq(read.value.kind, "timeout", "a read times out")
		spec.eq(#backend.calls, 0, "nothing was sent")
	end,
	["rule 90 a dispatched request with no answer by the deadline settles uncertain"] = function()
		local backend, realm, store = R.world()
		backend:gate("response", { kind = "update" })
		backend:gate("response", { kind = "read" })
		local update = realm.run(function()
			return store:update("quiet", noop, R.id(WALL - 5, "u1"))
		end)
		local read = realm.run(function()
			return store:read("quiet2")
		end)
		backend:advance(29)
		spec.eq(update.done or read.done, false, "still waiting before the deadline")
		backend:advance(2)
		spec.eq(update.value.kind, "uncertain", "an update is uncertain")
		spec.eq(read.value.kind, "uncertain", "a read is uncertain")
	end,
	["rule 90 the 33rd caller on one key is overloaded and the rest settle timeout"] = function()
		local backend, realm, store = R.world()
		realm.budget.update = 0
		local waiting = {}
		for index = 1, 32 do
			waiting[index] = realm.run(function()
				return store:update("busy", noop, R.id(WALL - 5, `b{index}`))
			end)
		end
		spec.eq(waiting[1].done or waiting[32].done, false, "32 callers queue")
		local over = realm.run(function()
			return store:update("busy", noop, R.id(WALL - 5, "b33"))
		end)
		spec.eq(over.done, true, "the 33rd is refused at once")
		spec.eq(over.value.kind, "overloaded", "overloaded")
		local read = realm.run(function()
			return store:read("busy")
		end)
		spec.eq(read.value.kind, "overloaded", "a read counts against the same cap")
		local other = realm.run(function()
			return store:update("quiet", noop, R.id(WALL - 5, "b34"))
		end)
		spec.eq(other.done, false, "another key still queues")
		backend:advance(31)
		for index = 1, 32 do
			spec.eq(waiting[index].value.kind, "timeout", `queued caller {index}`)
		end
		spec.eq(#backend.calls, 0, "nothing was sent")
	end,
	["rule 90 after the shutdown dispatch cutoff a call is closing and sends nothing"] = function()
		local backend, realm, store = R.world()
		realm.run(function()
			realm.shutdown.shutdownData(realm.data)
		end)
		backend:advance(0.1)
		local open = R.update(backend, realm, store, "open", noop, R.id(WALL - 5, "k1"), 0.2)
		spec.eq(open.kind, "ok", "accepted before the cutoff")
		backend:advance(0.3)
		local calls = #backend.calls
		local update = R.update(backend, realm, store, "late", noop, R.id(WALL - 5, "k2"))
		local read = R.read(backend, realm, store, "late")
		spec.eq(update.kind, "closing", "an update is closing")
		spec.eq(read.kind, "closing", "a read is closing")
		spec.eq(#backend.calls, calls, "nothing was sent")
	end,
	["rule 90 a non-retryable storage error is unavailable and diagnosed once per store and code"] = function()
		local backend, realm, store = R.world()
		backend:fail("reject", { kind = "update" }, 1, { code = 403 })
		backend:fail("reject", { kind = "read" }, 1, { code = 403 })
		local update = R.update(backend, realm, store, "rej", noop, R.id(WALL - 5, "e1"))
		local read = R.read(backend, realm, store, "rej")
		spec.eq(update.kind, "unavailable", "an update is unavailable")
		spec.eq(read.kind, "unavailable", "a read is unavailable")
		spec.eq(R.updates(backend), 1, "never retried")
		spec.eq(realm.engine.logged("storage error 403"), 1, "diagnosed once")
		spec.eq(R.stored(backend, "rej"), nil, "nothing was stored")
	end,

	["rule 52 a CommandLog flush through the real store dispatches in the open phase and a call after the cutoff is closing"] = function()
		local log
		local early, late
		local backend, realm = D2.sweepWorld(function(r)
			D2.sleep(r, 1)
			early = log:update("log", function(current)
				return { entries = { { id = "flush" } }, tallies = current.tallies, total = current.total + 1 }
			end, R.id(WALL - 5, "f1"))
			D2.sleep(r, 26)
			late = log:update("late", noop, R.id(WALL - 5, "f2"))
		end)
		log = realm.recordStore(R.STORE, { entries = {}, tallies = {}, total = 0 }, {
			node = R.node(realm.n),
			migrations = R.MIGRATIONS,
		})
		realm.close()
		backend:advance(30)
		spec.eq(early.kind, "ok", "accepted in the open phase")
		R.same(
			R.stored(backend, "log"),
			R.envelope({
				payload = { entries = { { id = "flush" } }, tallies = {}, total = 1 },
				operations = { R.id(WALL - 5, "f1") },
			}),
			"committed"
		)
		spec.eq(late.kind, "closing", "refused after the cutoff")
		spec.eq(#backend:callsTo({ key = "late" }), 0, "and sent nothing")
	end,

	["rule 110 unreadable and newer records are diagnosed once per store and code with the key and no value"] = function()
		local backend, realm, store = R.world()
		local good = { entries = {}, tallies = {}, total = 1 }
		R.seed(backend, "u1", { payload = { entries = {}, tallies = {}, total = R.SECRET } })
		R.seed(backend, "u2", { format = 2, payload = good })
		R.seed(backend, "u3", { schemaVersion = 3, payload = { entries = {}, extra = R.SECRET } })
		R.seed(backend, "u4", { schemaVersion = 3, payload = good })
		local reads = R.read(backend, realm, store, "u1")
		local updates = R.update(backend, realm, store, "u2", noop, R.id(WALL - 5, "x1"))
		R.read(backend, realm, store, "u1")
		spec.eq(reads.kind, "unavailable", "a read of an unreadable record")
		spec.eq(updates.kind, "unavailable", "an update of an unreadable record")
		spec.eq(realm.engine.logged("FD.CommandLog: u1 "), 1, "unreadable is diagnosed once, naming the first key")
		spec.eq(realm.engine.logged("FD.CommandLog: u2 "), 0, "and not again for the second")
		R.read(backend, realm, store, "u3")
		R.update(backend, realm, store, "u4", noop, R.id(WALL - 5, "x2"))
		spec.eq(realm.engine.logged("FD.CommandLog: u3 was written by a newer build"), 1, "newer is its own code")
		spec.eq(realm.engine.logged("FD.CommandLog: u4 "), 0, "also once")
		spec.eq(#realm.engine.logs, 2, "two diagnostics in all")
		spec.eq(string.find(R.logText(realm), R.SECRET, 1, true), nil, "never a value")
	end,
}
