local World = require("./lib/sync-world")
local spec = require("./lib/load")

local function declare(r)
	local n = r.n
	return {
		A = r.emit.replicated("Test.A", 0, n.f64),
		B = r.emit.replicated("Test.B", 0, n.f64),
	}
end

local function serve(s, d, w)
	s.service("TestService", function()
		w.a = s.entry.publish(d.A)
		w.b = s.entry.publish(d.B)
	end)
end

local function newWorld()
	local w = World.new({ declare = declare, serve = serve })
	w.a:set(1)
	w.b:set(2)
	return w
end

local function faults(w, client): number
	return #w.link:sent(function(entry)
		return entry.from == client.name and entry.opcode == 0
	end)
end

-- Rule 65's one outcome: values and the revision kept, every status failed, one fault, later frames ignored.
local function latched(w, client, label: string, fault: number)
	spec.eq(client.d.A(), 1, `{label}: A kept`)
	spec.eq(client.d.B(), 2, `{label}: B kept`)
	spec.eq(client.api.syncStatus(client.d.A), "failed", `{label}: A failed`)
	spec.eq(client.api.syncStatus(client.d.B), "failed", `{label}: B failed`)
	spec.eq(faults(w, client), fault, `{label}: fault messages`)
	spec.eq(client.logged("Crucible sync stopped"), 1, `{label}: one diagnostic`)
	w.a:set(50)
	w:heartbeat()
	spec.eq(client.d.A(), 1, `{label}: later frames are ignored`)
	spec.eq(client.logged("Crucible sync stopped"), 1, `{label}: and report nothing`)
end

local function update(revision: number, values, failed)
	return { failed = failed or {}, kind = "update", revision = revision, values = values }
end

local violations = {
	["revision gap"] = update(3, { { key = "Test.A", value = 9 } }),
	["repeated revision"] = update(1, { { key = "Test.A", value = 9 } }),
	["second snapshot"] = { failed = {}, kind = "snapshot", revision = 1, values = { { key = "Test.A", value = 9 } } },
	["unordered keys"] = update(2, { { key = "Test.B", value = 9 }, { key = "Test.A", value = 9 } }),
	["duplicate key"] = update(2, { { key = "Test.A", value = 9 }, { key = "Test.A", value = 8 } }),
	["key in both lists"] = update(2, { { key = "Test.A", value = 9 } }, { "Test.A" }),
	["unordered failed list"] = update(2, {}, { "Test.B", "Test.A" }),
}

local tests = {
	["62 65 K7 R3 a failure preparing the second of two values commits nothing, fails every status and faults once"] = function()
		local w = newWorld()
		local prepared = 0
		local ann = w:join("Ann", {
			patch = function(c)
				local values = c.graph.module("internal/sync/values")
				local reuse = values.reuse
				values.reuse = function(current, decoded)
					prepared += 1
					if prepared == 4 then
						error("injected preparation failure")
					end
					return reuse(current, decoded)
				end
			end,
		})
		spec.eq(prepared, 2, "the snapshot prepared both values")
		w.a:set(10)
		w.b:set(20)
		w:heartbeat()
		spec.eq(prepared, 4, "the update failed on its second value")
		latched(w, ann, "preparation", 1)
		local fault = w.link:sent(function(entry)
			return entry.from == "Ann" and entry.opcode == 0
		end)[1]
		spec.eq(
			buffer.readu32(fault.packet, buffer.len(fault.packet) - 4),
			1,
			"the fault carries the last applied revision"
		)
		w:finish()
	end,
	["62 a failed frame latches the client failed without a fault"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		w:inject(ann, { failed = {}, kind = "failed", revision = 1, values = {} })
		latched(w, ann, "failed frame", 0)
		w:finish()
	end,
	["62 an update before any snapshot latches the client failed"] = function()
		local w = World.new({ declare = declare })
		local ann = w:join("Ann")
		spec.eq(#w:frames(ann), 0, "no publication, no snapshot")
		w:inject(ann, update(1, { { key = "Test.A", value = 9 } }))
		spec.eq(ann.d.A(), 0, "nothing applied")
		spec.eq(ann.api.syncStatus(ann.d.A), "failed", "failed")
		spec.eq(faults(w, ann), 1, "one fault")
		w:finish()
	end,
	["65 a fault from a client retires only that client's view and kicks that player"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		local bea = w:join("Bea")
		local fault = ann.graph.module("internal/sync/frame").SyncFaultMessage
		ann.Network.send(fault, 1)
		w.link:flush()
		spec.eq(#w.link.kicks, 1, "one kick")
		spec.eq(w.link.kicks[1].player, "Ann", "of the faulting player")
		spec.eq(w.link.kicks[1].message, "Connection problem, please rejoin.", "with KICK_MESSAGE")
		spec.eq(w:sync().peers[ann.player], nil, "Ann's view ended")
		w.a:set(5)
		w:heartbeat()
		spec.eq(bea.d.A(), 5, "Bea keeps syncing")
		spec.eq(#w:frames(ann), 1, "Ann gets nothing more")
		w:finish()
	end,
	["60 a client computed over a replicated value recomputes after each applied frame, observed or not"] = function()
		local w = World.new({
			declare = function(r)
				return { Score = r.emit.replicated("Job.Score", 0, r.n.f64) }
			end,
			serve = function(s, d, world)
				s.service("JobService", function()
					world.score = s.entry.publish(d.Score)
				end)
			end,
		})
		local ann = w:join("Ann")
		local api = ann.api
		local doubled = api.computed(function()
			return ann.d.Score() * 2
		end)
		local seen = {}
		api.scope(function()
			api.effect(function()
				table.insert(seen, doubled())
			end)
		end)
		local unobserved = api.computed(function()
			return ann.d.Score() + 1
		end)
		w.link:flush()
		spec.eq(unobserved(), 1, "the unobserved computed reads the snapshot")
		w.score:set(5)
		w:heartbeat()
		spec.eq(unobserved(), 6, "the unobserved computed is not stale after a frame")
		spec.eq(doubled(), 10, "the observed computed recomputed")
		w.score:set(6)
		w:heartbeat()
		spec.eq(unobserved(), 7, "and again after the next frame")
		spec.list(seen, { 0, 10, 12 }, "the effect ran once per frame")
		w:finish()
	end,
}

for name, frame in violations do
	tests[`62 65 a raw frame with a {name} latches failed, keeps every value and the revision, and faults once`] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		w:inject(ann, frame)
		latched(w, ann, name, 1)
		w:finish()
	end
end

return tests
