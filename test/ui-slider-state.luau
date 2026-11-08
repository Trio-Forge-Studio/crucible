local M = require("./lib/ui-load")()("slider-state")
return {
	["slider snaps from min clamps bounds and reaches a partial final step"] = function()
		local range = { min = 2, max = 9, step = 2 }
		M.validateRange(range)
		assert(M.snapValue(-5, range) == 2, "clamps minimum")
		assert(M.snapValue(5, range) == 6, "snaps relative to min")
		assert(M.snapValue(20, range) == 9, "clamps maximum even off grid")
		assert(M.snapValue(8.9, range) == 8, "interior snaps to a step")
		assert(M.snapValue(7, { min = 4, max = 4, step = 1 }) == 4, "fixed range")
	end,
	["slider rejects invalid ranges steps and values"] = function()
		for _, range in
			{
				{ min = 5, max = 1, step = 1 },
				{ min = 0, max = math.huge, step = 1 },
				{ min = 0, max = 1, step = 0 },
				{ min = 0 / 0, max = 1, step = 1 },
				{ min = 0, max = 1, step = -1 },
			}
		do
			assert(not pcall(M.validateRange, range), "invalid configuration rejected")
		end
		assert(not pcall(M.snapValue, 0 / 0, { min = 0, max = 1, step = 1 }), "NaN rejected")
	end,
	["slider commits the last changed value once per hold"] = function()
		local hold = M.SliderCommit.new()
		hold:begin()
		assert(hold:release() == nil, "unchanged hold does not commit")
		hold:begin()
		hold:change(4)
		hold:change(6)
		assert(hold:release() == 6, "last changed value commits")
		assert(hold:release() == nil, "duplicate release does not commit")
		hold:change(8)
		hold:begin()
		assert(hold:release() == nil, "new hold starts clean")
	end,
	["slider maps pointer fractions onto snapped values and back"] = function()
		local range = { min = 10, max = 20, step = 5 }
		assert(M.valueAt(0.3, range) == 15, "a fraction snaps to the nearest step")
		assert(M.valueAt(-1, range) == 10 and M.valueAt(2, range) == 20, "fractions past the track clamp")
		assert(M.fractionOf(15, range) == 0.5, "fraction is relative to min")
		assert(M.fractionOf(4, { min = 4, max = 4, step = 1 }) == 0, "a fixed range has a finite fraction")
	end,
	["slider hold repeats after 0.4 s and accelerates to one step every 0.04 s"] = function()
		local hold = M.holdStart
		local fired = {}
		local function tick(delta)
			local result = M.tickRepeat(hold, delta)
			hold = result.state
			table.insert(fired, result.fire)
		end
		tick(0.39)
		assert(not fired[1], "nothing repeats before the delay")
		tick(0.01)
		assert(fired[2], "the first repeat fires at 0.4 s")
		tick(0.149)
		assert(not fired[3], "the next repeat waits 0.15 s")
		tick(0.001)
		assert(fired[4], "it fires at 0.55 s")
		local due = hold.due - hold.elapsed
		assert(math.abs(due - (0.15 - 0.15 * 0.04)) < 1e-9, "the interval shrinks with hold time")
		tick(10)
		assert(fired[5] and math.abs(hold.due - hold.elapsed - 0.04) < 1e-9, "the interval bottoms out at 0.04 s")
		tick(0.001)
		assert(not fired[6], "a long frame leaves no backlog of steps")
	end,
}
