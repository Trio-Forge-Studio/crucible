local roblox = require("@lune/roblox")
local World = require("./lib/sync-world")
local spec = require("./lib/load")

local function newWorld(descriptor)
	return World.new({
		declare = function(r)
			return r.emit.replicated("Bounds.Value", {}, descriptor(r.n))
		end,
		serve = function(s, d, world)
			s.service("Bounds", function()
				world.write = s.entry.publish(d)
			end)
		end,
	})
end

local function fill(count, value, prefix)
	local out = {}
	for i = 1, count do
		out[if prefix then `{prefix}{i}` else i] = value(i)
	end
	return out
end

local tests = {}
tests["pre-encoded sends retain session guards and queue the supplied packet"] = function()
	local port = require("./lib/network-port")()
	local player = { Name = "Ann" }
	port.players = function()
		return { player }
	end
	local graph = spec.runtime(nil, nil, nil, port)
	local Session = graph.module("internal/network/server-session").ServerSession
	local session = Session.new(port)
	local wire = graph.module("internal/network/envelope")
	local packet = wire.controlPacket(4, {})
	local ok, err = pcall(function()
		session:sendEncoded(player, packet)
	end)
	spec.eq(ok, false, "idle session rejects sends")
	spec.has(err, "networking is not ready", "same lifecycle diagnostic")
	session:open()
	session:loaded()
	spec.eq(session:sendEncoded({}, packet), false, "unknown player has no peer")
	spec.eq(session:sendEncoded(player, packet), true, "queued before commit and READY")
	local peer = session.peers[player]
	spec.eq(peer.queue[1], packet, "the supplied packet is queued without rebuilding")
	spec.eq(peer.queuedBytes, packet.charge, "transport charge retained")
	spec.eq(#port.fired, 0, "nothing fires early")
	session:commit()
	local ready = wire.controlPacket(4, {})
	port.receive(player, ready.bytes, ready.sidecar)
	spec.eq(#port.fired, 1, "READY flushes the queue")
	spec.eq(port.fired[1].packet, packet.bytes, "same buffer fired")
	spec.eq(port.fired[1].sidecar, packet.sidecar, "same sidecar fired")
	session:close()
	spec.eq(session:sendEncoded(player, packet), false, "closed session refuses sends")
end
tests["each sent frame uses one transport encode"] = function()
	spec.rewrite(function(path, source)
		if path == "packages/crucible/out/internal/network/envelope" then
			local rewritten, count = string.gsub(
				source,
				"local function encodeBody%(name, codec, path, value%)",
				function(signature)
					return signature .. '\n if name == "Crucible.State.Frame" then transportEncodeCount += 1 end'
				end
			)
			assert(count == 1, "one transport encoder")
			return "local transportEncodeCount = 0\n"
				.. string.gsub(rewritten, "\nreturn {", function()
					return "\nreturn {\n transportEncodes = function() return transportEncodeCount end,"
				end)
		end
		return source
	end)
	local w = World.new({
		declare = function(r)
			return r.emit.replicated("Encode.Score", 0, r.n.f64)
		end,
		serve = function(s, d, world)
			s.service("Encode", function()
				world.write = s.entry.publish(d)
			end)
		end,
	})
	spec.rewrite(nil)
	local ann, bob = w:join("Ann"), w:join("Bob")
	spec.eq(#w.sent, 2, "two snapshots sent")
	spec.eq(w.wire.envelope.transportEncodes(), #w.sent, "one transport encode per snapshot")
	w.write(42)
	w:heartbeat()
	spec.eq(#w.sent, 4, "two updates sent")
	spec.eq(w.wire.envelope.transportEncodes(), #w.sent, "one transport encode per sent frame")
	spec.eq(ann.d(), 42, "Ann received the update")
	spec.eq(bob.d(), 42, "Bob received the update")
	w:sync():stop()
	w.link:flush()
	spec.eq(#w.sent, 6, "two failed frames sent on stop")
	spec.eq(w.wire.envelope.transportEncodes(), #w.sent, "one transport encode per failed frame too")
	w:finish()
end
for _, count in { 3, 6000, 7000, 11000 } do
	tests[`clearing {count} map keys uses a bounded full replacement`] = function()
		local w = newWorld(function(n)
			return n.map(n.string, n.bool)
		end)
		w.write(fill(count, function()
			return true
		end, "k"))
		local ann = w:join("Ann")
		w.write({})
		w:heartbeat()
		spec.eq(next(ann.d()), nil, "all keys removed")
		spec.eq(ann.api.syncStatus(ann.d), "ready", "still ready")
		spec.eq(w:frames(ann)[2].frame.values[1].mode, "full", "the smaller representation")
		w:finish()
	end
end
tests["replacing 4000 map keys stays within the frame entry budget"] = function()
	local w = newWorld(function(n)
		return n.map(n.string, n.bool)
	end)
	w.write(fill(4000, function()
		return true
	end, "a"))
	local ann = w:join("Ann")
	w.write(fill(4000, function()
		return true
	end, "b"))
	w:heartbeat()
	spec.eq(ann.d().a1, nil, "old keys gone")
	spec.eq(ann.d().b4000, true, "new keys arrive")
	spec.eq(ann.api.syncStatus(ann.d), "ready", "still ready")
	w:finish()
end
tests["shrinking a 7000 item array to one item uses a full replacement"] = function()
	local w = newWorld(function(n)
		return n.array(n.f64)
	end)
	w.write(fill(7000, function(i)
		return i
	end))
	local ann = w:join("Ann")
	w.write({ 9 })
	w:heartbeat()
	spec.list(ann.d(), { 9 }, "one item remains")
	spec.eq(ann.api.syncStatus(ann.d), "ready", "still ready")
	w:finish()
end
for _, case in
	{
		{
			"vector3",
			7000,
			function(i)
				return roblox.Vector3.new(i, i, i)
			end,
		},
		{
			"cframe",
			1980,
			function(i)
				return roblox.CFrame.new(i, 0, 0)
			end,
		},
		{
			"bool",
			11999,
			function()
				return true
			end,
		},
		{
			"f64",
			11999,
			function(i)
				return i
			end,
		},
	}
do
	local kind, count, make = table.unpack(case)
	tests[`a joining client receives an admitted {count} element {kind} array`] = function()
		local w = newWorld(function(n)
			return n.array(n[kind])
		end)
		w.write(fill(count, make))
		local ann = w:join("Ann")
		spec.eq(#ann.d(), count, "all elements arrive")
		spec.eq(ann.d()[count], make(count), "last element intact")
		spec.eq(ann.api.syncStatus(ann.d), "ready", "ready on join")
		w:finish()
	end
end
tests["an equal snapshot retains the client's initial table"] = function()
	local w = newWorld(function(n)
		return n.map(n.string, n.bool)
	end)
	local before
	local ann = w:join("Ann", {
		declare = function(r)
			local d = r.emit.replicated("Bounds.Value", {}, r.n.map(r.n.string, r.n.bool))
			before = d()
			return d
		end,
	})
	spec.eq(ann.d(), before, "no extra atom write for an equal full value")
	w:finish()
end
tests["object key bytes and entries agree with the dynamic codec"] = function()
	local w = newWorld(function(n)
		return n.map(n.string, n.bool)
	end)
	local n = w.server.n
	local plan = w.server.values.compilePlan(n.object({ { "longer", n.bool }, { "x", n.bool } }))
	local value = w.server.values.admit(plan, "cost", { longer = true, x = false })
	local cost = w.server.values.costOf(value)
	local wire = w.wire
	local writer = wire.codec.Writer.new()
	wire.codec.encodeDynamic(writer, value, "value", {})
	local packet = writer:finish(buffer.create(0))
	spec.eq(cost.bytes, buffer.len(packet), "key lengths charge bytes")
	spec.eq(cost.entries, writer.budget.entries, "each key charges one entry")
	w:finish()
end
tests["combined patches fall back when their collection entries exceed one frame"] = function()
	local w = World.new({
		declare = function(r)
			return {
				A = r.emit.replicated(
					"Bounds.A",
					{ pad = "", pad2 = "", pad3 = "", keys = {} },
					r.n.object({
						{ "keys", r.n.map(r.n.string, r.n.bool) },
						{ "pad", r.n.string },
						{ "pad2", r.n.string },
						{ "pad3", r.n.string },
					})
				),
				B = r.emit.replicated("Bounds.B", {}, r.n.map(r.n.string, r.n.bool)),
			}
		end,
		serve = function(s, d, world)
			s.service("Bounds", function()
				world.a = s.entry.publish(d.A)
				world.b = s.entry.publish(d.B)
			end)
		end,
	})
	local pad = string.rep("x", 32000)
	w.a({ pad = pad, pad2 = pad, pad3 = pad, keys = fill(4000, function()
		return true
	end, "k") })
	local ann = w:join("Ann")
	w.a({ pad = pad, pad2 = pad, pad3 = pad, keys = {} })
	w.b(fill(4000, function()
		return false
	end, "b"))
	w:heartbeat()
	spec.eq(next(ann.d.A().keys), nil, "removals arrive")
	spec.eq(ann.d.B().b4000, false, "the other declaration arrives atomically")
	spec.eq(ann.api.syncStatus(ann.d.A), "ready", "full retry fits")
	spec.eq(w:frames(ann)[2].frame.values[1].mode, "full", "combined frame retried as full values")
	w:finish()
end
tests["an encoder error reports its actual cause"] = function()
	spec.rewrite(function(path, source)
		if path == "packages/crucible/out/internal/sync/frame" then
			return (
				string.gsub(
					source,
					"local body = encodeBody",
					'if frame.kind == "snapshot" then error("injected codec bug", 0) end\n local body = encodeBody'
				)
			)
		end
		return source
	end)
	local w = newWorld(function(n)
		return n.map(n.string, n.bool)
	end)
	spec.rewrite(nil)
	w:join("Ann")
	local lines = table.concat(w.server.diagnostics(), "\n")
	assert(string.find(lines, "frame encode failed", 1, true), lines)
	assert(string.find(lines, "injected codec bug", 1, true), lines)
	assert(not string.find(lines, "frame over", 1, true), lines)
	w:finish()
end
tests["patch conversion is frozen and its diff cache distinguishes bases"] = function()
	local w = newWorld(function(n)
		return n.array(n.f64)
	end)
	local p = w.server.graph.module("internal/sync/patches")
	local plan = w.server.values.compilePlan(w.server.n.array(w.server.n.f64))
	local function admit(value)
		return w.server.values.admit(plan, "test", value)
	end
	local a, b, c = admit({ 1, 2, 3 }), admit({ 1, 9, 3 }), admit({ 8, 9, 3 })
	assert(table.isfrozen(p.snapshot(a)), "encoded array is immutable")
	p.beginPatches()
	local work = { visits = 0, limit = 1000 }
	local ab = p.difference(a, b, work)
	local visits = work.visits
	spec.eq(visits, w.server.values.costOf(a).visits + w.server.values.costOf(b).visits, "both trees charged")
	spec.eq(p.difference(a, b, work), ab, "same pair cached")
	spec.eq(work.visits, visits, "cached diff incurs no repeated tree charge")
	local ac = p.difference(a, c, work)
	spec.list(p.apply(a, ac, false), c, "a different target gets a different patch")
	spec.list(p.apply(a, ab, false), b, "forward diff")
	p.beginPatches()
	local limited = { visits = 0, limit = visits - 1 }
	local ok, err = pcall(p.difference, a, b, limited)
	spec.eq(ok, false, "refuse before an over-budget diff")
	assert(w.server.values.isOverBudget(err), "the refusal is a work budget failure")
	w:finish()
end
tests["patches escape marker keys and escape prefixes even when a full value would be smaller"] = function()
	local w = newWorld(function(n)
		return n.map(n.string, n.string)
	end)
	local p = w.server.graph.module("internal/sync/patches")
	local plan = w.server.values.compilePlan(w.server.n.map(w.server.n.string, w.server.n.string))
	local before = w.server.values.admit(plan, "test", {})
	local after = w.server.values.admit(plan, "test", { __none = "__none", __array = "data", ["\0__none"] = "prefix" })
	p.beginPatches()
	local delta = p.difference(before, after, { visits = 0, limit = 1000 })
	local result = p.apply(before, delta, false)
	spec.eq(result.__none, "__none", "marker is ordinary data")
	spec.eq(result.__array, "data", "array marker is ordinary data")
	spec.eq(result["\0__none"], "prefix", "escape prefix survives")
	w:finish()
end
tests["a sparse array update stays compact and applies without holes"] = function()
	local w = newWorld(function(n)
		return n.array(n.string)
	end)
	local unchanged, changed = string.rep("x", 2000), string.rep("y", 2000)
	local before = { unchanged, unchanged, unchanged, unchanged }
	w.write(before)
	local ann = w:join("Ann")
	local after = table.clone(before)
	after[2], after[4] = changed, changed
	w.write(after)
	w:heartbeat()
	local update = w:frames(ann)[2]
	assert(update.bytes < 5000, "two changed strings should not resend all four strings")
	spec.eq(ann.d()[2], changed, "first changed element")
	spec.eq(ann.d()[4], changed, "second changed element")
	spec.eq(ann.d()[3], unchanged, "unchanged element between them")
	spec.eq(#ann.d(), 4, "dense client array")
	w:finish()
end
return tests
