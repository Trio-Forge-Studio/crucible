local spec = require("./lib/load")
local h = require("./lib/network-codec")

local tests = {}

tests["58 finite numbers and widths use little endian without wrapping"] = function()
	local r = h.realm()
	for _, fixture in
		{
			{ r.n.u8, 255, "ff" },
			{ r.n.u16, 513, "01 02" },
			{ r.n.u32, 4294967295, "ff ff ff ff" },
			{ r.n.f32, 1.5, "00 00 c0 3f" },
			{ r.n.f64, -1, "00 00 00 00 00 00 f0 bf" },
		}
	do
		local packet = h.encode(r, fixture[1], fixture[2])
		spec.eq(h.hex(packet), fixture[3], "golden bytes")
		spec.eq(h.decode(r, fixture[1], packet), fixture[2], "round trip")
		for i = 0, buffer.len(packet) - 1 do
			local cut = buffer.create(i)
			buffer.copy(cut, 0, packet, 0, i)
			h.rejects(function()
				h.decode(r, fixture[1], cut)
			end)
		end
	end
	for _, node in { r.n.u8, r.n.u16, r.n.u32 } do
		for _, bad in { -1, 0.5, math.huge, 0 / 0, 4294967296 } do
			h.rejects(function()
				h.encode(r, node, bad)
			end)
		end
	end
	for _, node in { r.n.f32, r.n.f64 } do
		for _, bad in { math.huge, -math.huge, 0 / 0 } do
			h.rejects(function()
				h.encode(r, node, bad)
			end)
		end
	end
	h.rejects(function()
		h.encode(r, r.n.f32, 1e40)
	end)
	for _, text in { "00 00 80 7f", "00 00 c0 7f" } do
		h.rejects(function()
			h.decode(r, r.n.f32, h.bytes(text))
		end)
	end
end

