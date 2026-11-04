-- Times the same UI as trunk B (the spike's binding on a patched trunk flush), head B (the runtime's own binding
-- node) and Vide, all on one fake Instance and at one optimization level. Each sample runs every subject once, in
-- an order that rotates by run, so warm-up and drift favour none. Crucible times include the flush that brings the
-- UI to rest; Vide updates at the write. Build both runtimes first, then from the repository root:
--
--   lune run spike/ui/bench <trunk out> <head out> [runs] [sections, such as 124]
local process = require("@lune/process")
local subjects = require("./lib/subjects")
local lists = require("./lib/lists")
local fake = require("./lib/instance")
local runtime = require("./lib/runtime")
local gc = require("./lib/gc")

local N = 1000
subjects.roots.trunk = assert(process.args[1], "the trunk runtime's out folder")
subjects.roots.head = assert(process.args[2], "the head runtime's out folder")
local RUNS = tonumber(process.args[3]) or 15
assert(RUNS >= 7, "at least 7 runs")
local SECTIONS = process.args[4] or "123456"
local function section(n: number): boolean
	return string.find(SECTIONS, tostring(n), 1, true) ~= nil
end

local function summary(unsorted: { number }, scale: number): string
	local samples = table.clone(unsorted)
	table.sort(samples)
	local n = #samples
	local median = if n % 2 == 1 then samples[(n + 1) / 2] else (samples[n / 2] + samples[n / 2 + 1]) / 2
	return string.format("%9.2f  (%.2f to %.2f)", median * scale, samples[1] * scale, samples[n] * scale)
end

local collect = gc.cycle

local function row(label: string, name: string, samples: { number }, scale: number, unit: string)
	print(string.format("  %-34s %-13s %s %s", label, name, summary(samples, scale), unit))
end

local function states(ui, n: number, shared)
	local out = table.create(n)
	for i = 1, n do
		out[i] = shared or ui.state(i)
	end
	return out
end

local function expectWrites(before: number, count: number, label: string)
	local wrote = fake.writes - before
	assert(wrote >= count, `{label}: {wrote} writes, expected at least {count}`)
end

