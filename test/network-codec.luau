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
	for _, fixture in { { r.n.u8, 256 }, { r.n.u16, 65536 }, { r.n.u32, 4294967296 } } do
		for _, bad in { -1, 0.5, fixture[2] } do
			h.rejects(function()
				h.encode(r, fixture[1], bad)
			end, "integer out of range")
		end
		for _, bad in { math.huge, 0 / 0 } do
			h.rejects(function()
				h.encode(r, fixture[1], bad)
			end, "expected finite number")
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
	for _, text in { "ed a0 80", "c0 80", "e0 80 80", "f0 80 80 80", "f4 90 80 80", "e2 82" } do
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
			end, "invalid UTF-8")
			h.rejects(function()
				h.decode(r, fixture[1], buffer.fromstring(fixture[3]))
			end, "invalid UTF-8")
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
	end, "invalid CFrame orthogonality")
	local function scaled(length)
		local frame = buffer.create(96)
		buffer.writef64(frame, 24, length)
		buffer.writef64(frame, 56, 1 / length)
		buffer.writef64(frame, 88, 1)
		return frame
	end
	local edge = h.decode(r, r.n.cframe, scaled(1.0007))
	spec.eq(math.abs(select(4, edge:GetComponents()) - 1.0007) < 1e-6, true, "row length inside tolerance decodes")
	spec.eq(buffer.len(h.encode(r, r.n.cframe, edge)), 96, "row length inside tolerance encodes")
	h.rejects(function()
		h.decode(r, r.n.cframe, scaled(1.0015))
	end, "invalid CFrame rows")
	h.rejects(function()
		h.encode(r, r.n.cframe, h.roblox.CFrame.new(0, 0, 0, 1.0015, 0, 0, 0, 1 / 1.0015, 0, 0, 0, 1))
	end, "invalid CFrame rows")
	local skewed = buffer.create(96)
	buffer.writef64(skewed, 24, 1)
	buffer.writef64(skewed, 56, 1)
	buffer.writef64(skewed, 88, -1)
	h.rejects(function()
		h.decode(r, r.n.cframe, skewed)
	end, "invalid CFrame determinant")
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
	local leave, _, _, leaveBudget = h.encode(r, party, { Kind = "Leave" })
	spec.eq(h.hex(leave), "01", "empty variant body")
	spec.eq(leaveBudget.visits, 2, "one selection and one variant value")
	local value, inviteBudget = h.decode(r, party, h.bytes("00 00 00 00 00 00 c0 5e 40"))
	spec.eq(value.Kind, "Invite", "discriminant restored")
	spec.eq(value.UserId, 123, "body restored")
	spec.eq(inviteBudget.visits, 3, "selection, variant value and its field")
	h.rejects(function()
		h.decode(r, party, h.bytes("02"))
	end, "bad union tag")
	h.rejects(function()
		h.encode(r, party, { Kind = "Bogus" })
	end, "no union branch")
	h.rejects(function()
		r.n.union({ r.n.u8, r.n.f64 })
	end)
	h.rejects(function()
		r.n.union({ r.n.literal("z"), r.n.literal("a") })
	end)
	h.rejects(function()
		r.n.union({ r.n.array(r.n.bool), party.branches[1] })
	end, "ambiguous variants")
	h.rejects(function()
		r.n.union({ party.branches[1], r.n.variant("Type", "Leave", r.n.object({})) })
	end, "ambiguous variants")
	h.rejects(function()
		r.n.variant("Kind", "Leave", r.n.object({ { "Kind", r.n.u8 } }))
	end, "invalid variant")
	h.rejects(function()
		r.n.union({ r.n.variant("Kind", "B", r.n.object({})), r.n.variant("Kind", "A", r.n.object({})) })
	end, "variant order")
	h.rejects(function()
		r.n.union({ r.n.f64, r.n.bool })
	end, "ambiguous union")
	h.rejects(function()
		r.n.union({ r.n.literal(1), r.n.literal(1) })
	end, "literal order")
	h.rejects(function()
		r.n.union({ r.n.bool, r.n["nil"] })
	end, "absent must be first")
end

