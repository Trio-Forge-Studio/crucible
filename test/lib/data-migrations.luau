-- Fixtures for the migration, admission and unknown-key specs: persist descriptors built outside any realm (admission
-- reads a node's shape and never its provenance), loads and writes on realm threads, and expected envelopes written by
-- hand from the contract's layout (data rule 38), never computed by the engine under test.
local spec = require("./load")
local D2 = require("./data-d2")

local DM = {}

DM.STORE = D2.STORE
DM.USER = D2.USER
DM.KEY = D2.KEY
DM.n = spec.fresh("internal/network/schema").n

local n = DM.n

DM.opt = function(node)
	return n.union({ n["nil"], node })
end

-- An object descriptor from a dictionary of field nodes, in the sorted order the builder asserts.
function DM.object(fields)
	local names = {}
	for name in fields do
		table.insert(names, name)
	end
	table.sort(names)
	local ordered = {}
	for _, name in names do
		table.insert(ordered, { name, fields[name] })
	end
	return n.object(ordered)
end

-- Structural equality of two plain values, reporting the first path that differs.
function DM.deepEq(actual, expected, label: string, path: string?)
	path = path or ""
	if type(actual) ~= type(expected) then
		error(
			`{label}: at {path ~= "" and path or "<root>"} expected {typeof(expected)} {tostring(expected)}, got {typeof(
				actual
			)} {tostring(actual)}`,
			2
		)
	end
	if type(expected) ~= "table" then
		if actual ~= expected and not (actual ~= actual and expected ~= expected) then
			error(
				`{label}: at {path ~= "" and path or "<root>"} expected {tostring(expected)}, got {tostring(actual)}`,
				2
			)
		end
		return
	end
	for key, value in expected do
		DM.deepEq(actual[key], value, label, `{path}.{tostring(key)}`)
	end
	for key in actual do
		if expected[key] == nil then
			error(`{label}: at {path}.{tostring(key)} unexpected key`, 2)
		end
	end
end

-- The envelope the contract's layout gives a profile key, with `fields` overriding.
function DM.envelope(fields)
	local envelope = {
		format = 1,
		kind = "profiles",
		schemaVersion = 0,
		payload = { coins = 0, gems = 0 },
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

function DM.attempt(realm, fn)
	local outcome = { done = false }
	realm.run(function()
		outcome.ok, outcome.err = pcall(fn)
		outcome.done = true
	end)
	return outcome
end

-- Joins `userId` and starts its load without stepping time.
function DM.begin(realm, userId: number?)
	local player = realm.join(userId or DM.USER)
	local handle = realm.run(function()
		return realm.store:load(player, "join")
	end)
	handle.startedAt = realm.backend.now
	return player, handle
end

-- Joins, loads and steps `seconds` (one by default).
function DM.load(backend, realm, userId: number?, seconds: number?)
	local player, handle = DM.begin(realm, userId)
	backend:advance(seconds or 1)
	return player, handle
end

function DM.session(realm, player)
	return realm.store.sessions[player]
end

function DM.stored(backend, userId: number?)
	return backend:envelope(DM.STORE, tostring(userId or DM.USER))
end

function DM.write(realm, player, change)
	return DM.attempt(realm, function()
		return realm.store:write(player, change)
	end)
end

function DM.save(realm, player, timeout: number?)
	return realm.run(function()
		return realm.store:save(player, timeout)
	end)
end

-- Leaves and releases, then steps one second so the release commits.
function DM.leave(backend, realm, player)
	realm.leave(player)
	realm.run(function()
		realm.store:leave(player)
	end)
	backend:advance(1)
end

function DM.inspect(backend, realm, userId: number?)
	local handle = realm.run(function()
		return realm.store:inspect(userId or DM.USER)
	end)
	backend:advance(1)
	return handle.value
end

-- The first kick of a player on a realm, with its time.
function DM.kick(realm, player)
	for _, kick in realm.kicks do
		if kick.player == player then
			return kick
		end
	end
	return nil
end

-- Every logged diagnostic containing `part`.
function DM.logged(realm, part: string): { string }
	local found = {}
	for _, entry in realm.engine.logs do
		if string.find(entry.message, part, 1, true) then
			table.insert(found, entry.message)
		end
	end
	return found
end

-- The load failed: refused at once with `message`, the stored envelope equals `seed` but for the acquisition's epoch
-- step, and the abandoned acquisition left no lease.
function DM.assertRefused(backend, realm, player, handle, message: string, seed, label: string, before: any?)
	spec.eq(handle.value, false, `{label}: the load fails`)
	local kick = DM.kick(realm, player)
	assert(kick ~= nil, `{label}: kicked`)
	spec.eq(kick.message, message, `{label}: kick message`)
	assert(
		kick.at - handle.startedAt < 0.5,
		`{label}: kicked at once, not at the load deadline (after {kick.at - handle.startedAt} s)`
	)
	local expected = if before then table.clone(before) else DM.envelope(seed)
	expected.epoch = (if before then before.epoch else seed.epoch) + 1
	DM.deepEq(DM.stored(backend), expected, `{label}: stored envelope`)
	spec.eq(realm.store:writable(player), false, `{label}: no writable session`)
end

-- A chain of `levels` nested tables under the key `deep`, the innermost empty: `levels` counts tables including the
-- table returned.
function DM.nest(levels: number, key: string?)
	local name = key or "deep"
	local root = {}
	local cursor = root
	for _ = 2, levels do
		local child = {}
		cursor[name] = child
		cursor = child
	end
	return root
end

return DM