-- Runs `sample(subject)` for every subject once per run, rotating which goes first, after one untimed run each.
-- A sample returns one number per measured quantity; the result holds each subject's list for each quantity.
local function interleave(names: { string }, sample: (string) -> { number })
	local results = {}
	for _, name in names do
		results[name] = {}
	end
	for run = 0, RUNS do
		for offset = 0, #names - 1 do
			local name = names[(run + offset) % #names + 1]
			local values = sample(name)
			if run > 0 then
				for q, value in values do
					results[name][q] = results[name][q] or {}
					table.insert(results[name][q], value)
				end
			end
		end
	end
	return results
end

print(`{_VERSION}, optimization level {runtime.OPTIMIZATION}, {RUNS} interleaved runs; median (min to max)`)
print(`trunk: {subjects.roots.trunk}\nhead:  {subjects.roots.head}`)

if section(1) then
	print(`\n1. Mount and unmount {N} elements, 3 fixed and 2 reactive props each`)
	local uis = {}
	for _, name in subjects.names do
		uis[name] = subjects.make(name)
	end
	local results = interleave(subjects.names, function(name)
		local ui = uis[name]
		local texts, colors = states(ui, N), states(ui, N)
		collect()
		local before = fake.writes
		local start = os.clock()
		local _, dispose = ui.mountItems(texts, colors)
		local mounted = os.clock()
		expectWrites(before, 5 * N, `{name} mount`)
		dispose()
		ui.flush()
		local finished = os.clock()
		return { mounted - start, finished - mounted }
	end)
	for _, name in subjects.names do
		row("mount", name, results[name][1], 1e6 / N, "µs an element")
	end
	for _, name in subjects.names do
		row("unmount", name, results[name][2], 1e6 / N, "µs an element")
	end
end

if section(2) then
	print(`\n2. Update {N} bound props`)
	local setups = {}
	for _, name in subjects.names do
		local ui = subjects.make(name)
		local shared = ui.state(0)
		local texts = states(ui, N)
		ui.mountItems(texts, states(ui, N, shared))
		setups[name] = { ui = ui, shared = shared, texts = texts }
	end
	local value = 0.5
	local results = interleave(subjects.names, function(name)
		local s = setups[name]
		local ui = s.ui
		value += 1
		collect()
		local before = fake.writes
		local start = os.clock()
		for i = 1, N do
			ui.set(s.texts[i], value)
		end
		ui.flush()
		local separate = os.clock() - start
		expectWrites(before, N, `{name} separate states`)
		collect()
		before = fake.writes
		start = os.clock()
		ui.set(s.shared, value)
		ui.flush()
		local one = os.clock() - start
		expectWrites(before, N, `{name} one state`)
		return { separate, one }
	end)
	for _, name in subjects.names do
		row(`{N} separate states`, name, results[name][1], 1e6 / N, "µs a prop")
	end
	for _, name in subjects.names do
		row(`{N} bindings on one state`, name, results[name][2], 1e6 / N, "µs a prop")
	end
	-- Each run times every subject back to back, so a per-run ratio cancels drift the subjects share.
	local function paired(q: number, label: string, name: string)
		local ratios = {}
		for i, value in results[name][q] do
			table.insert(ratios, value / results.Vide[q][i])
		end
		row(label, `{name}/Vide`, ratios, 1, "paired ratio")
	end
	for _, name in { "trunk B", "head B" } do
		paired(1, `{N} separate states`, name)
		paired(2, `{N} bindings on one state`, name)
	end
end

if section(3) then
	print(`\n3. Keyed For of {N} rows, 2 reactive props a row (one list change, flushed)`)
	local operations = {
		{
			"reverse",
			function(_, base)
				return lists.reverse(base)
			end,
		},
		{
			"insert 100 in the middle",
			function(ui, base)
				return lists.insertMiddle(base, lists.items(ui, 100, N))
			end,
		},
		{
			"remove 100 from the middle",
			function(_, base)
				return lists.removeMiddle(base, 100)
			end,
		},
		{
			"swap two rows",
			function(_, base)
				return lists.swap(base, 2, N - 1)
			end,
		},
	}
	local uis = {}
	for _, name in subjects.names do
		uis[name] = subjects.make(name)
	end
	for _, operation in operations do
		local results = interleave(subjects.names, function(name)
			local ui = uis[name]
			local base = lists.items(ui, N)
			local list = ui.state(base)
			local frame, dispose = ui.mountList(list)
			local following = operation[2](ui, base)
			collect()
			local start = os.clock()
			ui.set(list, following)
			ui.flush()
			local elapsed = os.clock() - start
			local ordered = fake.ordered(frame)
			assert(#ordered == #following, `{operation[1]}: {#ordered} rows`)
			for i, child in ordered do
				assert(child.Text == following[i].label(), `{operation[1]}: row {i} out of order`)
			end
			dispose()
			ui.flush()
			return { elapsed }
		end)
		for _, name in subjects.names do
			row(operation[1], name, results[name][1], 1e3, "ms")
		end
	end
end

if section(4) then
	local MEMORY_N = 10 * N
	print(`\n4. Memory per mounted element with 2 reactive props ({MEMORY_N} mounted, heap settled before and after)`)
	local SIZE = table.freeze({ 1, 0, 0, 24 })
	local function plainInstances(texts, colors)
		local root = fake.new("Frame")
		for i = 1, #texts do
			local e = fake.new("TextLabel")
			e.Name = "Item"
			e.Size = SIZE
			e.BackgroundTransparency = 1
			e.Text = texts[i]()
			e.TextColor3 = colors[i]()
			e.Parent = root
		end
		return root
	end
	local uis = {}
	for _, name in subjects.names do
		uis[name] = subjects.make(name)
	end
	local mounts = {
		["nothing mounted (noise floor)"] = function()
			return function() end
		end,
		["Instances alone, no reactivity"] = function(texts, colors)
			local root = plainInstances(texts, colors)
			return function()
				root:Destroy()
			end
		end,
	}
	local function mountWith(ui)
		return function(texts, colors)
			local _, dispose = ui.mountItems(texts, colors)
			return function()
				dispose()
				ui.flush()
			end
		end
	end
	for _, name in subjects.names do
		mounts[name] = mountWith(uis[name])
	end
	local stateMaker = uis["head B"]
	-- One sample per call, so nothing from an earlier sample is still held by a register of this frame.
	local function sample(name: string): { number }
		local texts, colors = states(stateMaker, MEMORY_N), states(stateMaker, MEMORY_N)
		if uis[name] ~= nil then
			texts, colors = states(uis[name], MEMORY_N), states(uis[name], MEMORY_N)
		end
		local before = gc.settled()
		local unmount = mounts[name](texts, colors)
		local after = gc.settled()
		unmount()
		return { after - before }
	end
	local names = { "nothing mounted (noise floor)", "Instances alone, no reactivity" }
	for _, name in subjects.names do
		table.insert(names, name)
	end
	local results = interleave(names, sample)
	for _, name in names do
		row(if uis[name] then "mounted, Instances included" else name, if uis[name] then name else "-", results[name][1], 1 / MEMORY_N, "bytes an element")
	end
end

if section(5) then
	-- The Crucible-to-Vide bridge as each runtime ships it: `useReadable` from the runtime's vide entry, each source
	-- read by one Vide effect the way a component binds a property.
	print(`\n5. useReadable, {N} bindings, each read by one Vide effect`)
	local bridges = {}
	for _, side in { "trunk", "head" } do
		local vide = runtime.vide()
		local r = runtime.crucible({ root = subjects.roots[side], vide = vide })
		bridges[`{side} bridge`] = { r = r, vide = vide, useReadable = r.module("vide").useReadable }
	end
	local names = { "trunk bridge", "head bridge" }
	local seen = 0
	local results = interleave(names, function(name)
		local b = bridges[name]
		local vide, r, useReadable = b.vide, b.r, b.useReadable
		local function bind(read)
			vide.effect(function()
				seen += 1
				return read()
			end)
		end
		local sources = table.create(N)
		for i = 1, N do
			sources[i] = r.api.state(0)
		end
		collect()
		local start = os.clock()
		local destroy = vide.root(function()
			for i = 1, N do
				bind(useReadable(sources[i]))
			end
		end)
		r.flush()
		destroy()
		local create = os.clock() - start
		local destroyUpdate = vide.root(function()
			for i = 1, N do
				bind(useReadable(sources[i]))
			end
		end)
		r.flush()
		seen = 0
		collect()
		start = os.clock()
		for i = 1, N do
			sources[i]:set(1)
		end
		r.flush()
		local update = os.clock() - start
		assert(seen == N, `{name} update: {seen} of {N} Vide effects ran`)
		destroyUpdate()
		return { create, update }
	end)
	for _, name in names do
		row("create, first flush, destroy", name, results[name][1], 1e6 / N, "µs a binding")
	end
	for _, name in names do
		row("update", name, results[name][2], 1e6 / N, "µs a binding")
	end
end

if section(6) then
	-- What one state write allocates, with one effect observing the state. The effect stays queued after the first
	-- write, so later writes only walk the observer set. The window is small, so a GC step inside it is rare. One
	-- would make that window low, never high.
	local WINDOW = 64
	print(`\n6. Heap growth per state write, one observer, {WINDOW} writes a window`)
	local realms = {}
	for _, side in { "trunk", "head" } do
		local r = runtime.crucible({ root = subjects.roots[side] })
		local s = r.api.state(0)
		r.api.scope(function()
			r.api.effect(function()
				s()
			end)
		end)
		r.flush()
		realms[side] = { r = r, s = s }
	end
	local names = { "trunk", "head" }
	local value = 0.5
	local results = interleave(names, function(side)
		local s = realms[side].s
		collect()
		local before = collectgarbage("count")
		for _ = 1, WINDOW do
			value += 1
			s:set(value)
		end
		local grown = collectgarbage("count") - before
		realms[side].r.flush()
		return { grown * 1024 }
	end)
	for _, name in names do
		row("write", name, results[name][1], 1 / WINDOW, "bytes a write")
	end
end
