-- Runs the compiler's Luau for generated programs with roblox-ts's RuntimeLib.
-- Usage: lune run lune.luau <out> <roblox-ts> <name>... Each program is <out>/<name>.luau, and <roblox-ts> holds the
-- vendored RuntimeLib.lua and Promise.lua. Prints `#<name>` and then one line per trace call, as node.mjs does.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local process = require("@lune/process")
local stdio = require("@lune/stdio")
local task = require("@lune/task")

local out = assert(process.args[1], "usage: lune.luau <out> <roblox-ts> <name>...")
local vendored = assert(process.args[2], "usage: lune.luau <out> <roblox-ts> <name>...")

local runService = {
	Heartbeat = {
		Connect = function()
			return { Disconnect = function() end }
		end,
	},
	IsStudio = function()
		return false
	end,
	IsRunning = function()
		return false
	end,
}
local game = {
	GetService = function()
		return runService
	end,
}
local promiseNode = {}
local promise
local function requireNode(node)
	assert(node == promiseNode, "RuntimeLib requires only Promise")
	if promise == nil then
		promise = luau.load(fs.readFile(`{vendored}/Promise.lua`), {
			debugName = "Promise",
			environment = { game = game, task = task },
		})()
	end
	return promise
end
local TS = luau.load(fs.readFile(`{vendored}/RuntimeLib.lua`), {
	debugName = "RuntimeLib",
	environment = { script = { Parent = { Promise = promiseNode } }, game = game, require = requireNode },
})()
local G = setmetatable({}, {
	__index = function()
		return TS
	end,
})

local function number(n: number): string
	if n ~= n then
		return "NaN"
	elseif n == math.huge then
		return "Infinity"
	elseif n == -math.huge then
		return "-Infinity"
	elseif n == 0 then
		return if 1 / n < 0 then "-0" else "0"
	elseif n == math.floor(n) and math.abs(n) < 2 ^ 53 then
		return string.format("%.0f", n)
	end
	-- The fewest digits that read back as `n`, as JavaScript's `String` picks them: the correctly rounded ones.
	for precision = 0, 16 do
		local text = string.format(`%.{precision}e`, n)
		if tonumber(text) == n or precision == 16 then
			local mantissa, exponent = string.match(text, "^(.*)e(.*)$")
			return `{mantissa}e{tonumber(exponent)}`
		end
	end
	error("unreachable")
end

local function quote(s: string): string
	local parts = { '"' }
	for i = 1, #s do
		local code = string.byte(s, i)
		if code >= 0x20 and code < 0x7f and code ~= 0x22 and code ~= 0x5c then
			table.insert(parts, string.char(code))
		else
			table.insert(parts, string.format("\\x%02x", code))
		end
	end
	table.insert(parts, '"')
	return table.concat(parts)
end

local function dynamic(value: any): string
	local kind = type(value)
	if kind == "number" then
		return number(value)
	elseif kind == "string" then
		return quote(value)
	elseif kind == "boolean" then
		return tostring(value)
	elseif kind == "nil" then
		return "undefined"
	end
	return "<object>"
end

type Shape = { kind: string, inner: Shape?, key: Shape?, value: Shape?, fields: { { any } }? }

local function parse(shape: string): Shape
	local at = 1
	local function peek()
		return string.sub(shape, at, at)
	end
	local function nextChar()
		local c = peek()
		at += 1
		return c
	end
	local function expect(c)
		if nextChar() ~= c then
			error(`bad shape {shape} at {at}`)
		end
	end
	local one
	function one(): Shape
		local c = nextChar()
		if c == "n" or c == "s" or c == "b" or c == "x" then
			return { kind = c }
		elseif c == "?" then
			return { kind = "?", inner = one() }
		elseif c == "[" then
			local inner = one()
			expect("]")
			return { kind = "[", inner = inner }
		elseif c == "S" then
			expect("(")
			local inner = one()
			expect(")")
			return { kind = "S", inner = inner }
		elseif c == "M" then
			expect("(")
			local key = one()
			expect(",")
			local value = one()
			expect(")")
			return { kind = "M", key = key, value = value }
		elseif c == "{" then
			local fields = {}
			while peek() ~= "}" do
				local name = ""
				while peek() ~= ":" do
					name ..= nextChar()
				end
				expect(":")
				table.insert(fields, { name, one() })
				if peek() == "," then
					at += 1
				end
			end
			expect("}")
			return { kind = "{", fields = fields }
		end
		error(`bad shape {shape} at {at}`)
	end
	local parsed = one()
	if at ~= #shape + 1 then
		error(`bad shape {shape}`)
	end
	return parsed
end

local function sorted(items: { string }): string
	table.sort(items)
	return table.concat(items, ",")
end

local show
function show(value: any, shape: Shape): string
	local kind = shape.kind
	if kind == "n" then
		return if type(value) == "number" then number(value) else `<not a number: {dynamic(value)}>`
	elseif kind == "s" then
		return if type(value) == "string" then quote(value) else `<not a string: {dynamic(value)}>`
	elseif kind == "b" then
		return if type(value) == "boolean" then tostring(value) else `<not a boolean: {dynamic(value)}>`
	elseif kind == "x" then
		return dynamic(value)
	elseif kind == "?" then
		return if value == nil then "undefined" else show(value, shape.inner :: Shape)
	end
	if type(value) ~= "table" then
		local what = if kind == "["
			then "an array"
			elseif kind == "{" then "an object"
			elseif kind == "M" then "a map"
			else "a set"
		return `<not {what}: {dynamic(value)}>`
	end
	if kind == "[" then
		local items = {}
		local length = #value
		for i = 1, length do
			table.insert(items, show(value[i], shape.inner :: Shape))
		end
		for key in value do
			if type(key) ~= "number" or key < 1 or key > length or key ~= math.floor(key) then
				table.insert(items, `<key {dynamic(key)}>`)
			end
		end
		return `[{table.concat(items, ",")}]`
	elseif kind == "{" then
		local items = {}
		for _, field in shape.fields :: { { any } } do
			table.insert(items, `{field[1]}={show(value[field[1]], field[2])}`)
		end
		return `\{{table.concat(items, ",")}}`
	elseif kind == "M" then
		local items = {}
		for k, v in value do
			table.insert(items, `{show(k, shape.key :: Shape)}=>{show(v, shape.value :: Shape)}`)
		end
		return `M\{{sorted(items)}}`
	else
		local items = {}
		for k, v in value do
			table.insert(
				items,
				if v == true then show(k, shape.inner :: Shape) else `<member {dynamic(k)}={dynamic(v)}>`
			)
		end
		return `S\{{sorted(items)}}`
	end
end

local shapes = {}
local output = {}
for i = 3, #process.args do
	local name = process.args[i]
	local lines = {}
	local function trace(value, shape)
		local parsed = shapes[shape]
		if parsed == nil then
			parsed = parse(shape)
			shapes[shape] = parsed
		end
		table.insert(lines, show(value, parsed))
	end
	local environment = { script = { Name = name }, _G = G, trace = trace }
	local loaded, chunk =
		pcall(luau.load, fs.readFile(`{out}/{name}.luau`), { debugName = name, environment = environment })
	if not loaded then
		table.insert(lines, `!syntax {quote(string.match(tostring(chunk), "^[^\n]*") or "")}`)
	else
		local ok, problem = pcall(chunk)
		if not ok then
			table.insert(lines, `!throw {dynamic(problem)}`)
		end
	end
	table.insert(output, `#{name}`)
	for _, line in lines do
		table.insert(output, line)
	end
end
table.insert(output, "")
stdio.write(table.concat(output, "\n"))
