local spec = require("./lib/load")

local scheduler = spec.module("internal/scheduler")
local tests = {}
local function test(name, fn)
	tests[name] = fn
end
local STEP = 1 / 60

local function near(actual, expected, label)
	if math.abs(actual - expected) > 1e-9 then
		error(`{label}: expected {expected}, got {actual}`, 2)
	end
end

test("a frame of exactly one step runs one step and carries nothing", function()
	local steps, carry = scheduler.accumulate(0, STEP, STEP)
	spec.eq(steps, 1, "steps")
	near(carry, 0, "carry")
end)

test("short frames accumulate until a whole step is due", function()
	local steps, carry = scheduler.accumulate(0, STEP * 0.6, STEP)
	spec.eq(steps, 0, "first frame")
	steps, carry = scheduler.accumulate(carry, STEP * 0.6, STEP)
	spec.eq(steps, 1, "second frame")
	near(carry, STEP * 0.2, "fraction kept")
end)

test("a long frame runs at most four steps, drops the other whole steps and keeps the fraction", function()
	local steps, carry = scheduler.accumulate(0, STEP * 10.5, STEP)
	spec.eq(steps, 4, "steps")
	near(carry, STEP * 0.5, "fraction kept")
end)

test("a hitch longer than 0.25 s counts as 0.25 s", function()
	local steps, carry = scheduler.accumulate(0, 3, STEP)
	spec.eq(steps, 4, "steps")
	near(carry, 0.25 - 15 * STEP, "fraction of the clamped frame")
end)

test("a negative frame time counts as zero", function()
	local steps, carry = scheduler.accumulate(STEP * 0.5, -1, STEP)
	spec.eq(steps, 0, "steps")
	near(carry, STEP * 0.5, "carry unchanged")
end)

test("the carry never goes negative from rounding", function()
	local carry = 0
	for _ = 1, 10000 do
		local _steps
		_steps, carry = scheduler.accumulate(carry, 1 / 59.7, STEP)
		if carry < 0 or carry >= STEP then
			error(`carry left [0, step): {carry}`)
		end
	end
end)

return tests
