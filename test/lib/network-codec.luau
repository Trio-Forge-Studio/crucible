local roblox = require("@lune/roblox")
local spec = require("./load")

local function realm()
	local runtime = spec.runtime({
		typeof = roblox.typeof,
		Vector3 = roblox.Vector3,
		Color3 = roblox.Color3,
		CFrame = roblox.CFrame,
	})
	return {
		n = runtime.module("internal/network/schema").n,
		codec = runtime.module("internal/network/codec"),
		catalog = runtime.module("internal/network/catalog"),
		limits = runtime.module("internal/network/limits"),
	}
end

local function encode(r, node, value)
	local writer = r.codec.Writer.new()
	r.codec.compile(node).encode(writer, value, "payload")
	local packet, sidecar, slots = writer:finish(buffer.create(0))
	return packet, sidecar, slots, writer.budget
end

local function decode(r, node, packet, sidecar, slots)
	local reader = r.codec.Reader.new(packet, 0, sidecar or {}, slots or 0)
	local value = r.codec.compile(node).decode(reader, "payload")
	assert(reader:finished(), "trailing bytes or unconsumed slots")
	return value, reader.budget
end

local function rejects(fn, reason)
	local ok, failure = pcall(fn)
	spec.eq(ok, false, "rejected")
	if reason ~= nil then
		local text = if type(failure) == "table" then failure.reason else tostring(failure)
		assert(string.find(text, reason, 1, true), `expected "{reason}", got "{text}"`)
	end
	return failure
end

local function hex(packet)
	local out = {}
	for i = 0, buffer.len(packet) - 1 do
		table.insert(out, string.format("%02x", buffer.readu8(packet, i)))
	end
	return table.concat(out, " ")
end

local function bytes(text)
	local result = {}
	for byte in string.gmatch(text, "%x%x") do
		table.insert(result, string.char(tonumber(byte, 16)))
	end
	return buffer.fromstring(table.concat(result))
end

return { realm = realm, encode = encode, decode = decode, rejects = rejects, hex = hex, bytes = bytes, roblox = roblox }
