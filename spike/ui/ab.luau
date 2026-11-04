-- Lever A/B: loads any number of built runtimes plus Vide, mounts the section-2 UI on each, and per run times each
-- subject's update 5 times, keeping the fastest, in an order that rotates by run. Prints the median of those minima
-- and the median paired ratio to Vide. lune run spike/ui/ab <runs> <name=out> [name=out ...]
local process = require("@lune/process")
local runtime = require("./lib/runtime")
local CrucibleUI = require("./lib/crucible-ui")
local VideUI = require("./lib/vide-ui")
local fake = require("./lib/instance")
local gc = require("./lib/gc")

local N = 1000
local REPS = 5
local RUNS = tonumber(process.args[1]) or 21
local subjects = {}
for i = 2, #process.args do
	local name, root = string.match(process.args[i], "^([^=]+)=(.+)$")
	local r = runtime.crucible({ root = root })
	table.insert(subjects, { name = name, ui = CrucibleUI(r, "H") })
end
table.insert(subjects, { name = "Vide", ui = VideUI(runtime.vide()) })
for _, s in subjects do
	local ui = s.ui
	s.shared = ui.state(0)
	s.texts = table.create(N)
	local colors = table.create(N)
	for i = 1, N do
		s.texts[i] = ui.state(i)
		colors[i] = s.shared
	end
	ui.mountItems(s.texts, colors)
	s.separate, s.one = {}, {}
end
local value = 0.5
local function best(fn): number
	local low = math.huge
	for _ = 1, REPS do
		value += 1
		gc.cycle()
		local before = fake.writes
		local start = os.clock()
		fn(value)
		local elapsed = os.clock() - start
		assert(fake.writes - before >= N, "every prop was written")
		low = math.min(low, elapsed)
	end
	return low
end
for run = 0, RUNS do
	for offset = 0, #subjects - 1 do
		local s = subjects[(run + offset) % #subjects + 1]
		local ui = s.ui
		local separate = best(function(v)
			for i = 1, N do
				ui.set(s.texts[i], v)
			end
			ui.flush()
		end)
		local one = best(function(v)
			ui.set(s.shared, v)
			ui.flush()
		end)
		if run > 0 then
			table.insert(s.separate, separate)
			table.insert(s.one, one)
		end
	end
end
local function median(list)
	local t = table.clone(list)
	table.sort(t)
	local n = #t
	return if n % 2 == 1 then t[(n + 1) / 2] else (t[n / 2] + t[n / 2 + 1]) / 2
end
local vide = subjects[#subjects]
print(`{RUNS} runs, min of {REPS} each; µs a prop, then the median paired ratio to Vide`)
for _, s in subjects do
	local rs, ro = {}, {}
	for i = 1, RUNS do
		rs[i] = s.separate[i] / vide.separate[i]
		ro[i] = s.one[i] / vide.one[i]
	end
	print(string.format("%-8s separate %.2f (%.3f)   one %.2f (%.3f)", s.name, median(s.separate) * 1e6 / N, median(rs), median(s.one) * 1e6 / N, median(ro)))
end
