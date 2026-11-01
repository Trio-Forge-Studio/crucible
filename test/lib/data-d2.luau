-- Fixtures for the receipt and inbox specs: a two-realm world over the shared backend, grant and message routes, and
-- operation ids built by an encoder of the spec's own (the ids' format is the contract's, not the engine's).
local Backend = require("./data-backend")

local D2 = {}

D2.STORE = "FD.Players"
D2.USER = 4120
D2.KEY = tostring(D2.USER)
D2.WALL = 1700000000
D2.PRODUCT = 10

local DIGITS = "0123456789abcdefghijklmnopqrstuvwxyz"

local function base36(value: number): string
	local text = ""
	repeat
		local digit = value % 36
		text = string.sub(DIGITS, digit + 1, digit + 1) .. text
		value = (value - digit) // 36
	until value == 0
	return text
end

-- An id created `seconds` of wall time after the backend's epoch zero.
function D2.id(created: number, suffix: string): string
	return `o1-{base36(created)}-{suffix}`
end

-- The wall second `offset` seconds from the backend's current time.
function D2.wallAt(backend, offset: number?): number
	return D2.WALL + math.floor(backend.now) + (offset or 0)
end

function D2.world(count: number?, options)
	options = options or {}
	local backend = Backend.new()
	local realms = {}
	for index = 1, count or 1 do
		local realm = backend:realm({ env = options.env, subsystems = options.subsystems })
		realm.store = realm.profile(
			D2.STORE,
			options.defaults or { coins = 0, gems = 0 },
			if options.versions then options.versions[index] else options.version
		)
		realm.receipts = realm.data.receipts
		table.insert(realms, realm)
	end
	return backend, realms[1], realms[2], realms[3]
end

function D2.join(backend, realm, userId: number?)
	local player = realm.join(userId or D2.USER)
	local handle = realm.run(function()
		return realm.store:load(player, "join")
	end)
	backend:advance(0.5)
	assert(handle.value == true, "loaded")
	return player
end

-- Seeds a stored envelope for a user. `fields` overrides any field; `unopened` leaves out the payload.
function D2.seed(backend, userId: number?, fields)
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
	if envelope.unopened then
		envelope.unopened = nil
		envelope.payload = nil
	end
	backend:seed(D2.STORE, tostring(userId or D2.USER), envelope, { userId or D2.USER }, {})
end

-- Joins and loads, stepping the clock until the load finishes: for a key whose lease must lapse first.
function D2.joinSlow(backend, realm, userId: number?)
	local player = realm.join(userId or D2.USER)
	local handle = realm.run(function()
		return realm.store:load(player, "join")
	end)
	local waited = 0
	while not handle.done and waited < 300 do
		backend:advance(0.05)
		waited += 0.05
	end
	assert(handle.value == true, "loaded after the lease lapsed")
	return player
end

-- Realm 1 boots DataRuntime's shutdown hook and one game hook, `game(realm, self)`, in parallel.
function D2.sweepWorld(game, count: number?, options)
	local holder = {}
	options = options or {}
	options.subsystems = {
		{
			id = "DataRuntime",
			methods = {
				onShutdown = function(self)
					holder.realm.shutdown.shutdownData(holder.realm.data, self)
				end,
			},
		},
		{
			id = "Game",
			methods = {
				onShutdown = function(self)
					if game then
						game(holder.realm, self)
					end
				end,
			},
		},
	}
	local backend, a, b = D2.world(count, options)
	holder.realm = a
	return backend, a, b
end

function D2.sleep(realm, seconds: number)
	local wake = realm.clock() + seconds
	while realm.clock() < wake do
		realm.wait(0)
	end
end

function D2.untilDone(backend, handle, limit: number?)
	local waited = 0
	while not handle.done and waited < (limit or 300) do
		backend:advance(0.05)
		waited += 0.05
	end
end

-- A message declaration whose payload is a table with a numeric `amount`; anything else is refused.
function D2.message(name: string)
	return {
		name = name,
		admit = function(payload)
			if type(payload) == "table" and type(payload.amount) == "number" then
				return { ok = true, value = payload }
			end
			return { ok = false }
		end,
	}
end

-- A declaration that admits any table, as a newer build that accepts more would.
function D2.lenient(name: string)
	return {
		name = name,
		admit = function(payload)
			if type(payload) == "table" then
				return { ok = true, value = payload }
			end
			return { ok = false }
		end,
	}
end

-- Registers a receiver that adds `payload.amount` to coins and records every call.
-- options.throws(payload, count) makes it throw; options.unchanged returns the data untouched.
function D2.receiver(realm, decl, options)
	local record = { calls = {} }
	realm.store.inbox:receive(decl, function(_, data, payload)
		table.insert(record.calls, { amount = payload.amount, at = realm.backend.now })
		if options and options.throws and options.throws(payload, #record.calls) then
			error("receiver failed")
		end
		if options and options.unchanged then
			return data
		end
		local value = table.clone(data)
		value.coins += payload.amount
		return value
	end)
	return record
end

function D2.send(realm, decl, payload, id, userId: number?)
	return realm.run(function()
		return realm.store.inbox:send(userId or D2.USER, decl, payload, id)
	end)
end

-- A stored inbox item.
function D2.item(id: string, name: string, payload, fields)
	local item = { id = id, name = name, version = 0, payload = payload, generation = 0, attempts = 0 }
	for key, value in fields or {} do
		item[key] = value
	end
	return item
end

function D2.session(realm, player)
	return realm.store.sessions[player]
end

function D2.stored(backend, userId: number?)
	return backend:envelope(D2.STORE, tostring(userId or D2.USER))
end

function D2.set(realm, player, change)
	local result
	realm.run(function()
		result = realm.store:write(player, change)
	end)
	return result
end

-- A grant that adds `gems` and records every call.
function D2.route(realm, productId: number, gems: number, hooks)
	local route = { calls = {} }
	realm.receipts:route(productId, {
		runtime = realm.store,
		grant = function(player, data, purchase, afterGrant)
			table.insert(route.calls, purchase.id)
			if hooks and hooks.during then
				hooks.during(player, data, purchase, afterGrant)
			end
			local value = table.clone(data)
			value.gems += gems
			return value
		end,
	})
	return route
end

function D2.info(id: string, productId: number?, userId: number?, spent: number?)
	return {
		PlayerId = userId or D2.USER,
		PurchaseId = id,
		ProductId = productId or D2.PRODUCT,
		CurrencySpent = spent or 99,
	}
end

-- Runs ProcessReceipt on a realm thread; `.value` is "granted" or "notYet" once it returns.
function D2.purchase(realm, info)
	return realm.run(function()
		return realm.receipts:process(info)
	end)
end

return D2
