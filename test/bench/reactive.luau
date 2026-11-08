-- Time per write that reaches one effect, and memory per value with one effect, at 1,000 values. Prints one JSON line.
-- `lune run test/bench/reactive.luau [root]` measures the compiled runtime in `root`, packages/crucible/out by default.
-- A runtime that exports `state` is measured with its own values, whose effects run on the flush it defers; any other
-- with Charm atoms and Crucible's `effect`, which runs inside the write.
local process = require("@lune/process")
local serde = require("@lune/serde")
local Engine = require("../lib/engine")
local spec = require("../lib/load")

local VALUES = 1_000
local WRITES = 100_000
local WARMUP = 10_000

local root = process.args[1] or "packages/crucible/out"
local engine = Engine.new()
local graph = spec.runtimeAt(root, nil, engine.loggingPort)
local api = graph.module("")
local legacy = api.state ~= nil

local makeValue, write, settle
if legacy then
	local flush = graph.module("internal/reactive/flush")
	flush.useReactivePort(engine.port)
	flush.releaseBarrier()
	makeValue = function()
		return api.state(0)
	end
	write = function(value, n)
		value:set(n)
	end
	settle = function()
		engine.flush()
	end
else
	local charm = graph.package("@rbxts/charm/src")
	charm.flags.strict = false
	makeValue = function()
		return charm.atom(0)
	end
	write = function(value, n)
		value(n)
	end
	settle = function() end
end

-- Luau cannot force a collection, so this churns garbage through several collection cycles and keeps the lowest heap
-- size it sees: the live heap plus at most one small table.
local function heap(): number
	local lowest = math.huge
	for _ = 1, 2_000_000 do
		local _ = { 0 }
		local size = collectgarbage("count")
		if size < lowest then
			lowest = size
		end
	end
	return lowest * 1024
end

local runs = 0
local function body(value)
	return function()
		value()
		runs += 1
	end
end

local before = heap()
local values = table.create(VALUES)
local dispose = api.scope(function()
	for i = 1, VALUES do
		local value = makeValue()
		values[i] = value
		api.effect(body(value))
	end
end)
settle()
local after = heap()
assert(runs == VALUES, `every effect ran once: {runs}`)

for n = 1, WARMUP do
	write(values[n % VALUES + 1], -n)
	settle()
end
runs = 0
local started = os.clock()
for n = 1, WRITES do
	write(values[n % VALUES + 1], n)
	settle()
end
local elapsed = os.clock() - started
assert(runs == WRITES, `every write reached its effect: {runs}`)
dispose()

print(serde.encode("json", {
	engine = if legacy then "state" else "charm",
	nsPerWrite = elapsed / WRITES * 1e9,
	bytesPerValue = (after - before) / VALUES,
}))
