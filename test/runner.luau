local spec = require("./lib/load")
local task = require("@lune/task")

local CallResult = { Ok = 0, Threw = 1, Yielded = 2 }

-- Counts worker creation so a test can see the steady state allocate none.
local created = 0
local counted = setmetatable({
	create = function(f)
		created += 1
		return coroutine.create(f)
	end,
}, { __index = coroutine })

local function add(a, b)
	return a + b
end

return {
	["a returning callback gives Ok and its value"] = function()
		local runner = spec.fresh("internal/runner")
		local result, value = runner.call(add, 2, 3)
		spec.eq(result, CallResult.Ok, "result")
		spec.eq(value, 5, "value")
	end,

	["a throwing callback gives Threw with the error and a traceback"] = function()
		local runner = spec.fresh("internal/runner")
		local result, value = runner.call(function()
			error("hook threw")
		end, nil, nil)
		spec.eq(result, CallResult.Threw, "result")
		spec.has(value, "hook threw", "message")
	end,

	["a yielding callback gives Yielded, and the rest of it never runs"] = function()
		local runner = spec.fresh("internal/runner")
		local resumed = false
		local result, value = runner.call(function()
			coroutine.yield()
			resumed = true
		end, nil, nil)
		spec.eq(result, CallResult.Yielded, "result")
		spec.has(value, "only start may yield", "message")
		spec.eq(resumed, false, "cancelled at the yield")
	end,

	["pcall alone lets the same yield through, which is why the runner exists"] = function()
		local thread = coroutine.create(function()
			return pcall(function()
				coroutine.yield("user yield")
			end)
		end)
		local ok, value = coroutine.resume(thread)
		spec.eq(ok and value == "user yield" and coroutine.status(thread) == "suspended", true, "pcall yielded through")
		coroutine.close(thread)
	end,

	["a callback that calls the runner again gets its own worker"] = function()
		local runner = spec.fresh("internal/runner")
		local result, value = runner.call(function()
			local inner, innerValue = runner.call(function(x)
				return x * 2
			end, 21, nil)
			return if inner == CallResult.Ok then innerValue else -1
		end, nil, nil)
		spec.eq(result, CallResult.Ok, "outer")
		spec.eq(value, 42, "inner value")
	end,

	["calls that return allocate no worker once the pool is warm"] = function()
		local runner = spec.fresh("internal/runner", { coroutine = counted })
		runner.call(add, 1, 1)
		local before = created
		for i = 1, 1000 do
			runner.call(add, i, i)
		end
		spec.eq(created - before, 0, "workers created in 1,000 calls")
		runner.call(function()
			error("once")
		end, nil, nil)
		runner.call(add, 1, 1)
		spec.eq(created - before, 1, "a failure costs one new worker")
	end,

	["act-on 4: a yield whose cancellation throws still gives Yielded and keeps the worker out of the pool"] = function()
		local refusing = setmetatable({
			cancel = function()
				error("cancel refused")
			end,
		}, { __index = task })
		local runner = spec.fresh("internal/runner", { coroutine = counted, task = refusing })
		local ok, result, value = pcall(runner.call, function()
			coroutine.yield()
		end, nil, nil)
		spec.eq(ok, true, `the call returned: {result}`)
		spec.eq(result, CallResult.Yielded, "result")
		spec.has(value, "only start may yield", "the yield is named")
		spec.has(value, "cancellation failed", "the failed cancellation is named")
		spec.has(value, "cancel refused", "the cancellation error is kept")
		local before = created
		runner.call(add, 1, 1)
		spec.eq(created - before, 1, "the next call takes a new worker")
	end,
}
