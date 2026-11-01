-- Built from wire.md alone, so a codec defect cannot move the reasons the FD cut specs expect.

local widths = {
	["nil"] = 0,
	literal = 0,
	instance = 0,
	bool = 1,
	u8 = 1,
	u16 = 2,
	u32 = 4,
	f32 = 4,
	f64 = 8,
	vector3 = 24,
	color3 = 24,
	cframe = 96,
}

-- The fewest bytes any valid encoding of the node takes, from wire.md's Bytes column.
local function least(node): number
	local kind = node.kind
	if widths[kind] ~= nil then
		return widths[kind]
	elseif kind == "string" or kind == "dynamic" or kind == "array" or kind == "map" then
		return 1
	elseif kind == "variant" then
		return least(node.body)
	elseif kind == "object" then
		local total = 0
		for _, field in node.fields do
			total += least(field[2])
		end
		return total
	elseif kind == "tuple" then
		local total = 0
		for _, element in node.elements do
			total += least(element)
		end
		return total
	elseif kind == "union" then
		local smallest = math.huge
		for _, branch in node.branches do
			smallest = math.min(smallest, least(branch))
		end
		return 1 + smallest
	end
	error(`no wire layout for {kind}`)
end

local function walk(root, packet: buffer)
	local events = {}
	local cursor = 0
	local function span(size)
		assert(cursor + size <= buffer.len(packet), "model ran past the packet")
		cursor += size
		table.insert(events, { stop = cursor })
	end
	local function varuint()
		local value, scale = 0, 1
		while true do
			local byte = buffer.readu8(packet, cursor)
			span(1)
			value += (byte % 128) * scale
			scale *= 128
			if byte < 128 then
				return value
			end
		end
	end
	local function count(elementBytes)
		local entries = varuint()
		assert(entries * elementBytes <= buffer.len(packet) - cursor, "model count fits the full packet")
		table.insert(events, { stop = cursor, entries = entries, elementBytes = elementBytes })
		return entries
	end
	local function text()
		span(varuint())
	end
	local dynamic
	function dynamic()
		local tag = buffer.readu8(packet, cursor)
		span(1)
		if tag == 3 then
			span(8)
		elseif tag == 4 then
			text()
		elseif tag == 5 then
			for _ = 1, count(1) do
				dynamic()
			end
		elseif tag == 6 then
			for _ = 1, count(2) do
				text()
				dynamic()
			end
		elseif tag == 7 or tag == 8 then
			span(24)
		elseif tag == 9 then
			span(96)
		else
			assert(tag <= 2 or tag == 10, "model knows the dynamic tag")
		end
	end
	local function value(node)
		local kind = node.kind
		if widths[kind] ~= nil then
			span(widths[kind])
		elseif kind == "string" then
			text()
		elseif kind == "dynamic" then
			dynamic()
		elseif kind == "variant" then
			value(node.body)
		elseif kind == "object" then
			for _, field in node.fields do
				value(field[2])
			end
		elseif kind == "tuple" then
			for _, element in node.elements do
				value(element)
			end
		elseif kind == "union" then
			value(node.branches[varuint() + 1])
		elseif kind == "array" then
			for _ = 1, count(least(node.element)) do
				value(node.element)
			end
		elseif kind == "map" then
			for _ = 1, count(least(node.key) + least(node.value)) do
				value(node.key)
				value(node.value)
			end
		else
			error(`no wire layout for {kind}`)
		end
	end
	value(root)
	assert(cursor == buffer.len(packet), "model consumed the whole packet")
	return events
end

local function cutReasons(node, packet: buffer): { [number]: string }
	local events = walk(node, packet)
	local reasons = {}
	for length = 0, buffer.len(packet) - 1 do
		for _, event in events do
			if event.entries == nil and event.stop > length then
				reasons[length] = "truncated packet"
				break
			elseif event.entries ~= nil and event.entries * event.elementBytes > length - event.stop then
				reasons[length] = "count exceeds remaining bytes"
				break
			end
		end
		assert(reasons[length] ~= nil, `cut {length} has an expected rejection`)
	end
	return reasons
end

return { cutReasons = cutReasons }