tests["66 zero-byte arrays charge counts and maps enforce key order"] = function()
	local r = h.realm()
	for _, fixture in
		{
			{ r.n.literal("Tick"), "Tick" },
			{ r.n.object({ { "Kind", r.n.literal("Leave") } }), { Kind = "Leave" } },
		}
	do
		local node = r.n.array(fixture[1])
		local packet, _, _, budget = h.encode(r, node, table.create(100, fixture[2]))
		spec.eq(h.hex(packet), "64", "zero-byte elements are one count byte")
		spec.eq(budget.entries, 100, "entries charged")
		local decoded, decodedBudget = h.decode(r, node, packet)
		spec.eq(#decoded, 100, "count preserved")
		spec.eq(decodedBudget.entries, 100, "decode charges the same entries")
		h.rejects(function()
			h.encode(r, node, table.create(16385, fixture[2]))
		end, "entry budget exhausted")
		h.rejects(function()
			h.decode(r, node, h.bytes("81 80 01"))
		end, "entry budget exhausted")
	end
	for _, fixture in
		{
			{ r.n.string, { z = 2, a = 1 }, "02 01 61 01 01 7a 02", "02 01 7a 02 01 61 01", "02 01 61 01 01 61 02" },
			{
				r.n.string,
				{ [utf8.char(0xe9)] = 2, z = 1 },
				"02 01 7a 01 02 c3 a9 02",
				"02 02 c3 a9 02 01 7a 01",
				"02 01 7a 01 01 7a 02",
			},
			{ r.n.u8, { [3] = 2, [1] = 1 }, "02 01 01 03 02", "02 03 02 01 01", "02 01 01 01 02" },
			{ r.n.u16, { [256] = 2, [2] = 1 }, "02 02 00 01 00 01 02", "02 00 01 02 02 00 01", "02 02 00 01 02 00 02" },
			{
				r.n.u32,
				{ [65536] = 2, [3] = 1 },
				"02 03 00 00 00 01 00 00 01 00 02",
				"02 00 00 01 00 02 03 00 00 00 01",
				"02 03 00 00 00 01 03 00 00 00 02",
			},
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
			end, "map keys not strictly ascending")
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
	local missing = h.rejects(function()
		h.decode(r, r.n.instance({ "Model" }), buffer.create(0), {}, 1)
	end, "missing Instance")
	spec.eq(missing.missing, true, "a required missing Instance is a race, not an attack")
	local wrong = h.rejects(function()
		h.decode(r, r.n.instance({ "Model" }), buffer.create(0), { ["1"] = h.roblox.Instance.new("Part") }, 1)
	end, "expected declared Instance class")
	spec.eq(wrong.missing, nil, "a present Instance of the wrong class is a rejection")
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
	for _, bad in
		{
			{ cycle, "cycle" },
			{ { [2] = 1 }, "expected dense array" },
			{ { [1] = 1, a = 2 }, "mixed dynamic table" },
			{ function() end, "expected table" },
			{ buffer.create(0), "expected table" },
			{ setmetatable({}, {}), "dynamic metatable" },
		}
	do
		h.rejects(function()
			h.encode(r, r.n.dynamic, bad[1])
		end, bad[2])
	end
	for _, bad in
		{
			{ "ff", "bad dynamic tag" },
			{ "05 01 00", "nil nested dynamic value" },
			{ "05 01 0a", "Instance array element" },
			{ "06 01 01 61 00", "nil nested dynamic value" },
			{ "06 02 01 61 01 01 61 02", "record keys not strictly ascending" },
		}
	do
		h.rejects(function()
			h.decode(r, r.n.dynamic, h.bytes(bad[1]))
		end, bad[2])
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
	for _, bad in
		{
			{ "80 00", "non-minimal varuint" },
			{ "80", "truncated packet" },
			{ "ff ff ff ff 10", "varuint overflow" },
			{ "80 80 80 80 80 00", "varuint overflow" },
		}
	do
		h.rejects(function()
			r.codec.Reader.new(h.bytes(bad[1]), 0, {}, 0):varuint()
		end, bad[2])
	end
	h.rejects(function()
		h.decode(r, r.n.bool, h.bytes("02"))
	end, "invalid boolean")
	h.rejects(function()
		h.decode(r, r.n.bool, h.bytes("01 00"))
	end, "trailing bytes")
	for _, sidecar in
		{
			{ ["01"] = h.roblox.Instance.new("Model") },
			{ ["2"] = h.roblox.Instance.new("Model") },
			{ ["1.0"] = h.roblox.Instance.new("Model") },
			{ [" 1"] = h.roblox.Instance.new("Model") },
			{ [1] = h.roblox.Instance.new("Model") },
			{ ["1"] = true },
		}
	do
		h.rejects(function()
			r.codec.Reader.new(buffer.create(0), 0, sidecar, 1)
		end, "invalid sidecar slot")
	end
	h.rejects(function()
		r.codec.Reader.new(buffer.create(0), 0, setmetatable({}, {}), 1)
	end, "invalid sidecar")
	local crowded = {}
	for i = 1, 257 do
		crowded[tostring(i)] = h.roblox.Instance.new("Model")
	end
	h.rejects(function()
		r.codec.Reader.new(buffer.create(0), 0, crowded, 256)
	end, "invalid sidecar slot")
	h.rejects(function()
		h.decode(r, r.n["nil"], buffer.create(0), {}, 1)
	end, "unconsumed slots")
	h.rejects(function()
		r.codec.Reader.new(buffer.create(262145), 0, {}, 0)
	end, "invalid packet")
	h.rejects(function()
		h.encode(r, r.n.string, string.rep("x", 65537))
	end, "string too long")
	local nested = true
	for _ = 1, 33 do
		nested = { nested }
	end
	h.rejects(function()
		h.encode(r, r.n.dynamic, nested)
	end, "depth budget exhausted")
	local budget = r.codec.Budget.new()
	for _ = 1, 65536 do
		budget:visit("payload")
	end
	h.rejects(function()
		budget:visit("payload")
	end, "visit budget exhausted")
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
	end, "visit budget exhausted")
	local tooManyVisits = buffer.create(3 + 16384 * 4)
	buffer.writeu8(tooManyVisits, 0, 128)
	buffer.writeu8(tooManyVisits, 1, 128)
	buffer.writeu8(tooManyVisits, 2, 1)
	h.rejects(function()
		h.decode(r, quartet, tooManyVisits)
	end, "visit budget exhausted")
	local depthBytes = string.rep("\5\1", 33) .. "\1"
	h.rejects(function()
		h.decode(r, n.dynamic, buffer.fromstring(depthBytes))
	end, "depth budget exhausted")
	local slots = n.array(n.instance({ "Model" }))
	local model = h.roblox.Instance.new("Model")
	local slotPacket, sidecar, count = h.encode(r, slots, table.create(256, model))
	spec.eq(count, 256, "slot ceiling accepted")
	spec.eq(#h.decode(r, slots, slotPacket, sidecar, count), 256, "all slots consumed")
	h.rejects(function()
		h.encode(r, slots, table.create(257, model))
	end, "slot budget exhausted")
	h.rejects(function()
		r.codec.Reader.new(slotPacket, 0, sidecar, 257)
	end, "integer out of range")
	local writer = r.codec.Writer.new()
	for _ = 1, 262144 do
		writer:u8(0)
	end
	spec.eq(buffer.len(writer:finish(buffer.create(0))), 262144, "packet ceiling accepted")
	h.rejects(function()
		writer:u8(0)
	end, "packet too large")
	h.rejects(function()
		writer:finish(buffer.create(1))
	end, "packet too large")
	h.rejects(function()
		r.codec.Reader.new(h.bytes("01"), 0, {}, 0):count(8, "payload")
	end, "count exceeds remaining bytes")
	local floating = buffer.create(24)
	buffer.writef64(floating, 0, 1e300)
	for _, node in { n.vector3, n.color3 } do
		h.rejects(function()
			h.decode(r, node, floating)
		end, "expected finite number")
	end
	local tooManyEntries = n.array(n.array(n.literal("Tick")))
	h.rejects(function()
		h.encode(r, tooManyEntries, { table.create(8192, "Tick"), table.create(8192, "Tick") })
	end, "entry budget exhausted")
	h.rejects(function()
		h.decode(r, tooManyEntries, h.bytes("02 80 40 80 40"))
	end, "entry budget exhausted")
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
	for _ = 1, 31 do
		deep = r.n.array(deep)
	end
	r.codec.compile(deep)
	h.rejects(function()
		r.codec.compile(r.n.array(deep))
	end, "schema budget exhausted")
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

tests["58 60 66 69 70 exact width and budget ceilings and every dynamic datatype"] = function()
	local r = h.realm()
	local n = r.n
	for _, fixture in
		{
			{ n.u8, 0, "00" },
			{ n.u16, 0, "00 00" },
			{ n.u16, 65535, "ff ff" },
			{ n.u32, 0, "00 00 00 00" },
			{ n.f32, 2 ^ -149, "01 00 00 00" },
			{ n.f32, (2 - 2 ^ -23) * 2 ^ 127, "ff ff 7f 7f" },
		}
	do
		local packet = h.encode(r, fixture[1], fixture[2])
		spec.eq(h.hex(packet), fixture[3], "width edge bytes")
		spec.eq(h.decode(r, fixture[1], packet), fixture[2], "width edge round trip")
	end
	for _, text in { "00 00 00 00 00 00 f0 7f", "00 00 00 00 00 00 f8 7f" } do
		h.rejects(function()
			h.decode(r, n.f64, h.bytes(text))
		end)
		h.rejects(function()
			h.decode(r, n.dynamic, h.bytes("03 " .. text))
		end)
	end
	for _, fixture in
		{
			{ n.color3, h.roblox.Color3.new(0.25, 0.5, 1.5) },
			{ n.cframe, h.roblox.CFrame.Angles(0.2, 0.3, 0.4) },
		}
	do
		local packet = h.encode(r, fixture[1], fixture[2])
		for length = 0, buffer.len(packet) - 1 do
			local cut = buffer.create(length)
			buffer.copy(cut, 0, packet, 0, length)
			h.rejects(function()
				h.decode(r, fixture[1], cut)
			end)
		end
	end
	local tuple = n.tuple({ n.u8, n.string, n.bool })
	spec.eq(h.hex(h.encode(r, tuple, { 3, "A", false })), "03 01 41 00", "tuple has no count")
	local tupleValue = h.decode(r, tuple, h.bytes("03 01 41 00"))
	spec.eq(tupleValue[1], 3, "tuple first")
	spec.eq(tupleValue[2], "A", "tuple second")
	spec.eq(tupleValue[3], false, "tuple third")
	local model = h.roblox.Instance.new("Model")
	local instancePacket, sidecar, slots = h.encode(r, n.dynamic, model)
	spec.eq(h.hex(instancePacket), "0a", "dynamic Instance tag")
	spec.eq(h.decode(r, n.dynamic, instancePacket, sidecar, slots), model, "dynamic Instance")
	spec.eq(h.decode(r, n.dynamic, instancePacket, {}, slots), nil, "missing whole Instance")
	local recordPacket, _, recordSlots = h.encode(r, n.dynamic, { a = model, b = false })
	local record = h.decode(r, n.dynamic, recordPacket, {}, recordSlots)
	spec.eq(record.a, nil, "missing Instance key absent")
	spec.eq(record.b, false, "later record value intact")
	h.rejects(function()
		h.encode(r, n.dynamic, { model })
	end)
	for _, value in
		{
			h.roblox.Vector3.new(1, 2, 3),
			h.roblox.Color3.new(0.25, 0.5, 1.5),
			h.roblox.CFrame.Angles(0.2, 0.3, 0.4),
			1.5,
		}
	do
		local packet = h.encode(r, n.dynamic, value)
		spec.eq(
			h.hex(h.encode(r, n.dynamic, h.decode(r, n.dynamic, packet))),
			h.hex(packet),
			"dynamic datatype round trip"
		)
	end
	for _, value in { coroutine.create(function() end), h.roblox.Vector2.new(1, 2), { [true] = 1 } } do
		h.rejects(function()
			h.encode(r, n.dynamic, value)
		end)
	end
	local atDepth = buffer.fromstring(string.rep("\5\1", 32) .. "\1")
	spec.eq(h.hex(h.encode(r, n.dynamic, h.decode(r, n.dynamic, atDepth))), h.hex(atDepth), "depth ceiling admitted")
	local maximumString = string.rep("x", 65536)
	spec.eq(h.decode(r, n.string, h.encode(r, n.string, maximumString)), maximumString, "string ceiling admitted")
	local fields = {}
	for i = 1, 4095 do
		table.insert(fields, { string.format("F%04d", i), n.bool })
	end
	r.codec.compile(n.object(fields))
	local branches = {}
	for i = 0, 255 do
		table.insert(branches, n.literal(i))
	end
	local union = n.union(branches)
	spec.eq(h.hex(h.encode(r, union, 255)), "ff 01", "256 branches use varuint tags")
	spec.eq(h.decode(r, union, h.bytes("ff 01")), 255, "maximum tag")
	table.insert(branches, n.literal(256))
	h.rejects(function()
		n.union(branches)
	end)
end

tests["61 66 static tables admit metatables while 69 dynamic tables reject them"] = function()
	local r = h.realm()
	local object = r.n.object({ { "a", r.n.u8 } })
	spec.eq(h.hex(h.encode(r, object, setmetatable({ a = 3, extra = 4 }, {}))), "03", "static object")
	spec.eq(h.hex(h.encode(r, r.n.array(r.n.u8), setmetatable({ 3, 4 }, {}))), "02 03 04", "static array")
	spec.eq(h.hex(h.encode(r, r.n.map(r.n.string, r.n.u8), setmetatable({ a = 3 }, {}))), "01 01 61 03", "static map")
	h.rejects(function()
		h.encode(r, r.n.dynamic, setmetatable({ a = 3 }, {}))
	end)
end

return tests
