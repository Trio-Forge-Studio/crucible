local fs = require("@lune/fs")
local luau = require("@lune/luau")
local serde = require("@lune/serde")

local n = {}
for _, kind in { "nil", "bool", "f64", "f32", "u8", "u16", "u32", "string", "vector3", "color3", "cframe", "dynamic" } do
	n[kind] = { kind = kind }
end
for kind, keys in
	{
		literal = { "value" },
		instance = { "classes" },
		object = { "fields" },
		array = { "element" },
		tuple = { "elements" },
		map = { "key", "value" },
		union = { "branches" },
		variant = { "key", "value", "body" },
	}
do
	n[kind] = function(...)
		local values = { ... }
		local result = { kind = kind }
		for i, key in keys do
			result[key] = values[i]
		end
		return result
	end
end
local declarations = {}
local emit = { n = n }
function emit.toServer(name, payload)
	declarations[name] = { payload }
end
emit.toClient = emit.toServer
function emit.askServer(name, request, response)
	declarations[name] = { request, response }
end
local node = {}
setmetatable(node, {
	__index = function()
		return node
	end,
	__call = function()
		return node
	end,
})
local runtime = {
	import = function()
		return emit
	end,
}
local environment = setmetatable({
	game = node,
	script = node,
	require = function()
		return runtime
	end,
}, { __index = getfenv() })
luau.load(fs.readFile("out/shared/messages.luau"), { environment = environment })()
local function equal(actual, expected, path)
	assert(type(actual) == type(expected), path)
	if type(actual) ~= "table" then
		assert(actual == expected, path)
		return
	end
	for key, value in expected do
		equal(actual[key], value, `{path}.{key}`)
	end
	for key in actual do
		assert(expected[key] ~= nil, `{path}.{key} is unexpected`)
	end
end
local expected = serde.decode("json", fs.readFile("expected.json"))
local count = 0
for name, descriptors in expected do
	equal(declarations[name], descriptors, name)
	count += 1
end
assert(count == 52)
print("52 emitted descriptor declarations matched")
