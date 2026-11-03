-- Times one flush that reruns 10,000 effects, each with its own state, so every run goes through the protected
-- boundary once. "with a cleanup" effects also return a function, which the next run releases. Build the runtime
-- first with `pnpm build`.
--
--   lune run bench-effects [runs]
local process = require("@lune/process")
local spec = require("../test/lib/load")
local realm = require("../test/lib/reactive-world")

local COUNT = 10000
local OPTIMIZATION = 2
local runs = tonumber(process.args[1]) or 15
assert(runs >= 7, "at least 7 runs")

spec.optimize(OPTIMIZATION)

local function median(samples: { number }): number
	table.sort(samples)
	local n = #samples
	return if n % 2 == 1 then samples[(n + 1) / 2] else (samples[n / 2] + samples[n / 2 + 1]) / 2
end

local function subject(withCleanup: boolean)
	local r = realm()
	local api = r.api
	local states = table.create(COUNT)
	local ran = 0
	local released = 0
	local function cleanup()
		released += 1
	end
	r.root(function()
		for i = 1, COUNT do
			local s = api.state(0)
			states[i] = s
			if withCleanup then
				api.effect(function()
					s()
					ran += 1
					return cleanup
				end)
			else
				api.effect(function()
					s()
					ran += 1
				end)
			end
		end
	end)
	r.flush()
	return function(round: number): number
		for i = 1, COUNT do
			states[i]:set(round)
		end
		ran, released = 0, 0
		local start = os.clock()
		r.flush()
		local elapsed = os.clock() - start
		assert(ran == COUNT, `{ran} of {COUNT} effects ran`)
		assert(not withCleanup or released == COUNT, `{released} of {COUNT} cleanups released`)
		return elapsed
	end
end

print(`{_VERSION}, optimization level {OPTIMIZATION}, {runs} runs; median (min to max)`)
for _, case in { { "effects", false }, { "effects with a cleanup", true } } do
	local rerun = subject(case[2])
	rerun(1)
	local samples = {}
	for round = 2, runs + 1 do
		table.insert(samples, rerun(round))
	end
	local middle = median(samples)
	print(
		string.format(
			"%-24s %7.2f ms (%.2f to %.2f), %5.0f ns an effect",
			`{COUNT} {case[1]}`,
			middle * 1e3,
			samples[1] * 1e3,
			samples[#samples] * 1e3,
			middle / COUNT * 1e9
		)
	)
end
