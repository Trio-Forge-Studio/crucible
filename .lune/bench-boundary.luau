-- Splits the cost of one protected call into steps, so a change to the reactive boundary can be aimed at where the
-- time goes. Each "boundary steps" row adds one layer to the row above it, using the runtime's own functions; the
-- difference between two rows is that layer's cost. The release rows time only the release, values owned untimed.
-- Build the runtime first with `pnpm build`.
--
--   lune run bench-boundary [runs]
local process = require("@lune/process")
local spec = require("../test/lib/load")
local realm = require("../test/lib/reactive-world")
local ownable = require("../test/lib/ownable")

local OPTIMIZATION = 2
local N = 20000
local CALLBACK_HOOK = 8
local runs = tonumber(process.args[1]) or 15
assert(runs >= 7, "at least 7 runs")

spec.optimize(OPTIMIZATION)
local r = realm(nil, ownable.globals)
local graph = r.graph
local runner = graph.module("internal/runner")
local lifecycle = graph.module("internal/lifecycle")
local context = graph.module("internal/reactive/context")
local reactive = graph.module("internal/reactive/graph")
local own, scope = r.api.own, r.api.scope

-- `step(n)` does n of something and returns the seconds the timed part took.
local function time(label: string, step: (n: number) -> number)
	step(N)
	local samples = {}
	for _ = 1, runs do
		table.insert(samples, step(N))
	end
	table.sort(samples)
	print(
		string.format(
			"  %-46s %6.0f ns  (%.0f to %.0f)",
			label,
			samples[(#samples + 1) // 2] / N * 1e9,
			samples[1] / N * 1e9,
			samples[#samples] / N * 1e9
		)
	)
end

local function loop(body: () -> ()): (n: number) -> number
	return function(n)
		local start = os.clock()
		for _ = 1, n do
			body()
		end
		return os.clock() - start
	end
end

local hits = 0
local function body()
	hits += 1
end
local function noop() end
local OWNING = 3

local DONE = {}
local parked = coroutine.create(function()
	while true do
		coroutine.yield(DONE)
	end
end)

print(`{_VERSION}, optimization level {OPTIMIZATION}, {runs} runs of {N}; median ns each (min to max)`)
print("Luau and Lune primitives")
time("call a function", loop(body))
time(
	"pcall",
	loop(function()
		pcall(body)
	end)
)
time(
	"resume a parked coroutine",
	loop(function()
		coroutine.resume(parked)
	end)
)
time(
	"allocate a table of 3 fields",
	loop(function()
		local _ = { a = 1, b = 2, c = 3 }
	end)
)

print("Boundary steps, one function each, cumulative")
time("1 the function, called", loop(body))
time(
	"2 + one pooled worker (runner.call)",
	loop(function()
		runner.call(body)
	end)
)
local function onWorker()
	runner.call(body)
end
time(
	"3 + a second worker inside the first",
	loop(function()
		runner.call(onWorker)
	end)
)
time(
	"4 + the first as a lifecycle scope (within)",
	loop(function()
		lifecycle.within(nil, nil, CALLBACK_HOOK, onWorker)
	end)
)
local function framed(lifetime)
	local frame = { kind = OWNING, lifetime = lifetime }
	context.push(frame)
	body()
	context.pop(frame)
end
local lifetime = {}
if reactive.contained then
	-- The boundary before batching: `within` on one worker, the body on a second.
	time(
		"5 context.boundary: + job, completion, frames",
		loop(function()
			context.boundary(nil, body, nil, noop)
		end)
	)
	time(
		"6 graph.contained: + completion closure",
		loop(function()
			reactive.contained(nil, body, nil, noop)
		end)
	)
	time(
		"7 + an Owning frame pushed on the worker",
		loop(function()
			reactive.contained(nil, framed, lifetime, noop)
		end)
	)
else
	time(
		"5 context.boundary: one worker in a scope",
		loop(function()
			context.boundary(nil, nil, body, nil, noop)
		end)
	)
	local frame = { kind = OWNING, lifetime = lifetime }
	time(
		"6 + the body's frame installed",
		loop(function()
			context.boundary(nil, frame, body, nil, noop)
		end)
	)
end

print("Releases, a disposed scope, per value")
local function releases(label: string, make: () -> any)
	time(label, function(n)
		local values = table.create(n)
		for i = 1, n do
			values[i] = make()
		end
		local dispose = scope(function()
			for i = 1, n do
				own(values[i])
			end
		end)
		local start = os.clock()
		dispose()
		return os.clock() - start
	end)
end
releases("a function", function()
	return body
end)
local Lower = {}
Lower.__index = Lower
function Lower.destroy() end
releases("a table's destroy()", function()
	return setmetatable({}, Lower)
end)
releases("a connection", function()
	return ownable.connection(noop)
end)
time("a function, released at once by an ended owner", function(n)
	local dispose = scope(function()
		r.api.onCleanup(function()
			for _ = 1, n do
				r.api.onCleanup(body)
			end
		end)
	end)
	local start = os.clock()
	dispose()
	return os.clock() - start
end)
