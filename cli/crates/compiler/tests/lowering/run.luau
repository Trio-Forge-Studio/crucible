-- Runs the compiled lowering fixtures and checks their results against what the TypeScript computes.
-- `lune run run.luau <out>`
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local process = require("@lune/process")

local root = assert(process.args[1], "usage: run.luau <out>")

local nodes = {}
local function node(path: string)
	if nodes[path] == nil then
		nodes[path] = setmetatable({ path = path }, {
			__index = function(self, key)
				if key == "Parent" then
					return node(string.match(self.path, "^(.*)/[^/]+$") or "")
				end
				return nil
			end,
		})
	end
	return nodes[path]
end

local loaded = {}
local TS = {}
local function load(path: string)
	if loaded[path] == nil then
		local file = if fs.isFile(`{root}/{path}.luau`) then `{root}/{path}.luau` else `{root}/{path}/init.luau`
		local environment = { script = node(path), _G = setmetatable({}, {
			__index = function()
				return TS
			end,
		}) }
		loaded[path] = luau.load(fs.readFile(file), { debugName = path, environment = environment })()
	end
	return loaded[path]
end
function TS.import(_caller, base, ...)
	local path = base.path
	for _, name in { ... } do
		path = if path == "" then name else `{path}/{name}`
	end
	return load(path)
end

local failures = 0
local function eq(label: string, actual: any, expected: any)
	if actual ~= expected then
		failures += 1
		print(`FAIL {label}: expected {tostring(expected)}, got {tostring(actual)}`)
	end
end
local function list(label: string, actual: { any }, expected: { any })
	for i = 1, math.max(#expected, #actual) do
		eq(`{label}[{i}]`, actual[i], expected[i])
	end
end

local modules = load("modules")
eq("template and imports", modules.shout({ sides = 3 }), "hi 12 2")
eq("re-export", modules.hello, "hi")
eq("hoisted function", modules.early(), "late")
eq("namespace state", modules.Counter.advance() + modules.Counter.advance(), 3)
eq("init module import", load("nested").quadruple(2), 8)
eq("const enum value", load("nested").high, 5)

local control = load("control")
eq("for with continue", control.sumEvens(7), 12)
eq("while and do-while", control.countDown(3), "3,2,1,0")
for n, expected in { [0] = "zero", [1] = "small", [2] = "small", [3] = "three!", [9] = "big" } do
	eq(`switch {n}`, control.classify(n), expected)
end
eq("fallthrough 1", control.fallthrough(1), "one,two")
eq("fallthrough 2", control.fallthrough(2), "two")
eq("fallthrough default", control.fallthrough(5), "other")
eq("for-of", control.iterate({ "x", "y" }, { a = 1, b = 2 }, { m = true, n = true }), "x y a=1 m")

local classes = load("classes")
list("classes", classes.build(), { "a:leaf+3+1", 2, "leaf", "static", "Leaf" })
eq("abstract has no new", classes.Base.new, nil)
eq("inherited static", classes.Leaf.make, classes.Base.make)

local operators = load("operators")
eq("truthy all", operators.truthy(1, "a", false), "n,s,!flag")
eq("truthy none", operators.truthy(0, "", true), "")
eq("truthy NaN", operators.truthy(0 / 0, "", true), "")
eq("|| on zero", operators.pick(0, 5), 5)
eq("|| on value", operators.pick(2, 5), 2)
eq("&& on zero", operators.both(0, 5), 0)
eq("&& on value", operators.both(2, 5), 5)
eq("?? on nil", operators.coalesce(nil, 4), 4)
eq("?? on zero", operators.coalesce(0, 4), 0)
eq("?? keeps false", operators.maybeFalse(false), false)
eq("?? on nil boolean", operators.maybeFalse(nil), true)
eq("??=", operators.fill({}), 7)
eq("??= keeps", operators.fill({ value = 0 }), 0)
eq("string +", operators.concat("n", 1), "n1!2")
list("increments", operators.increments(), { 0, 2, 2 })
list("bits", operators.bits(6, 3), { 2, 7, 24 })
list(
	"optional chain",
	operators.chain({ inner = {
		value = 3,
		get = function(self)
			return self.value + 1
		end,
	} }),
	{ 3, 4 }
)
list("optional chain nil", operators.chain(nil), {})
list("optional chain inner nil", operators.chain({}), {})
eq("conditional true", operators.ternary(true, 4), 8)
eq("conditional false", operators.ternary(false, 4), 3)

local macros = load("macros")
list("arrays", macros.arrays(), { 3, 4, "1-2-3", "10,21,32", 1, 2, 1, -1, 6, true, true, true })
list("maps", macros.maps(), { 2, false, 1, true, false, 2, true, false })
list("strings", macros.strings("a,b"), { 3, "a,", 2, true, "number" })

local patterns = load("patterns")
list("destructure", patterns.destructure({ 1, 2, nil, 4, 5 }, { a = 9 }), { 1, "one", 1, 30, 2, 9, "default" })
eq("parameters", patterns.parameters({ x = 1 }, { 3, 4 }, 7, 8), 12)
list("assign patterns", patterns.assign({ 5, 6 }), { 5, 6, 1 })

if failures > 0 then
	print(`{failures} failed`)
	process.exit(1)
end
print("lowering fixtures pass")
