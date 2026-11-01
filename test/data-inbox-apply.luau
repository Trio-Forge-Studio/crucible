local spec = require("./lib/load")
local D2 = require("./lib/data-d2")

local W = D2.WALL

local function inbox(backend)
	return D2.stored(backend).inbox
end

local function clear(backend)
	for _, plan in backend.plans do
		plan.remaining = 0
	end
end

local function clearQuarantined(realm, ids, action)
	return realm.run(function()
		return realm.store.inbox:clearQuarantined(D2.USER, ids, action)
	end)
end

-- Rule 72's R3: a mark staged against generation g reaches the disk after a clearQuarantined retry raised the item to
-- g + 2. The mark rides the sealed snapshot into the same server's next acquisition, where it must skip.
local function staleMark(reason: string)
	local backend, a, b = D2.world(2)
	local id = D2.id(W - 10, "m1")
	local generation = if reason == "payload" then 0 else 3
	local item = if reason == "payload"
		then D2.item(id, "grant", { amount = "bad" })
		else D2.item(id, "grant", { amount = 5 }, { attempts = 4, generation = 3 })
	D2.seed(backend, nil, { inbox = { item } })
	D2.receiver(a, D2.message("grant"), {
		throws = function()
			return true
		end,
	})
	if reason == "payload" then
		backend:fail("committed", { server = a.id }, 100000, { after = 1 })
	end
	local player = D2.join(backend, a)
	if reason == "attempts" then
		backend:advance(59.4)
		backend:fail("committed", { server = a.id }, 100000)
	end
	backend:advance(if reason == "payload" then 1 else 1.5)
	local session = D2.session(a, player)
	assert(session.marks[id] ~= nil, "the session holds its mark")
	spec.eq(session.marks[id].generation, generation, "read at the generation the item had")
	local quarantined = D2.stored(backend).quarantine[1]
	assert(quarantined ~= nil, "the mark committed: the item is quarantined")
	spec.eq(quarantined.generation, generation + 1, "at the next generation")
	spec.eq(quarantined.reason, reason, "with its reason")
	D2.set(a, player, function()
		return { coins = 5, gems = 0 }
	end)
	local cleared = clearQuarantined(b, { id }, "retry")
	backend:advance(1)
	spec.eq(cleared.value.value, 1, "the operator retried one item")
	local live = inbox(backend)[1]
	spec.eq(live.generation, generation + 2, "the retry raised the generation again")
	spec.eq(live.attempts, 0, "and reset the attempts")
	backend:advance(80)
	spec.eq(session.sealedBy, "authority", "the old session lost authority")
	clear(backend)
	local calls = 0
	a.store.inbox:receive(D2.lenient("grant"), function(_, data)
		calls += 1
		local value = table.clone(data)
		value.coins += 100
		return value
	end)
	a.leave(player)
	local again = a.join(D2.USER)
	local loading = a.run(function()
		return a.store:load(again, "join")
	end)
	backend:advance(5)
	spec.eq(loading.value, true, "the user rejoined the same server")
	spec.eq(calls, 1, "the item was applied once by the new build")
	local envelope = D2.stored(backend)
	spec.eq(envelope.payload.coins, 105, "on the sealed snapshot, with its change")
	spec.eq(#envelope.inbox, 0, "the stale mark did not requarantine it")
	spec.eq(#envelope.quarantine, 0, "the quarantine is empty")
	spec.list(envelope.seen, { id }, "and the ack is in seen")
end

return {
	["rule 72 items apply in stored order, each ack in the revision of its change"] = function()
		local backend, a = D2.world()
		local ids = { D2.id(W - 30, "m1"), D2.id(W - 20, "m2"), D2.id(W - 10, "m3") }
		D2.seed(backend, nil, {
			inbox = {
				D2.item(ids[1], "grant", { amount = 1 }),
				D2.item(ids[2], "grant", { amount = 2 }),
				D2.item(ids[3], "grant", { amount = 4 }),
			},
		})
		local record = D2.receiver(a, D2.message("grant"))
		local player = D2.join(backend, a)
		backend:advance(1)
		local order = {}
		for _, call in record.calls do
			table.insert(order, call.amount)
		end
		spec.list(order, { 1, 2, 4 }, "applied in stored order")
		spec.eq(D2.session(a, player).revision, 3, "one revision per item")
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 7, "stored")
		spec.eq(#envelope.inbox, 0, "off the inbox")
		spec.list(envelope.seen, ids, "and in seen, in the same order")
	end,
	["rule 72 an unchanged return consumes the item"] = function()
		local backend, a = D2.world()
		local id = D2.id(W - 10, "m1")
		D2.seed(backend, nil, { inbox = { D2.item(id, "grant", { amount = 1 }) } })
		local record = D2.receiver(a, D2.message("grant"), { unchanged = true })
		local player = D2.join(backend, a)
		backend:advance(1)
		spec.eq(#record.calls, 1, "the receiver ran")
		spec.eq(D2.session(a, player).revision, 1, "an unchanged return still counts a revision")
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 0, "no data change")
		spec.list(envelope.seen, { id }, "but the item is consumed")
		spec.eq(#envelope.inbox, 0, "off the inbox")
	end,
	["rule 72 and 109 replay before the ack is saved: a crash leaves the item queued and the change unsaved, so the next server applies it once"] = function()
		local backend, a, b = D2.world(2)
		local id = D2.id(W - 10, "m1")
		D2.seed(backend, nil, { inbox = { D2.item(id, "grant", { amount = 3 }) } })
		local first = D2.receiver(a, D2.message("grant"))
		local gate = backend:gate("commit", { server = a.id, index = 2 })
		D2.join(backend, a)
		backend:advance(1)
		spec.eq(#first.calls, 1, "applied in memory")
		local parked = D2.stored(backend)
		spec.eq(parked.payload.coins, 0, "the change is not on disk")
		spec.eq(#parked.inbox, 1, "the item is still queued")
		spec.eq(#parked.seen, 0, "and not acknowledged")
		assert(#gate.parked == 1, "the save is parked before its commit")
		a.crash()
		local second = D2.receiver(b, D2.message("grant"))
		D2.joinSlow(backend, b)
		backend:advance(2)
		spec.eq(#second.calls, 1, "the next server applied it once")
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 3, "the change is stored exactly once")
		spec.list(envelope.seen, { id }, "and acknowledged")
	end,
	["rule 72 an item with no receiver on this server is left queued for a server that has one"] = function()
		local backend, a, b = D2.world(2)
		local referral = D2.id(W - 20, "r1")
		local grant = D2.id(W - 10, "g1")
		D2.seed(backend, nil, {
			inbox = { D2.item(referral, "referral", { amount = 9 }), D2.item(grant, "grant", { amount = 1 }) },
		})
		D2.receiver(a, D2.message("grant"))
		local player = D2.join(backend, a)
		backend:advance(60)
		local envelope = D2.stored(backend)
		spec.eq(#envelope.inbox, 1, "the unhandled item is still queued")
		spec.eq(envelope.inbox[1].id, referral, "it is the referral")
		spec.eq(envelope.inbox[1].generation, 0, "untouched")
		spec.eq(#envelope.quarantine, 0, "never quarantined")
		spec.eq(envelope.payload.coins, 1, "the other item was applied")
		a.leave(player)
		a.run(function()
			a.store:leave(player)
		end)
		backend:advance(2)
		local record = D2.receiver(b, D2.message("referral"))
		D2.join(backend, b)
		backend:advance(2)
		spec.eq(#record.calls, 1, "the server with a receiver applied it")
		spec.list(D2.stored(backend).seen, { grant, referral }, "both are acknowledged")
	end,
	["rule 72 an item from a newer build stays queued, is diagnosed once per id per session, and a newer build applies it"] = function()
		local backend, a, b = D2.world(2, { versions = { 2, 5 } })
		local id = D2.id(W - 10, "n1")
		D2.seed(backend, nil, { inbox = { D2.item(id, "grant", { amount = 4 }, { version = 5 }) } })
		local old = D2.receiver(a, D2.message("grant"))
		local player = D2.join(backend, a)
		backend:advance(120)
		spec.eq(#old.calls, 0, "the older build did not apply it")
		local envelope = D2.stored(backend)
		spec.eq(#envelope.inbox, 1, "it stays queued")
		spec.eq(#envelope.quarantine, 0, "never quarantined")
		spec.eq(envelope.inbox[1].generation, 0, "and unmarked")
		spec.eq(a.engine.logged("newer build"), 1, "diagnosed once, though renewals delivered it again")
		a.leave(player)
		a.run(function()
			a.store:leave(player)
		end)
		backend:advance(2)
		local fresh = D2.receiver(b, D2.message("grant"))
		D2.join(backend, b)
		backend:advance(2)
		spec.eq(#fresh.calls, 1, "the newer build applied it")
		spec.eq(D2.stored(backend).payload.coins, 4, "once")
	end,
	["rule 72 a payload this build refuses is quarantined with reason payload and its unknown keys intact"] = function()
		local backend, a = D2.world()
		local bad = D2.id(W - 20, "b1")
		local good = D2.id(W - 10, "g1")
		D2.seed(backend, nil, {
			inbox = {
				D2.item(bad, "grant", { amount = "many", future = { kept = true } }),
				D2.item(good, "grant", { amount = 2 }),
			},
		})
		local record = D2.receiver(a, D2.message("grant"))
		D2.join(backend, a)
		backend:advance(5)
		spec.eq(#record.calls, 1, "only the valid item reached the receiver")
		local envelope = D2.stored(backend)
		spec.eq(#envelope.inbox, 0, "both left the inbox")
		spec.eq(#envelope.quarantine, 1, "one is quarantined")
		local item = envelope.quarantine[1]
		spec.eq(item.id, bad, "the invalid one")
		spec.eq(item.reason, "payload", "reason payload")
		spec.eq(item.generation, 1, "its generation rose once")
		spec.eq(item.attempts, 0, "with no attempts")
		spec.eq(item.payload.future.kept, true, "the payload keeps its unknown keys")
		spec.list(envelope.seen, { good }, "the valid one is acknowledged")
		spec.eq(a.engine.logged("refuses"), 1, "diagnosed once")
		spec.eq(a.engine.logged("many"), 0, "without the value")
	end,
	["rule 73 a throwing receiver retries after 5, 15, 60 and 60 s, then quarantines with reason attempts, and blocks nothing"] = function()
		local backend, a = D2.world()
		local bad = D2.id(W - 20, "b1")
		local good = D2.id(W - 10, "g1")
		D2.seed(backend, nil, {
			inbox = {
				D2.item(bad, "grant", { amount = 13, future = "kept" }),
				D2.item(good, "grant", { amount = 1 }),
			},
		})
		local record = D2.receiver(a, D2.message("grant"), {
			throws = function(payload)
				return payload.amount == 13
			end,
		})
		D2.join(backend, a)
		backend:advance(1)
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 1, "the good item was applied at once")
		spec.eq(#envelope.inbox, 1, "the poisoned item stays")
		spec.eq(envelope.inbox[1].attempts, 1, "one persisted attempt")
		spec.eq(envelope.inbox[1].generation, 1, "its generation rose")
		spec.eq(envelope.inbox[1].payload.future, "kept", "its payload is untouched")
		backend:advance(200)
		local attempts = {}
		for _, call in record.calls do
			if call.amount == 13 then
				table.insert(attempts, call.at)
			end
		end
		spec.eq(#attempts, 5, "five attempts in all")
		local waits = { 5, 15, 60, 60 }
		for index, wait in waits do
			local gap = attempts[index + 1] - attempts[index]
			assert(
				gap >= wait - 0.01 and gap < wait + 0.5,
				`attempt {index + 1} came {gap} s after the last, not {wait}`
			)
		end
		local final = D2.stored(backend)
		spec.eq(#final.inbox, 0, "off the live inbox")
		spec.eq(#final.quarantine, 1, "quarantined")
		spec.eq(final.quarantine[1].reason, "attempts", "with reason attempts")
		spec.eq(final.quarantine[1].attempts, 5, "after five attempts")
		spec.eq(final.quarantine[1].generation, 5, "each attempt raised the generation")
		spec.eq(final.quarantine[1].payload.future, "kept", "and its payload is kept")
		spec.eq(final.payload.coins, 1, "the good item was applied once")
		spec.eq(a.engine.logged("failed attempt"), 1, "diagnosed once for the id in this session")
		spec.eq(a.engine.logged("receiver failed"), 0, "without the error text")
	end,
	["rule 73 quarantined lists the items with one read, and clearQuarantined retry returns them to the live inbox"] = function()
		local backend, a, b = D2.world(2)
		local id = D2.id(W - 20, "q1")
		D2.seed(backend, nil, {
			quarantine = {
				D2.item(id, "grant", { amount = 6, future = 1 }, { reason = "attempts", attempts = 5, generation = 5 }),
			},
		})
		local listed = b.run(function()
			return b.store.inbox:quarantined(D2.USER)
		end)
		backend:advance(1)
		spec.eq(listed.value.kind, "ok", "listed")
		spec.eq(#listed.value.value, 1, "one item")
		local entry = listed.value.value[1]
		spec.eq(entry.id, id, "its id")
		spec.eq(entry.name, "grant", "its name")
		spec.eq(entry.reason, "attempts", "its reason")
		spec.eq(entry.attempts, 5, "its attempts")
		spec.eq(entry.sentAt, W - 20, "the id's creation time")
		spec.eq(entry.payload.future, 1, "its payload with unknown keys")
		spec.eq(#backend.calls, 1, "one call")
		spec.eq(backend.calls[1].kind, "read", "a GetAsync")
		local record = D2.receiver(a, D2.message("grant"))
		D2.join(backend, a)
		local cleared = clearQuarantined(a, { id, "unknown-id" }, "retry")
		backend:advance(1)
		spec.eq(cleared.value.value, 1, "one item changed: the unlisted id is ignored")
		spec.eq(#record.calls, 1, "the session on this server read the change within a round trip")
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 6, "and applied it")
		spec.list(envelope.seen, { id }, "once")
		spec.eq(#envelope.quarantine, 0, "the quarantine is empty")
	end,
	["rule 73 clearQuarantined retry returns full with nothing changed when the inbox has no room"] = function()
		local backend, a = D2.world()
		local items = {}
		for index = 1, 128 do
			items[index] = D2.item(D2.id(W - 1000 - index, `m{index}`), "unhandled", { amount = 1 })
		end
		local id = D2.id(W - 20, "q1")
		D2.seed(backend, nil, {
			inbox = items,
			quarantine = { D2.item(id, "grant", { amount = 6 }, { reason = "payload", generation = 1 }) },
		})
		local result = clearQuarantined(a, { id }, "retry")
		backend:advance(1)
		spec.eq(result.value.kind, "full", "full")
		local envelope = D2.stored(backend)
		spec.eq(#envelope.inbox, 128, "the inbox is as it was")
		spec.eq(#envelope.quarantine, 1, "and the item is still quarantined")
		spec.eq(envelope.quarantine[1].generation, 1, "untouched")
	end,
	["rule 73 clearQuarantined discard records the exact ids in seen, so a resend is deduplicated and never applied"] = function()
		local backend, a = D2.world()
		local one = D2.id(W - 30, "q1")
		local two = D2.id(W - 20, "q2")
		D2.seed(backend, nil, {
			quarantine = {
				D2.item(one, "grant", { amount = 6 }, { reason = "payload", generation = 1 }),
				D2.item(two, "grant", { amount = 7 }, { reason = "attempts", generation = 5, attempts = 5 }),
			},
		})
		local result = clearQuarantined(a, { one, two }, "discard")
		backend:advance(1)
		spec.eq(result.value.value, 2, "two items discarded")
		local envelope = D2.stored(backend)
		spec.eq(#envelope.quarantine, 0, "the quarantine is empty")
		spec.eq(#envelope.inbox, 0, "and nothing returned to the inbox")
		spec.list(envelope.seen, { one, two }, "the exact ids are in seen")
		local resend = D2.send(a, D2.message("grant"), { amount = 6 }, one)
		backend:advance(1)
		spec.eq(resend.value.kind, "ok", "a resend is ok")
		spec.eq(#inbox(backend), 0, "and appends nothing")
	end,
	["rule 72 a full quarantine leaves an item live and unapplied, diagnosed once, by count and by bytes"] = function()
		for _, variant in { "count", "bytes" } do
			local backend, a = D2.world()
			local held = {}
			local note = string.rep("q", 20000)
			if variant == "count" then
				for index = 1, 8 do
					held[index] = D2.item(
						D2.id(W - 500 - index, `h{index}`),
						"grant",
						{ amount = 1 },
						{ reason = "payload", generation = 1 }
					)
				end
			else
				for index = 1, 3 do
					held[index] = D2.item(
						D2.id(W - 500 - index, `h{index}`),
						"grant",
						{ amount = 1, note = note },
						{ reason = "payload", generation = 1 }
					)
				end
			end
			local bad = D2.id(W - 20, "bad")
			local good = D2.id(W - 10, "good")
			D2.seed(backend, nil, {
				quarantine = held,
				inbox = {
					D2.item(bad, "grant", { amount = "no", note = if variant == "bytes" then note else nil }),
					D2.item(good, "grant", { amount = 2 }),
				},
			})
			local record = D2.receiver(a, D2.message("grant"))
			D2.join(backend, a)
			backend:advance(200)
			local envelope = D2.stored(backend)
			spec.eq(#envelope.quarantine, #held, `{variant}: the quarantine did not grow`)
			spec.eq(#envelope.inbox, 1, `{variant}: the item stays live`)
			spec.eq(envelope.inbox[1].id, bad, `{variant}: it is the refused one`)
			spec.eq(#record.calls, 1, `{variant}: the receiver saw only the valid item`)
			spec.eq(envelope.payload.coins, 2, `{variant}: which was applied`)
			spec.eq(
				a.engine.logged("quarantine full for FD.Players 4120; clear it with clearQuarantined"),
				1,
				`{variant}: diagnosed once`
			)
		end
	end,
	["rule 72 R3 a stale payload quarantine mark held across an ambiguous commit and a clear is skipped by its generation"] = function()
		staleMark("payload")
	end,
	["rule 72 R3 a stale attempts quarantine mark is skipped by its generation, and the attempts the clear reset stay reset"] = function()
		staleMark("attempts")
	end,
	["rule 72 an ack removes its id from the quarantine, wherever a stale mark put the item"] = function()
		local backend, a = D2.world()
		local id = D2.id(W - 10, "m1")
		D2.seed(backend, nil, { inbox = { D2.item(id, "grant", { amount = 3 }) } })
		local record = D2.receiver(a, D2.message("grant"))
		local gate = backend:gate("transform", { server = a.id, index = 2 })
		D2.join(backend, a)
		backend:advance(1)
		spec.eq(#record.calls, 1, "applied in memory")
		local entry = backend:stored(D2.STORE, D2.KEY)
		local item = table.remove(entry.value.inbox, 1)
		item.generation = 1
		item.reason = "payload"
		table.insert(entry.value.quarantine, item)
		backend:seed(D2.STORE, D2.KEY, entry.value, entry.userIds, entry.metadata)
		gate.release()
		backend:advance(1)
		local envelope = D2.stored(backend)
		spec.eq(#envelope.quarantine, 0, "the ack took it out of the quarantine")
		spec.eq(#envelope.inbox, 0, "it is not in the inbox either")
		spec.list(envelope.seen, { id }, "its id is in seen")
		spec.eq(envelope.payload.coins, 3, "with the change it acknowledges")
	end,
	["rule 71 an owner's write removes only the acks it carries, so an append between its read and write survives"] = function()
		local backend, a, b = D2.world(2)
		local first = D2.id(W - 20, "m0")
		local second = D2.id(W - 10, "m1")
		D2.seed(backend, nil, { inbox = { D2.item(first, "grant", { amount = 2 }) } })
		local record = D2.receiver(a, D2.message("grant"))
		local gate = backend:gate("commit", { server = a.id, index = 2 })
		D2.join(backend, a)
		backend:advance(1)
		spec.eq(#record.calls, 1, "the first item is applied and its save is parked before its commit")
		local sent = D2.send(b, D2.message("grant"), { amount = 4 }, second)
		backend:advance(1)
		spec.eq(sent.value.kind, "ok", "another server appended meanwhile")
		gate.release()
		local envelope = D2.stored(backend)
		spec.list(envelope.seen, { first }, "the owner's write acknowledged only its own item")
		spec.eq(#envelope.inbox, 1, "the concurrent append survived")
		spec.eq(envelope.inbox[1].id, second, "it is the appended one")
		backend:advance(2)
		spec.eq(#record.calls, 2, "the owner read it from the response and applied it")
		local final = D2.stored(backend)
		spec.eq(final.payload.coins, 6, "both changes are stored")
		spec.list(final.seen, { first, second }, "both are acknowledged")
	end,
	["rule 71 a message to an idle player on another server is read within one renewal"] = function()
		local backend, a, b = D2.world(2)
		local record = D2.receiver(a, D2.message("grant"))
		D2.join(backend, a)
		local start = backend.now
		local sent = D2.send(b, D2.message("grant"), { amount = 5 }, D2.id(W, "m1"))
		backend:advance(1)
		spec.eq(sent.value.kind, "ok", "sent")
		spec.eq(#record.calls, 0, "not read at once: the owner is on another server")
		backend:advance(24)
		spec.eq(#record.calls, 1, "read at the owner's next renewal")
		local late = record.calls[1].at - start
		assert(late <= 23, `read ${late} s after the send`)
		spec.eq(D2.stored(backend).payload.coins, 5, "and saved")
	end,
	["rule 72 a stale scan never reapplies an item whose ack is not saved yet"] = function()
		local backend, a = D2.world()
		local retried = D2.id(W - 20, "j1")
		local fresh = D2.id(W - 10, "m1")
		D2.seed(backend, nil, {
			inbox = {
				D2.item(retried, "grant", { amount = 100 }, { attempts = 2, generation = 3 }),
				D2.item(fresh, "grant", { amount = 1 }),
			},
		})
		local record = D2.receiver(a, D2.message("grant"))
		local gate = backend:gate("transform", { server = a.id, index = 2 })
		local player = D2.join(backend, a)
		backend:advance(13)
		spec.eq(#record.calls, 1, "the fresh item was applied; the retried one waits out its delay")
		backend:advance(4)
		spec.eq(#record.calls, 2, "the retried item came due and ran once")
		spec.eq(D2.session(a, player).value.coins, 101, "the fresh item was not applied a second time")
		gate.release()
		backend:advance(2)
		local envelope = D2.stored(backend)
		spec.eq(envelope.payload.coins, 101, "stored once each")
		spec.eq(#envelope.inbox, 0, "both left the inbox")
		spec.eq(#envelope.seen, 2, "and are acknowledged")
	end,
	["rule 52 and 67 sends are accepted until the dispatch cutoff and return closing after it"] = function()
		local backend, a = D2.sweepWorld(function(realm)
			D2.sleep(realm, 40)
		end)
		local decl = D2.message("grant")
		backend:advance(1)
		a.close()
		backend:advance(1)
		local early = D2.send(a, decl, { amount = 1 }, D2.id(W, "m1"), D2.USER + 1)
		backend:advance(1)
		spec.eq(early.value.kind, "ok", "accepted in the open phase")
		backend:advance(20)
		local late = D2.send(a, decl, { amount = 1 }, D2.id(W, "m2"), D2.USER + 1)
		backend:advance(1)
		spec.eq(late.value.kind, "ok", "and in the final phase, before the cutoff")
		backend:advance(4)
		local calls = #backend.calls
		local closed = D2.send(a, decl, { amount = 1 }, D2.id(W, "m3"), D2.USER + 1)
		spec.eq(closed.value.kind, "closing", "refused once the cutoff passed")
		backend:advance(1)
		spec.eq(#backend.calls, calls, "before dispatch")
		spec.eq(#D2.stored(backend, D2.USER + 1).inbox, 2, "the first two are queued, the third is not")
	end,
}