tests["59 UTF-8 rejects malformed strings in every string position"] = function()
	local r = h.realm()
	for _, text in { "ed a0 80", "c0 80", "f4 90 80 80", "e2 82" } do
		local invalid = buffer.tostring(h.bytes(text))
		for _, fixture in
			{
				{ r.n.string, invalid, string.char(#invalid) .. invalid },
				{
					r.n.map(r.n.string, r.n.bool),
					{ [invalid] = true },
					"\1" .. string.char(#invalid) .. invalid .. "\1",
				},
				{ r.n.dynamic, invalid, "\4" .. string.char(#invalid) .. invalid },
				{ r.n.dynamic, { [invalid] = true }, "\6\1" .. string.char(#invalid) .. invalid .. "\2" },
			}
		do
			h.rejects(function()
				h.encode(r, fixture[1], fixture[2])
			end)
			h.rejects(function()
				h.decode(r, fixture[1], buffer.fromstring(fixture[3]))
			end)
		end
	end
	local valid = "hello " .. utf8.char(0x10ffff)
	spec.eq(h.decode(r, r.n.string, h.encode(r, r.n.string, valid)), valid, "valid UTF-8")
end

tests["60 Vector3 Color3 and CFrame preserve all finite components"] = function()
	local r = h.realm()
	for _, fixture in
		{
			{ r.n.vector3, h.roblox.Vector3.new(1, -2, 3) },
			{ r.n.color3, h.roblox.Color3.new(0.25, 0.5, 1.5) },
			{ r.n.cframe, h.roblox.CFrame.new(1, 2, 3) * h.roblox.CFrame.Angles(0.2, 0.3, 0.4) },
			{ r.n.cframe, h.roblox.CFrame.Angles(0, math.pi - 0.0001, 0) },
			{ r.n.cframe, h.roblox.CFrame.Angles(0.000001, 0, 0) },
		}
	do
		local packet = h.encode(r, fixture[1], fixture[2])
		local value = h.decode(r, fixture[1], packet)
		spec.eq(h.hex(h.encode(r, fixture[1], value)), h.hex(packet), "component bytes")
	end
	local bad = buffer.create(96)
	h.rejects(function()
		h.decode(r, r.n.cframe, bad)
	end)
	for i = 0, 2 do
		buffer.writef64(bad, 24 + (i * 3 + i) * 8, 1)
	end
	buffer.writef64(bad, 88, -1)
	h.rejects(function()
		h.decode(r, r.n.cframe, bad)
	end)
	buffer.writef64(bad, 24, 2)
	buffer.writef64(bad, 56, 0.5)
	buffer.writef64(bad, 88, 1)
	h.rejects(function()
		h.decode(r, r.n.cframe, bad)
	end)
	buffer.writef64(bad, 24, 1)
	buffer.writef64(bad, 48, 0.0015)
	buffer.writef64(bad, 56, math.sqrt(1 - 0.0015 ^ 2))
	h.rejects(function()
		h.decode(r, r.n.cframe, bad)
	end)
end

tests["61 objects ignore undeclared fields and reject empty and numeric shapes"] = function()
	local r = h.realm()
	local node = r.n.object({ { "a", r.n.u8 }, { "z", r.n.u16 } })
	local packet = h.encode(r, node, { a = 3, z = 513, extra = "ignored" })
	spec.eq(h.hex(packet), "03 01 02", "field order")
	local value = h.decode(r, node, packet)
	spec.eq(value.extra, nil, "extra absent")
	h.rejects(function()
		r.codec.compile(r.n.object({}))
	end)
	h.rejects(function()
		r.n.object({ { "1", r.n.u8 } })
	end)
	h.rejects(function()
		r.n.object({ { "z", r.n.u8 }, { "a", r.n.u8 } })
	end)
end

tests["63 64 65 canonical unions select literals kinds and variants"] = function()
	local r = h.realm()
	local party = r.n.union({
		r.n.variant("Kind", "Invite", r.n.object({ { "UserId", r.n.f64 } })),
		r.n.variant("Kind", "Leave", r.n.object({})),
	})
	spec.eq(h.hex(h.encode(r, party, { Kind = "Leave" })), "01", "empty variant body")
	local value = h.decode(r, party, h.bytes("00 00 00 00 00 00 c0 5e 40"))
	spec.eq(value.Kind, "Invite", "discriminant restored")
	spec.eq(value.UserId, 123, "body restored")
	h.rejects(function()
		h.decode(r, party, h.bytes("02"))
	end)
	h.rejects(function()
		h.encode(r, party, { Kind = "Bogus" })
	end)
	h.rejects(function()
		r.n.union({ r.n.u8, r.n.f64 })
	end)
	h.rejects(function()
		r.n.union({ r.n.literal("z"), r.n.literal("a") })
	end)
	h.rejects(function()
		r.n.union({ r.n.array(r.n.bool), party.branches[1] })
	end)
end

tests["66 zero-byte arrays charge counts and maps enforce key order"] = function()
	local r = h.realm()
	for _, element in
		{
			r.n.literal("Tick"),
			r.n.union({ r.n.variant("Kind", "Leave", r.n.object({})) }),
		}
	do
		local node = r.n.array(element)
		local values = table.create(100, if element.kind == "literal" then "Tick" else { Kind = "Leave" })
		local packet, _, _, budget = h.encode(r, node, values)
		spec.eq(budget.entries, 100, "entries charged")
		spec.eq(#h.decode(r, node, packet), 100, "count preserved")
		if element.kind == "literal" then
			spec.eq(h.hex(packet), "64", "zero-byte literal")
		end
		h.rejects(function()
			h.encode(r, node, table.create(16385, values[1]))
		end)
		h.rejects(function()
			h.decode(r, node, h.bytes("81 80 01"))
		end)
	end
	for _, fixture in
		{
			{ r.n.string, { z = 2, a = 1 }, "02 01 61 01 01 7a 02", "02 01 7a 02 01 61 01", "02 01 61 01 01 61 02" },
			{ r.n.u8, { [3] = 2, [1] = 1 }, "02 01 01 03 02", "02 03 02 01 01", "02 01 01 01 02" },
			{
				r.n.union({ r.n.literal("a"), r.n.literal("b") }),
				{ b = 2, a = 1 },
				"02 00 01 01 02",
				"02 01 02 00 01",
				"02 00 01 00 02",
			},
		}
	do
		local node = r.n.map(fixture[1], r.n.u8)
		spec.eq(h.hex(h.encode(r, node, fixture[2])), fixture[3], "sorted bytes")
		spec.eq(h.hex(h.encode(r, node, h.decode(r, node, h.bytes(fixture[3])))), fixture[3], "sorted round trip")
		for i = 4, 5 do
			h.rejects(function()
				h.decode(r, node, h.bytes(fixture[i]))
			end)
		end
	end
	h.rejects(function()
		r.n.array(r.n["nil"])
	end)
	h.rejects(function()
		r.n.map(r.n.f32, r.n.bool)
	end)
	h.rejects(function()
		h.encode(r, r.n.array(r.n.bool), { [2] = true })
	end)
end

tests["67 slots preserve occurrences and missing optional Instances"] = function()
	local r = h.realm()
	local model = h.roblox.Instance.new("Model")
	local other = h.roblox.Instance.new("Model")
	local optional = r.n.union({ r.n["nil"], r.n.instance({ "Model" }) })
	local node = r.n.object({ { "a", optional }, { "b", optional } })
	local packet, sidecar, slots = h.encode(r, node, { a = model, b = other })
	spec.eq(slots, 2, "one slot per occurrence")
	spec.eq(sidecar["1"], model, "first slot")
	spec.eq(sidecar["2"], other, "second slot")
	local value = h.decode(r, node, packet, { ["2"] = other }, slots)
	spec.eq(value.a, nil, "missing optional")
	spec.eq(value.b, other, "later slot intact")
	h.rejects(function()
		h.decode(r, r.n.instance({ "Model" }), buffer.create(0), {}, 1)
	end)
	h.rejects(function()
		h.decode(r, r.n.instance({ "Model" }), buffer.create(0), { ["1"] = h.roblox.Instance.new("Part") }, 1)
	end)
end

tests["69 dynamic tags patches copies and hostile tables"] = function()
	local r = h.realm()
	local shared = { __none = "__none" }
	local value = h.decode(r, r.n.dynamic, h.encode(r, r.n.dynamic, { a = shared, b = shared }))
	spec.eq(value.a.__none, "__none", "patch intact")
	spec.eq(value.a == value.b, false, "copied twice")
	for _, fixture in { { false, "01" }, { true, "02" }, { {}, "06 00" }, { { true, false }, "05 02 02 01" } } do
		spec.eq(h.hex(h.encode(r, r.n.dynamic, fixture[1])), fixture[2], "dynamic golden")
	end
	local cycle = {}
	cycle.self = cycle
	local failure = h.rejects(function()
		h.encode(r, r.n.dynamic, cycle)
	end)
	spec.eq(failure.reason, "cycle", "cycle detected before a budget is exhausted")
	for _, bad in { cycle, { [2] = 1 }, { [1] = 1, a = 2 }, function() end, buffer.create(0), setmetatable({}, {}) } do
		h.rejects(function()
			h.encode(r, r.n.dynamic, bad)
		end)
	end
	for _, bad in { "ff", "05 01 00", "05 01 0a", "06 01 01 61 00", "06 02 01 61 01 01 61 02" } do
		h.rejects(function()
			h.decode(r, r.n.dynamic, h.bytes(bad))
		end)
	end
end

tests["70 71 budgets varuint truncation tags counts and sidecars reject before dispatch"] = function()
	local r = h.realm()
	for _, value in { 0, 127, 128, 16384, 4294967295 } do
		local writer = r.codec.Writer.new()
		writer:varuint(value)
		local packet = writer:finish(buffer.create(0))
		local reader = r.codec.Reader.new(packet, 0, {}, 0)
		spec.eq(reader:varuint(), value, "minimal varuint")
	end
	for _, bad in { "80 00", "80", "ff ff ff ff 10", "80 80 80 80 80 00" } do
		h.rejects(function()
			r.codec.Reader.new(h.bytes(bad), 0, {}, 0):varuint()
		end)
	end
	for _, bad in { "02", "01 00" } do
		h.rejects(function()
			h.decode(r, r.n.bool, h.bytes(bad))
		end)
	end
	for _, sidecar in
		{
			{ ["01"] = h.roblox.Instance.new("Model") },
			{ ["2"] = h.roblox.Instance.new("Model") },
			{ ["1"] = true },
			setmetatable({}, {}),
		}
	do
		h.rejects(function()
			r.codec.Reader.new(buffer.create(0), 0, sidecar, 1)
		end)
	end
	h.rejects(function()
		h.decode(r, r.n["nil"], buffer.create(0), {}, 1)
	end)
	h.rejects(function()
		r.codec.Reader.new(buffer.create(262145), 0, {}, 0)
	end)
	h.rejects(function()
		h.encode(r, r.n.string, string.rep("x", 65537))
	end)
	local nested = true
	for _ = 1, 33 do
		nested = { nested }
	end
	h.rejects(function()
		h.encode(r, r.n.dynamic, nested)
	end)
	local budget = r.codec.Budget.new()
	for _ = 1, 65536 do
		budget:visit("payload")
	end
	h.rejects(function()
		budget:visit("payload")
	end)
end

tests["70 shared budgets enforce exact visits depth entries bytes and slots"] = function()
	local r = h.realm()
	local n = r.n
	local literal = n.array(n.object({ { "Kind", n.literal("Leave") } }))
	local packet, _, _, budget = h.encode(r, literal, table.create(100, { Kind = "Leave" }))
	spec.eq(h.hex(packet), "64", "zero-byte object elements")
	spec.eq(budget.entries, 100, "entries")
	spec.eq(budget.visits, 201, "one visit per array object and literal value")
	local _, decodedBudget = h.decode(r, literal, packet)
	spec.eq(decodedBudget.visits, budget.visits, "identical budget")
	local union = n.union({ n["nil"], n.bool })
	local _, _, _, unionBudget = h.encode(r, union, false)
	spec.eq(unionBudget.visits, 2, "one value and one selection")
	local _, _, _, dynamicBudget = h.encode(r, n.dynamic, false)
	spec.eq(dynamicBudget.visits, 1, "one dynamic value")
	local quartet = n.array(n.tuple({ n.bool, n.bool, n.bool, n.bool }))
	h.rejects(function()
		h.encode(r, quartet, table.create(16384, { true, false, true, false }))
	end)
	local tooManyVisits = buffer.create(3 + 16384 * 4)
	buffer.writeu8(tooManyVisits, 0, 128)
	buffer.writeu8(tooManyVisits, 1, 128)
	buffer.writeu8(tooManyVisits, 2, 1)
	h.rejects(function()
		h.decode(r, quartet, tooManyVisits)
	end)
	local depthBytes = string.rep("\5\1", 33) .. "\1"
	h.rejects(function()
		h.decode(r, n.dynamic, buffer.fromstring(depthBytes))
	end)
	local slots = n.array(n.instance({ "Model" }))
	local model = h.roblox.Instance.new("Model")
	local slotPacket, sidecar, count = h.encode(r, slots, table.create(256, model))
	spec.eq(count, 256, "slot ceiling accepted")
	spec.eq(#h.decode(r, slots, slotPacket, sidecar, count), 256, "all slots consumed")
	h.rejects(function()
		h.encode(r, slots, table.create(257, model))
	end)
	h.rejects(function()
		r.codec.Reader.new(slotPacket, 0, sidecar, 257)
	end)
	local writer = r.codec.Writer.new()
	for _ = 1, 262144 do
		writer:u8(0)
	end
	spec.eq(buffer.len(writer:finish(buffer.create(0))), 262144, "packet ceiling accepted")
	h.rejects(function()
		writer:u8(0)
	end)
	h.rejects(function()
		writer:finish(buffer.create(1))
	end)
	h.rejects(function()
		r.codec.Reader.new(h.bytes("01"), 0, {}, 0):count(8, "payload")
	end)
	local floating = buffer.create(24)
	buffer.writef64(floating, 0, 1e300)
	for _, node in { n.vector3, n.color3 } do
		h.rejects(function()
			h.decode(r, node, floating)
		end)
	end
	local tooManyEntries = n.array(n.array(n.literal("Tick")))
	h.rejects(function()
		h.encode(r, tooManyEntries, { table.create(8192, "Tick"), table.create(8192, "Tick") })
	end)
	h.rejects(function()
		h.decode(r, tooManyEntries, h.bytes("02 80 40 80 40"))
	end)
end

tests["57 62 descriptors freeze copy and reject malformed or oversized graphs"] = function()
	local r = h.realm()
	local fields = { { "a", r.n.u8 } }
	local node = r.n.object(fields)
	fields[1][1] = "z"
	spec.eq(node.fields[1][1], "a", "constructor copies fields")
	spec.eq(table.isfrozen(node), true, "frozen node")
	spec.eq(table.isfrozen(node.fields), true, "frozen collection")
	spec.eq(table.isfrozen(node.fields[1]), true, "frozen field")
	h.rejects(function()
		r.n.array({ kind = "Bogus" })
	end)
	h.rejects(function()
		r.n.array(table.freeze({ kind = "object" }))
	end)
	h.rejects(function()
		r.codec.compile({ kind = "Bogus" })
	end)
	h.rejects(function()
		r.n.tuple({ r.n["nil"] })
	end)
	h.rejects(function()
		r.n.instance({})
	end)
	h.rejects(function()
		r.n.instance({ "Model", "BasePart" })
	end)
	h.rejects(function()
		r.n.literal(0 / 0)
	end)
	local many = {}
	for i = 1, 4096 do
		table.insert(many, { string.format("F%04d", i), r.n.bool })
	end
	h.rejects(function()
		r.codec.compile(r.n.object(many))
	end)
	local deep = r.n.bool
	for _ = 1, 32 do
		deep = r.n.array(deep)
	end
	h.rejects(function()
		r.codec.compile(deep)
	end)
	h.rejects(function()
		r.codec.compile({ kind = "union", branches = { r.n.u8, r.n.f64 } })
	end)
end

tests["73 codec failures carry paths and reasons without logging payload content"] = function()
	local logged = 0
	local graph = spec.runtime({
		game = setmetatable({}, {
			__index = function()
				error("shared codec reached an engine service")
			end,
		}),
		warn = function()
			logged += 1
		end,
		print = function()
			logged += 1
		end,
	})
	local codec = graph.module("internal/network/codec")
	local n = graph.module("internal/network/schema").n
	local failure = h.rejects(function()
		codec
			.compile(n.object({ { "secret", n.f64 } }))
			.encode(codec.Writer.new(), { secret = "private payload" }, "payload")
	end)
	spec.eq(failure.codecFailure, true, "structured failure")
	spec.eq(failure.path, "payload.secret", "property path")
	spec.eq(string.find(failure.reason, "private payload", 1, true), nil, "content omitted")
	spec.eq(logged, 0, "no logging")
end

return tests
