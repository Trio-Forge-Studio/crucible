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
local G = setmetatable({}, {
	__index = function()
		return TS
	end,
})
local function load(path: string)
	if loaded[path] == nil then
		local file = if fs.isFile(`{root}/{path}.luau`) then `{root}/{path}.luau` else `{root}/{path}/init.luau`
		local environment = { script = node(path), _G = G }
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
eq("for-of over a LuaTuple", control.spanOf(), 7)
eq("for-of", control.iterate({ "x", "y" }, { a = 1, b = 2 }, { m = true, n = true }), "x y a=1 m")
eq("for-of over a string", control.letters("abc"), "a-b-c")
eq("closures capture each pass", control.captured(), "0,1,2")
eq("closures see their own pass", control.capturedAndChanged(), "1,3,5")
eq("continue keeps a changed variable", control.capturedWithContinue(), "0,4,5")
eq("increment reads the head's scope", control.shadowedIncrement(), 30)
eq("do-while condition reads the outer scope", control.shadowedCondition(), 3)
eq("whole LuaTuple iterator steps", control.wholeSteps({ a = 1 }, { { 1, 2 }, { 3, 4 } }), "1 a=1 a:1 1:3 2:7")

local exceptions = load("exceptions")
eq("a thrown string reaches catch unchanged", exceptions.caught(0), "plain")
eq("a thrown table reaches catch as itself", exceptions.isMarker(exceptions.caught(1)), true)
eq("a thrown number reaches catch", exceptions.caught(2), 42)
eq("an engine error reaches catch", exceptions.caught(3), "raised")
eq("return inside try", exceptions.caught(4), "none")
eq(
	"an empty catch swallows",
	exceptions.swallow(function()
		error("boom")
	end),
	false
)
eq("try without a throw", exceptions.swallow(function() end), true)
eq("finally after falling through", exceptions.finallyPaths("normal"), "fell|try,end,finally")
eq("finally after return", exceptions.finallyPaths("return"), "returned|try,finally")
eq("finally after catch", exceptions.finallyPaths("throw"), "caught|try,catch thrown,finally")
eq("finally on break and continue", exceptions.loopExits(), "body 0,finally 0,finally 1,body 2,finally 2,finally 3")
eq(
	"break and continue in catch",
	exceptions.catchExits({ 1, -1, -3, -2, 5 }),
	"ok 1,finally 1,finally -1,caught -3,finally -3,finally -2"
)
eq("break from a switch inside try", exceptions.switchBreak(1), "one,finally")
eq("switch default after try", exceptions.switchBreak(2), "other")
local thrownTable = {}
eq("a throw passes finally unchanged", exceptions.throughFinally(thrownTable), thrownTable)
eq("a string passes finally unchanged", exceptions.throughFinally("s"), "s")
eq("rethrow keeps the table", exceptions.rethrown(thrownTable), thrownTable)
eq("rethrow keeps the string", exceptions.rethrown("again"), "again")
local catchLog = {}
local catchOk, catchError = pcall(exceptions.catchThrows, catchLog)
eq("a throw from catch escapes", catchOk, false)
eq("a throw from catch keeps its value", catchError, "second")
eq("finally runs before a throw from catch escapes", catchLog[1], "finally")
eq("finally return replaces a throw", exceptions.finallyReplacesThrow(), "finally")
eq("finally return replaces a return", exceptions.finallyReplacesReturn(), "finally")
local nestedLog = {}
eq("return through nested finally", exceptions.nestedReturn(nestedLog), 7)
list("nested finally order", nestedLog, { "inner", "outer" })
list("LuaTuple return through finally", { exceptions.tupleThroughFinally() }, { 1, "one" })
eq("finally ran for the LuaTuple return", exceptions.finishedCount(), 1)
eq("catch destructures the thrown value", exceptions.patternInCatch(), 5)
eq("finally ran after the destructuring catch", exceptions.finishedCount(), 2)

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
eq("non-null call on nil", operators.callNonNull(nil), nil)
eq(
	"non-null call",
	operators.callNonNull({
		run = function()
			return 4
		end,
	}),
	4
)
list("operands keep their order", operators.order(), { "0,0", "ab", 6 })
eq("&& keeps NaN falsy", operators.eitherOr(0 / 0), "fallback")
eq("&& keeps zero falsy", operators.eitherOr(0), "fallback")
eq("&& passes a truthy value", operators.eitherOr(1), "s")

local macros = load("macros")
list("arrays", macros.arrays(), { 3, 4, "1-2-3", "10,21,32", 1, 2, 1, -1, 6, true, true, true })
list("maps", macros.maps(), { 2, false, 1, true, false, 2, true, false })
list("strings", macros.strings("a,b"), { 3, "a,", 2, true, "number" })
list("macro operands keep their order", macros.order(), { "set,arg,set,arg,list,arg,list,arg", false, 1, "3+2" })
eq("assert passes", macros.checked(2), 2)
eq("assert rejects zero", pcall(macros.checked, 0), false)

local patterns = load("patterns")
list("destructure", patterns.destructure({ 1, 2, nil, 4, 5 }, { a = 9 }), { 1, "one", 1, 30, 2, 9, "default" })
eq("parameters", patterns.parameters({ x = 1 }, { 3, 4 }, 7, 8), 12)
list("assign patterns", patterns.assign({ 5, 6 }), { 5, 6, 1 })
list("LuaTuple elements", patterns.tuples(), { 10, 2, "first", "second" })

if failures > 0 then
	print(`{failures} failed`)
	process.exit(1)
end
print("lowering fixtures pass")
