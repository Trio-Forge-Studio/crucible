local spec = require("./lib/load")
local D2 = require("./lib/data-d2")

local W = D2.WALL
local LOADING, SEALED = 0, 2

local function item(id: string, generation: number, attempts: number)
	return D2.item(id, "none", { amount = 1 }, { generation = generation, attempts = attempts })
end

-- One session with a quarantine mark staged at generation 2 and 1 attempt, delivered an envelope in which the item has
-- moved to `generation` and `attempts`.
local function delivered(generation: number, attempts: number)
	local backend, a = D2.world()
	local player = D2.join(backend, a)
	local session = D2.session(a, player)
	session.marks["m1"] = { id = "m1", generation = 2, attempts = 1, quarantine = "attempts" }
	a.store.inbox:deliver(session, { inbox = { item("m1", generation, attempts) } })
	return a, session
end

local function foreignLease()
	return { token = "other", serverId = "elsewhere", jobId = "job-elsewhere", expiresAt = W + 80 }
end

return {
	["A16 rule 72 an ack at exactly the confirmed revision is retired by the envelope that confirms it"] = function()
		local backend, a = D2.world()
		local player = D2.join(backend, a)
		local session = D2.session(a, player)
		session.confirmed = 3
		session.acks["x2"] = 2
		session.acks["x3"] = 3
		session.acks["x4"] = 4
		a.store.inbox:deliver(session, { inbox = {} })
		spec.eq(session.acks["x2"], nil, "an ack below the confirmed revision is retired")
		spec.eq(session.acks["x3"], nil, "an ack at the confirmed revision is retired")
		spec.eq(session.acks["x4"], 4, "an ack above it stays")
	end,
	["A16 rule 72 an applied item's ack is gone from the session once the save that holds it is confirmed"] = function()
		local backend, a = D2.world()
		local id = D2.id(W - 10, "m1")
		D2.seed(backend, nil, { inbox = { D2.item(id, "grant", { amount = 3 }) } })
		D2.receiver(a, D2.message("grant"))
		local player = D2.join(backend, a)
		backend:advance(2)
		local session = D2.session(a, player)
		local envelope = D2.stored(backend)
		spec.list(envelope.seen, { id }, "the ack is in seen")
		spec.eq(envelope.commit.revision, 1, "the save held revision 1")
		spec.eq(session.confirmed, 1, "and it is confirmed")
		spec.eq(next(session.acks), nil, "so the session holds no ack")
	end,
	["A16 rule 70 a quarantine mark whose item moved by one generation and a different attempt count is not a full quarantine"] = function()
		local raised, raisedSession = delivered(3, 5)
		spec.eq(raised.engine.logged("quarantine full"), 0, "attempts raised by another writer: no diagnostic")
		spec.eq(raisedSession.marks["m1"], nil, "the mark is retired")
		local reset, resetSession = delivered(3, 0)
		spec.eq(reset.engine.logged("quarantine full"), 0, "attempts reset by a retry: no diagnostic")
		spec.eq(resetSession.marks["m1"], nil, "the mark is retired")
		local full, fullSession = delivered(3, 1)
		spec.eq(
			full.engine.logged("quarantine full for FD.Players 4120; clear it with clearQuarantined"),
			1,
			"the same attempts at the next generation is the item that stayed live"
		)
		spec.eq(fullSession.marks["m1"], nil, "the mark is retired")
	end,
	["A16 rule 71 a send to a user whose session on this server is loading requests no write"] = function()
		local backend, a = D2.world()
		D2.seed(backend, nil, { epoch = 1, lease = foreignLease() })
		local decl = D2.message("grant")
		D2.receiver(a, decl)
		local player = a.join(D2.USER)
		a.run(function()
			return a.store:load(player, "join")
		end)
		backend:advance(2)
		local session = D2.session(a, player)
		spec.eq(session.phase, LOADING, "the load is held by the other server's lease")
		local sent = D2.send(a, decl, { amount = 1 }, D2.id(W - 5, "m1"))
		backend:advance(1)
		spec.eq(sent.value.kind, "ok", "the message is queued")
		spec.eq(session.phase, LOADING, "the session is still loading")
		spec.eq(session.want, nil, "no write was requested for it")
		spec.eq(session.op, 0, "and none was started")
	end,
	["A16 rule 71 a send answered while the user's session on this server is sealed requests no write"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		D2.receiver(a, decl)
		local player = D2.join(backend, a)
		local session = D2.session(a, player)
		local gate = backend:gate("response", { key = D2.KEY })
		local sent = D2.send(a, decl, { amount = 1 }, D2.id(W - 5, "m2"))
		backend:advance(0.3)
		spec.eq(sent.done, false, "the send's response is parked")
		local handoff = a.run(function()
			return a.store:handoff(player)
		end)
		backend:advance(0.3)
		spec.eq(session.phase, SEALED, "the handoff sealed the session behind the send")
		local before = session.op
		gate.release()
		backend:advance(2)
		spec.eq(sent.value.kind, "ok", "the message is queued")
		spec.eq(handoff.value, "ok", "and the handoff committed")
		spec.eq(session.want, nil, "no write was requested for the sealed session")
		spec.eq(session.op, before, "and none was started")
	end,
	["A16 rule 69 a seen ring's head is refused at 59.9 seconds old and accepted at 60.1"] = function()
		local backend, a = D2.world()
		local decl = D2.message("grant")
		local seen = { D2.id(W, "head") }
		for index = 1, 1023 do
			seen[index + 1] = D2.id(W - 5000 - index, `s{index}`)
		end
		D2.seed(backend, nil, { seen = seen })
		backend:advance(59.85)
		local refused = D2.send(a, decl, { amount = 1 }, D2.id(W + 55, "m1"))
		backend:advance(0.2)
		spec.eq(refused.value.kind, "overloaded", "the head is 59.9 seconds old at the transform")
		spec.eq(#D2.stored(backend).inbox, 0, "nothing was written")
		spec.eq(D2.stored(backend).seen[1], seen[1], "and the head is still first")
		local accepted = D2.send(a, decl, { amount = 1 }, D2.id(W + 55, "m1"))
		backend:advance(0.2)
		spec.eq(accepted.value.kind, "ok", "the head is 60.1 seconds old at the transform")
		spec.eq(#D2.stored(backend).inbox, 1, "the item was queued")
		spec.eq(D2.stored(backend).inbox[1].id, D2.id(W + 55, "m1"), "it is the new message")
	end,
}
