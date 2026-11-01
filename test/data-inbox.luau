local serde = require("@lune/serde")
local spec = require("./lib/load")
local D2 = require("./lib/data-d2")

local W = D2.WALL

local function attempt(realm, fn)
	local outcome = {}
	realm.run(function()
		outcome.ok, outcome.err = pcall(fn)
	end)
	return outcome
end

local function inbox(backend)
	return D2.stored(backend).inbox
end

return {
	["Limits receipt and inbox rows hold the contract values"] = function()
		local _, a = D2.world()
		local limits = a.limits
		local expected = {
			RECEIPT_SAVE_TIMEOUT_SECONDS = 30,
			MAX_PURCHASE_IDS = 8192,
			RECEIPTS_MAX_BYTES = 393216,
			PURCHASE_ID_MAX_BYTES = 64,
			INBOX_MAX_MESSAGES = 128,
			INBOX_MAX_BYTES = 131072,
			QUARANTINE_MAX_MESSAGES = 8,
			QUARANTINE_MAX_BYTES = 69632,
			MESSAGE_PAYLOAD_MAX_BYTES = 8192,
			MESSAGE_ID_MAX_AGE_SECONDS = 604800,
			ID_FUTURE_SKEW_SECONDS = 5,
			ID_EVICT_MIN_AGE_SECONDS = 60,
			OPERATION_ID_MAX_BYTES = 64,
			SEEN_MAX_MESSAGES = 1024,
			SEEN_MAX_BYTES = 69632,
			INBOX_MAX_ATTEMPTS = 5,
		}
		for name, value in expected do
			spec.eq(limits[name], value, name)
		end
		spec.list(limits.INBOX_RETRY_SECONDS, { 5, 15, 60, 60 }, "INBOX_RETRY_SECONDS")
	end,
	["rule 66 an invalid or oversize payload and a bad id throw at the call with nothing dispatched"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local function send(payload, id)
			return attempt(a, function()
				return a.store.inbox:send(D2.USER, decl, payload, id)
			end)
		end
		local invalid = send("not a table")
		spec.eq(invalid.ok, false, "an invalid payload throws")
		spec.has(invalid.err, "invalid", "naming the problem")
		spec.eq(string.find(invalid.err, "not a table", 1, true), nil, "never the value")
		local atLimit = { amount = 1, note = string.rep("x", 8170) }
		local overLimit = { amount = 1, note = string.rep("x", 8171) }
		spec.eq(#serde.encode("json", atLimit), 8192, "the fixture is exactly 8,192 bytes")
		spec.eq(#serde.encode("json", overLimit), 8193, "and the other is one over")
		local over = send(overLimit)
		spec.eq(over.ok, false, "an oversize payload throws")
		spec.has(over.err, "8193 bytes, over 8192", "naming the sizes")
		for label, id in
			{
				["an id operationId() did not make"] = "abc",
				["an id over 64 bytes"] = D2.id(W, string.rep("a", 70)),
				["a non-string id"] = 5,
			}
		do
			local outcome = send({ amount = 1 }, id)
			spec.eq(outcome.ok, false, `{label} throws`)
			spec.has(outcome.err, "operationId()", `{label}: naming where ids come from`)
		end
		spec.eq(#backend.calls, 0, "nothing was dispatched")
		local fits = send(atLimit)
		backend:advance(1)
		spec.eq(fits.ok, true, "an 8,192-byte payload is accepted")
		spec.eq(#backend.calls, 1, "and dispatched")
	end,
	["rule 66 a send to an absent key creates an unopened envelope holding only the item"] = function()
		local backend, a = D2.world(1, { version = 3 })
		local decl = D2.message("grant")
		local id = D2.id(W, "m1")
		local gate = backend:gate("commit", { server = a.id })
		local handle = D2.send(a, decl, { amount = 5 }, id)
		backend:advance(1)
		spec.eq(handle.done, false, "the call yields until the append commits")
		gate.release()
		backend:advance(0.5)
		spec.eq(handle.value.kind, "ok", "ok once the envelope shows the id")
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload, nil, "no payload: the envelope is unopened")
		spec.eq(envelope.epoch, 0, "epoch 0")
		spec.eq(envelope.lease, nil, "no lease")
		spec.eq(envelope.commit, nil, "no commit marker")
		spec.eq(#envelope.receipts, 0, "no receipts")
		spec.eq(#envelope.inbox, 1, "one item")
		local item = envelope.inbox[1]
		spec.eq(item.id, id, "its id")
		spec.eq(item.name, "grant", "its name")
		spec.eq(item.version, 3, "the sending build's store version")
		spec.eq(item.generation, 0, "generation 0")
		spec.eq(item.attempts, 0, "no attempts")
		spec.eq(item.payload.amount, 5, "its payload")
		spec.list(backend:stored(D2.STORE, D2.KEY).userIds, { D2.USER }, "the user is recorded")
		spec.eq(backend.calls[1].kind, "update", "one UpdateAsync")
	end,
	["rule 66 a send needs no lease and works whatever the stored version"] = function()
		local backend, a, b = D2.world(2, { versions = { 3, 3 } })
		local decl = D2.message("grant")
		D2.join(backend, a)
		local before = D2.stored(backend)
		local sent = D2.send(b, decl, { amount = 1 }, D2.id(W, "m1"))
		backend:advance(1)
		spec.eq(sent.value.kind, "ok", "a send while another server owns the lease")
		local after = D2.stored(backend)
		spec.eq(after.lease.token, before.lease.token, "the lease is untouched")
		spec.eq(after.epoch, before.epoch, "so is the epoch")
		spec.eq(#after.inbox, 1, "the item is queued")

		local second, c = D2.world(1, { version = 2 })
		D2.seed(second, nil, {
			schemaVersion = 9,
			epoch = 4,
			lease = { token = "foreign", serverId = "x", jobId = "y", expiresAt = W + 1000 },
			payload = { coins = 7, gems = 0 },
		})
		local newer = D2.send(c, decl, { amount = 1 }, D2.id(W, "m2"))
		second:advance(1)
		spec.eq(newer.value.kind, "ok", "a send to a key stored at a newer version")
		local envelope = D2.stored(second)
		spec.eq(envelope.schemaVersion, 9, "the version is untouched")
		spec.eq(envelope.lease.token, "foreign", "and the lease")
		spec.eq(envelope.payload.coins, 7, "and the payload")
		spec.eq(#envelope.inbox, 1, "while the item is queued")
		spec.eq(envelope.inbox[1].version, 2, "stamped with the sender's version")
	end,
	["rule 66 and 71 a send to a player on this server prompts that session's save"] = function()
		local backend, a = D2.world()
		D2.join(backend, a)
		local decl = D2.message("grant")
		local record = D2.receiver(a, decl)
		local id = D2.id(W, "m1")
		D2.send(a, decl, { amount = 7 }, id)
		backend:advance(1)
		spec.eq(#record.calls, 1, "applied within a round trip")
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 7, "and saved")
		spec.list(envelope.seen, { id }, "acknowledged")
		spec.eq(#envelope.inbox, 0, "off the inbox")
	end,
	["rule 66 a send prompts only the target user's session"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		D2.join(backend, a)
		local other = D2.join(backend, a, D2.USER + 1)
		D2.receiver(a, decl)
		backend:advance(1)
		local before = D2.session(a, other).op
		D2.send(a, decl, { amount = 7 }, D2.id(W, "m1"))
		backend:advance(1)
		spec.eq(D2.stored(backend).payload.coins, 7, "the target's session applied it within a round trip")
		spec.eq(D2.session(a, other).op, before, "and no other session was asked to save")
	end,
	["rule 67 unavailable for an unreadable envelope or a non-retryable error, with nothing written"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		D2.seed(backend, nil, { format = 2 })
		local unreadable = D2.send(a, decl, { amount = 1 }, D2.id(W, "m1"))
		backend:advance(1)
		spec.eq(unreadable.value.kind, "unavailable", "an unreadable envelope")
		spec.eq(D2.stored(backend).format, 2, "left as it was")
		local second, b = D2.world()
		second:fail("reject", { server = b.id }, 1, { code = 400 })
		local rejected = D2.send(b, decl, { amount = 1 }, D2.id(W, "m2"))
		second:advance(1)
		spec.eq(rejected.value.kind, "unavailable", "a non-retryable error")
		spec.eq(D2.stored(second), nil, "nothing was written")
	end,
	["rule 67 timeout when never dispatched and uncertain when possibly appended, and a retry with the same id is ok"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		a.budget.update = 0
		local waiting = D2.send(a, decl, { amount = 1 }, D2.id(W, "m1"))
		backend:advance(a.limits.CALL_DEADLINE_SECONDS - 0.5)
		spec.eq(waiting.done, false, "waiting for budget")
		backend:advance(1)
		spec.eq(waiting.value.kind, "timeout", "timeout at the call deadline")
		spec.eq(#backend.calls, 0, "nothing was dispatched")
		a.budget.update = 1000
		local id = D2.id(W, "m2")
		backend:fail("committed", { server = a.id }, 100000)
		local doubtful = D2.send(a, decl, { amount = 1 }, id)
		backend:advance(a.limits.CALL_DEADLINE_SECONDS + 1)
		spec.eq(doubtful.value.kind, "uncertain", "uncertain: it may have appended")
		spec.eq(#inbox(backend), 1, "it did")
		for _, plan in backend.plans do
			plan.remaining = 0
		end
		local retry = D2.send(a, decl, { amount = 1 }, id)
		backend:advance(1)
		spec.eq(retry.value.kind, "ok", "a retry with the same id is ok")
		spec.eq(#inbox(backend), 1, "and appends nothing")
	end,
	["rule 67 overloaded when the key's pending cap refuses a send, with nothing written"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local gate = backend:gate("transform", { server = a.id })
		local handles = {}
		for index = 1, 33 do
			handles[index] = D2.send(a, decl, { amount = 1 }, D2.id(W, `m{index}`))
		end
		spec.eq(handles[33].value.kind, "overloaded", "the 33rd pending operation on one key is refused at once")
		gate.release()
		backend:advance(10)
		for index = 1, 32 do
			spec.eq(handles[index].value.kind, "ok", `send {index}`)
		end
		spec.eq(#inbox(backend), 32, "exactly the 32 accepted are queued")
	end,
	["rule 68 the same id with the same name and payload is ok without appending, anything else is reused"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local other = D2.message("other")
		local id = D2.id(W, "m1")
		local function payload()
			return { amount = 5, tags = { "a", "b" }, nested = { x = 1.5 } }
		end
		local function send(message, body)
			local handle = D2.send(a, message, body, id)
			backend:advance(1)
			return handle.value.kind
		end
		spec.eq(send(decl, payload()), "ok", "first")
		spec.eq(send(decl, payload()), "ok", "an identical resend")
		spec.eq(#inbox(backend), 1, "appends nothing")
		local different = payload()
		different.amount = 6
		spec.eq(send(decl, different), "reused", "another amount")
		different = payload()
		different.nested.x = 1.5000000000000002
		spec.eq(send(decl, different), "reused", "a number that differs in its last bit")
		different = payload()
		different.tags = { "b", "a" }
		spec.eq(send(decl, different), "reused", "another list order")
		different = payload()
		different.extra = true
		spec.eq(send(decl, different), "reused", "an extra key")
		different = payload()
		different.tags = { "a" }
		spec.eq(send(decl, different), "reused", "a shorter list")
		spec.eq(send(other, payload()), "reused", "another message name")
		local items = inbox(backend)
		spec.eq(#items, 1, "still one item")
		spec.eq(items[1].payload.amount, 5, "unchanged")
		spec.eq(items[1].name, "grant", "under its own name")
	end,
	["rule 68 a quarantined id compares like a queued one, and a seen id is ok by its exact string"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local other = D2.message("other")
		local quarantined = D2.id(W - 50, "q1")
		local seen = D2.id(W - 100, "seen1")
		local neighbour = D2.id(W - 100, "seen2")
		D2.seed(backend, nil, {
			quarantine = { D2.item(quarantined, "grant", { amount = 5 }, { reason = "payload", generation = 1 }) },
			seen = { seen },
		})
		local function send(message, body, id)
			local handle = D2.send(a, message, body, id)
			backend:advance(1)
			return handle.value.kind
		end
		spec.eq(send(decl, { amount = 5 }, quarantined), "ok", "the same message while quarantined")
		spec.eq(send(decl, { amount = 6 }, quarantined), "reused", "another payload")
		spec.eq(send(other, { amount = 5 }, quarantined), "reused", "another name")
		spec.eq(#inbox(backend), 0, "nothing was appended")
		spec.eq(send(other, { amount = 99 }, seen), "ok", "a seen id is ok whatever its message")
		spec.eq(#inbox(backend), 0, "and appends nothing")
		spec.eq(send(decl, { amount = 1 }, neighbour), "ok", "an id one character apart is another id")
		spec.eq(#inbox(backend), 1, "so it was appended")
	end,
	["rule 69 a send without an id makes an operationId that carries its creation second"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		D2.send(a, decl, { amount = 1 })
		D2.send(a, decl, { amount = 2 })
		backend:advance(1)
		local items = inbox(backend)
		spec.eq(#items, 2, "each call made its own id")
		assert(items[1].id ~= items[2].id, "distinct ids")
		for _, item in items do
			local seconds, suffix = string.match(item.id, "^o1%-(%w+)%-(%w+)$")
			assert(seconds ~= nil and suffix ~= nil, `the shape o1-<second>-<guid>: {item.id}`)
			spec.eq(tonumber(seconds, 36), W, "the creation second in base 36")
			assert(#item.id <= 64, "at most 64 bytes")
		end
	end,
	["rule 69 an id outside its age horizon is expired before dispatch or again inside the transform"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local function send(offset, suffix)
			local calls = #backend.calls
			local handle = D2.send(a, decl, { amount = 1 }, D2.id(D2.wallAt(backend) + offset, suffix))
			backend:advance(1)
			return handle.value.kind, #backend.calls - calls
		end
		local kind, calls = send(-604801, "old1")
		spec.eq(kind, "expired", "older than seven days")
		spec.eq(calls, 0, "refused before dispatch")
		kind, calls = send(-604800, "old2")
		spec.eq(kind, "expired", "an id that turns seven days old in flight")
		spec.eq(calls, 1, "is refused inside the transform with that call's wall time")
		kind = send(-604799, "old3")
		spec.eq(kind, "ok", "just inside the horizon")
		kind, calls = send(6, "new1")
		spec.eq(kind, "expired", "more than 5 s in the future")
		spec.eq(calls, 0, "refused before dispatch")
		kind = send(5, "new2")
		spec.eq(kind, "ok", "exactly 5 s ahead is allowed")
		spec.eq(#inbox(backend), 2, "only the two fresh ids were queued")
	end,
	["rule 69 an id at or below the seen floor without a record is expired, and a seen id below it is ok"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local floor = W - 1000
		local recorded = D2.id(floor - 50, "kept")
		D2.seed(backend, nil, { seenFloor = floor, seen = { recorded } })
		local function send(created, suffix, id)
			local handle = D2.send(a, decl, { amount = 1 }, id or D2.id(created, suffix))
			backend:advance(1)
			return handle.value.kind
		end
		spec.eq(send(floor, "at"), "expired", "an id created at the floor")
		spec.eq(send(floor - 10, "below"), "expired", "an id created below it")
		spec.eq(#inbox(backend), 0, "neither was queued")
		spec.eq(send(nil, nil, recorded), "ok", "an id still in seen is ok by its exact string")
		spec.eq(#inbox(backend), 0, "and appends nothing")
		spec.eq(send(floor + 1, "above"), "ok", "an id above the floor is queued")
	end,
	["rule 69 a ring full of young ids refuses an append as overloaded and an old head allows it"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local function ring(headAge)
			local seen = { D2.id(D2.wallAt(backend) - headAge, "head") }
			for index = 1, 1023 do
				seen[index + 1] = D2.id(W - 5000 - index, `s{index}`)
			end
			return seen
		end
		local function send(suffix)
			local handle = D2.send(a, decl, { amount = 1 }, D2.id(D2.wallAt(backend) - 30, suffix))
			backend:advance(1)
			return handle.value.kind
		end
		D2.seed(backend, nil, { seen = ring(10) })
		spec.eq(send("m1"), "overloaded", "the head would be evicted younger than 60 s")
		spec.eq(#inbox(backend), 0, "nothing was written")
		D2.seed(backend, nil, { seen = ring(59) })
		spec.eq(send("m2"), "overloaded", "59 s is still too young")
		D2.seed(backend, nil, { seen = ring(61) })
		spec.eq(send("m3"), "ok", "a head older than 60 s may be evicted")
		local short = ring(10)
		table.remove(short)
		D2.seed(backend, nil, { seen = short })
		spec.eq(send("m4"), "ok", "a ring with room needs no eviction")
	end,
	["rule 69 an ack evicts regardless of age and raises the floor, so the evicted id is expired"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local head = D2.id(W - 5, "head")
		local seen = { head }
		for index = 1, 1023 do
			seen[index + 1] = D2.id(W - 5000 - index, `s{index}`)
		end
		D2.seed(backend, nil, {
			seen = seen,
			inbox = { D2.item(D2.id(W - 10, "m1"), "grant", { amount = 3 }) },
		})
		local record = D2.receiver(a, decl)
		D2.join(backend, a)
		backend:advance(1)
		spec.eq(#record.calls, 1, "the queued item was applied")
		local envelope = D2.stored(backend)
		spec.eq(#envelope.seen, 1024, "the ring stays at its cap")
		spec.eq(envelope.seen[1024], D2.id(W - 10, "m1"), "the ack is the newest")
		for _, id in envelope.seen do
			assert(id ~= head, "the young head was evicted by the ack")
		end
		spec.eq(envelope.seenFloor, W - 5, "the floor rose to the evicted id's creation time")
		local again = D2.send(a, decl, { amount = 3 }, head)
		backend:advance(1)
		spec.eq(again.value.kind, "expired", "a resend of the evicted id is refused")
		spec.eq(D2.stored(backend).payload.coins, 3, "never applied twice")
	end,
	["rule 69 a replayed id is ok while in the ring and expired after its eviction, never applied twice"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local pa = D2.join(backend, a)
		D2.receiver(a, decl)
		local x = D2.id(W - 3600, "x")
		D2.send(a, decl, { amount = 1 }, x)
		backend:advance(1)
		spec.eq(D2.stored(backend).payload.coins, 1, "X applied")
		local envelope = backend:stored(D2.STORE, D2.KEY)
		for index = 1, 1023 do
			table.insert(envelope.value.seen, D2.id(W - 3000 - index, `f{index}`))
		end
		backend:seed(D2.STORE, D2.KEY, envelope.value, envelope.userIds, envelope.metadata)
		local replay = D2.send(a, decl, { amount = 1 }, x)
		backend:advance(1)
		spec.eq(replay.value.kind, "ok", "X is still in the ring: ok")
		spec.eq(D2.stored(backend).payload.coins, 1, "and not applied again")
		D2.send(a, decl, { amount = 10 }, D2.id(W - 3500, "y"))
		backend:advance(1)
		local after = D2.stored(backend)
		spec.eq(after.payload.coins, 11, "Y applied, which pushed X out of the full ring")
		spec.eq(after.seenFloor, W - 3600, "the floor is X's creation time")
		backend:advance(25)
		local late = D2.send(a, decl, { amount = 1 }, x)
		backend:advance(1)
		spec.eq(late.value.kind, "expired", "X is refused once evicted, also after the next renewal")
		spec.eq(D2.stored(backend).payload.coins, 11, "so it is never applied twice")
		assert(pa ~= nil, "present")
	end,
	["rule 70 a full inbox returns full and drops nothing, by count and by bytes"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local items = {}
		for index = 1, 128 do
			items[index] = D2.item(D2.id(W - 1000 - index, `m{index}`), "grant", { amount = index })
		end
		D2.seed(backend, nil, { inbox = items })
		local refused = D2.send(a, decl, { amount = 1 }, D2.id(W, "new"))
		backend:advance(1)
		spec.eq(refused.value.kind, "full", "the 129th message")
		local stored = inbox(backend)
		spec.eq(#stored, 128, "nothing was dropped")
		for index = 1, 128 do
			spec.eq(stored[index].id, items[index].id, `item {index} kept its place`)
		end

		local roomy, d = D2.world()
		local almost = {}
		for index = 1, 127 do
			almost[index] = D2.item(D2.id(W - 1000 - index, `m{index}`), "grant", { amount = index })
		end
		D2.seed(roomy, nil, { inbox = almost })
		local last = D2.send(d, decl, { amount = 1 }, D2.id(W, "last"))
		roomy:advance(1)
		spec.eq(last.value.kind, "ok", "the 128th message fits")
		spec.eq(#inbox(roomy), 128, "and is queued")

		local big, c = D2.world()
		local note = string.rep("n", 4000)
		local function fill(count)
			local list = {}
			for index = 1, count do
				list[index] = D2.item(D2.id(W - 1000 - index, `b{index}`), "grant", { amount = 1, note = note })
			end
			return list
		end
		local thirty, thirtyOne, thirtyTwo = fill(30), fill(31), fill(32)
		assert(#serde.encode("json", thirtyOne) <= 131072, "31 items fit the byte cap")
		assert(#serde.encode("json", thirtyTwo) > 131072, "32 do not")
		D2.seed(big, nil, { inbox = thirtyOne })
		local over = D2.send(c, decl, { amount = 1, note = note }, D2.id(W, "over"))
		big:advance(1)
		spec.eq(over.value.kind, "full", "the 32nd item passes INBOX_MAX_BYTES")
		spec.eq(#inbox(big), 31, "and nothing was dropped")
		D2.seed(big, nil, { inbox = thirty })
		local fits = D2.send(c, decl, { amount = 1, note = note }, D2.id(W, "fits"))
		big:advance(1)
		spec.eq(fits.value.kind, "ok", "the 31st item fits")
		spec.eq(#inbox(big), 31, "and was queued")
	end,
}
