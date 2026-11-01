-- Fixtures for the generic store specs (data rules 85 to 91): a world with one record store, hand-written records
-- envelopes, and an encoder and decoder for operation ids and sizes that are the spec's own, not the engine's.
local serde = require("@lune/serde")
local Backend = require("./data-backend")
local D2 = require("./data-d2")

local R = {}

R.STORE = "FD.CommandLog"
R.KEY = "log"
R.WALL = D2.WALL
R.VERSION = 2
R.SECRET = "S3CRET-token-9f2a"
R.id = D2.id

local DIGITS = "0123456789abcdefghijklmnopqrstuvwxyz"

function R.decode36(text: string): number
	local value = 0
	for index = 1, #text do
		value = value * 36 + (string.find(DIGITS, string.sub(text, index, index), 1, true) :: number) - 1
	end
	return value
end

-- Two migration steps with distinct arithmetic, so a skipped or repeated step changes the result: from version 0
-- a record with total 2 becomes total 3 and label "m3"; from version 1 it keeps total 2 and gets label "m2".
R.MIGRATIONS = {
	function(old)
		return { total = old.total + 1 }
	end,
	function(old)
		return { label = "m" .. tostring(old.total) }
	end,
}

function R.node(n)
	return n.object({
		{ "entries", n.array(n.object({ { "id", n.string } })) },
		{ "label", n.union({ n["nil"], n.string }) },
		{ "tallies", n.map(n.string, n.object({ { "hits", n.f64 } })) },
		{ "total", n.f64 },
	})
end

function R.bulkNode(n)
	return n.object({ { "chunks", n.array(n.string) } })
end

function R.world(options)
	options = options or {}
	local backend = Backend.new()
	local realm = backend:realm()
	local store =
		realm.recordStore(options.name or R.STORE, options.defaults or { entries = {}, tallies = {}, total = 0 }, {
			node = options.node or R.node(realm.n),
			migrations = options.migrations or R.MIGRATIONS,
		})
	return backend, realm, store
end

function R.bulkWorld()
	local backend = Backend.new()
	local realm = backend:realm()
	local store = realm.recordStore("FD.Bulk", { chunks = {} }, { node = R.bulkNode(realm.n), migrations = {} })
	return backend, realm, store
end

-- A second realm over the same backend and the same store.
function R.second(backend, options)
	options = options or {}
	local realm = backend:realm()
	local store = realm.recordStore(options.name or R.STORE, { entries = {}, tallies = {}, total = 0 }, {
		node = R.node(realm.n),
		migrations = R.MIGRATIONS,
	})
	return realm, store
end

-- A records envelope as rule 38 lays it out; `fields` overrides any field and `unopened` leaves out the payload.
function R.envelope(fields)
	local envelope = {
		format = 1,
		kind = "records",
		schemaVersion = R.VERSION,
		payload = nil,
		epoch = 0,
		receipts = {},
		inbox = {},
		quarantine = {},
		seen = {},
		seenFloor = 0,
		operations = {},
		operationFloor = 0,
	}
	for key, value in fields or {} do
		envelope[key] = value
	end
	return envelope
end

function R.seed(backend, key: string, fields, store: string?)
	backend:seed(store or R.STORE, key, R.envelope(fields), {}, {})
end

function R.stored(backend, key: string?, store: string?)
	return backend:envelope(store or R.STORE, key or R.KEY)
end

function R.updates(backend, store: string?, key: string?)
	local filter = { kind = "update", store = store or R.STORE }
	if key then
		filter.key = key
	end
	return #backend:callsTo(filter)
end

function R.committed(backend, store: string?, key: string?)
	local count = 0
	local filter = { kind = "update", store = store or R.STORE }
	if key then
		filter.key = key
	end
	for _, call in backend:callsTo(filter) do
		if call.committed then
			count += 1
		end
	end
	return count
end

-- Runs `fn` on a realm thread, advances the timeline, and returns what it returned.
function R.call(backend, realm, fn, seconds: number?)
	local handle = realm.run(fn)
	backend:advance(seconds or 1)
	assert(handle.done, "the call returned")
	return handle.value
end

-- Like `call` for a function that may throw: returns `ok, value-or-error`.
function R.try(backend, realm, fn, seconds: number?)
	local out = R.call(backend, realm, function()
		local ok, value = pcall(fn)
		return { ok = ok, value = value }
	end, seconds)
	return out.ok, out.value
end

function R.update(backend, realm, store, key: string, change, id: string?, seconds: number?)
	return R.call(backend, realm, function()
		return store:update(key, change, id)
	end, seconds)
end

function R.read(backend, realm, store, key: string, seconds: number?)
	return R.call(backend, realm, function()
		return store:read(key)
	end, seconds)
end

function R.logText(realm): string
	local lines = {}
	for _, entry in realm.engine.logs do
		table.insert(lines, entry.message)
	end
	return table.concat(lines, "\n")
end

local function same(actual, expected, label: string, path: string)
	if type(actual) ~= type(expected) then
		error(`{label}: {path} expected {type(expected)}, got {type(actual)}`, 0)
	end
	if type(expected) ~= "table" then
		if actual ~= expected then
			error(`{label}: {path} expected {tostring(expected)}, got {tostring(actual)}`, 0)
		end
		return
	end
	for key, value in expected do
		same(actual[key], value, label, `{path}.{tostring(key)}`)
	end
	for key in actual do
		if expected[key] == nil then
			error(`{label}: {path}.{tostring(key)} is present and should not be`, 0)
		end
	end
end

-- Deep equality over plain tables: every key on both sides, so an extra stored field fails too.
function R.same(actual, expected, label: string)
	same(actual, expected, label, "$")
end

local function frozen(value, label: string, path: string)
	if type(value) ~= "table" then
		return
	end
	if not table.isfrozen(value) then
		error(`{label}: {path} is not frozen`, 0)
	end
	for key, entry in value do
		frozen(entry, label, `{path}.{tostring(key)}`)
	end
end

function R.frozen(value, label: string)
	frozen(value, label, "$")
end

-- The ids of a ring: `count` ids created one second apart from `first`, each padded to `width` bytes.
function R.ring(count: number, first: number, tag: string, width: number?): { string }
	local ids = {}
	for index = 1, count do
		local created = first + index - 1
		local prefix = R.id(created, "")
		local number = `{tag}{index}`
		local pad = (width or #prefix + #number) - #prefix - #number
		table.insert(ids, prefix .. number .. string.rep("a", math.max(pad, 0)))
	end
	return ids
end

-- Strings that make `{ chunks = [...] }` encode to exactly `bytes` of JSON, each at most the 65,536-byte string
-- limit. Encoded size is 12 + the sum of lengths + 3 per chunk: `{"chunks":[` and `]}` plus quotes and commas.
function R.chunks(bytes: number, secret: string?): { string }
	local count = 1
	while count * 65536 < bytes - 12 - 3 * count do
		count += 1
	end
	local remaining = bytes - 12 - 3 * count
	local chunks = {}
	for index = 1, count do
		local length = math.min(remaining, 65536)
		remaining -= length
		chunks[index] = string.rep("a", length)
	end
	if secret then
		chunks[1] = secret .. string.sub(chunks[1], #secret + 1)
	end
	assert(#serde.encode("json", { chunks = chunks }) == bytes, "the chunk model matches an encoder")
	return chunks
end

function R.encodedBytes(value): number
	return #serde.encode("json", value)
end

return R
