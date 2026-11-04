-- Times the same UI in Crucible A (an effect per reactive prop), Crucible B (a binding per reactive prop) and Vide,
-- all on one fake Instance and at one optimization level. Crucible times include the flush that brings the UI to
-- rest; Vide updates at the write. Run from the repository root after `pnpm build`:
--
--   lune run spike/ui/bench [runs] [sections, such as 124]
local process = require("@lune/process")
local subjects = require("./lib/subjects")
local lists = require("./lib/lists")
local fake = require("./lib/instance")
local runtime = require("./lib/runtime")
local gc = require("./lib/gc")

local N = 1000
local RUNS = tonumber(process.args[1]) or 15
assert(RUNS >= 7, "at least 7 runs")
local SECTIONS = process.args[2] or "123456"
local function section(n: number): boolean
	return string.find(SECTIONS, tostring(n), 1, true) ~= nil
end

local function summary(samples: { number }, scale: number): string
	table.sort(samples)
	local n = #samples
	local median = if n % 2 == 1 then samples[(n + 1) / 2] else (samples[n / 2] + samples[n / 2 + 1]) / 2
	return string.format("%8.2f  (%.2f to %.2f)", median * scale, samples[1] * scale, samples[n] * scale)
end

local collect = gc.cycle

local function row(label: string, name: string, samples: { number }, scale: number, unit: string)
	print(string.format("  %-34s %-11s %s %s", label, name, summary(samples, scale), unit))
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

print(`{_VERSION}, optimization level {runtime.OPTIMIZATION}, {RUNS} runs; median (min to max)`)

if section(1) then
	print(`\n1. Mount and unmount {N} elements, 3 fixed and 2 reactive props each`)
	for _, name in subjects.names do
		local ui = subjects.make(name)
		local mounts, unmounts = {}, {}
		for run = -1, RUNS do
			local texts, colors = states(ui, N), states(ui, N)
			collect()
			local start = os.clock()
			local _, dispose = ui.mountItems(texts, colors)
			local mounted = os.clock()
			dispose()
			ui.flush()
			local finished = os.clock()
			if run > 0 then
				table.insert(mounts, mounted - start)
				table.insert(unmounts, finished - mounted)
			end
		end
		row("mount", name, mounts, 1e6 / N, "µs an element")
		row("unmount", name, unmounts, 1e6 / N, "µs an element")
	end
end

if section(2) then
	print(`\n2. Update {N} bound props`)
	for _, name in subjects.names do
		local ui = subjects.make(name)
		local shared = ui.state(0)
		local texts = states(ui, N)
		local _, dispose = ui.mountItems(texts, states(ui, N, shared))
		local separate, one = {}, {}
		for run = -1, RUNS do
			local value = run + 0.5
			collect()
			local before = fake.writes
			local start = os.clock()
			for i = 1, N do
				ui.set(texts[i], value)
			end
			ui.flush()
			local elapsed = os.clock() - start
			expectWrites(before, N, "separate states")
			collect()
			before = fake.writes
			start = os.clock()
			ui.set(shared, value)
			ui.flush()
			local elapsedOne = os.clock() - start
			expectWrites(before, N, "one state")
			if run > 0 then
				table.insert(separate, elapsed)
				table.insert(one, elapsedOne)
			end
		end
		row(`{N} separate states`, name, separate, 1e6 / N, "µs a prop")
		row(`{N} bindings on one state`, name, one, 1e6 / N, "µs a prop")
		dispose()
		ui.flush()
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
	for _, operation in operations do
		for _, name in subjects.names do
			local ui = subjects.make(name)
			local samples = {}
			for run = -1, RUNS do
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
				if run > 0 then
					table.insert(samples, elapsed)
				end
			end
			row(operation[1], name, samples, 1e3, "ms")
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
	-- One sample per call, so nothing from an earlier sample is still held by a register of this frame.
	local function sample(ui, mount: (texts: any, colors: any) -> () -> ()): number
		local texts, colors = states(ui, MEMORY_N), states(ui, MEMORY_N)
		local before = gc.settled()
		local unmount = mount(texts, colors)
		local after = gc.settled()
		unmount()
		return after - before
	end
	local function memory(label: string, name: string, ui, mount: (texts: any, colors: any) -> () -> ())
		local samples = {}
		for run = -1, RUNS do
			local bytes = sample(ui, mount)
			if run > 0 then
				table.insert(samples, bytes)
			end
		end
		row(label, name, samples, 1 / MEMORY_N, "bytes an element")
	end
	do
		local ui = subjects.make("Crucible A")
		memory("nothing mounted (noise floor)", "-", ui, function()
			return function() end
		end)
		memory("Instances alone, no reactivity", "fake", ui, function(texts, colors)
			local root = plainInstances(texts, colors)
			return function()
				root:Destroy()
			end
		end)
	end
	for _, name in subjects.names do
		local ui = subjects.make(name)
		memory("mounted, Instances included", name, ui, function(texts, colors)
			local _, dispose = ui.mountItems(texts, colors)
			return function()
				dispose()
				ui.flush()
			end
		end)
	end
end

if section(5) then
	print(`\n5. Where an update's time goes, {N} props (µs a prop)`)
	local function timed(samples, body: () -> ())
		collect()
		local start = os.clock()
		body()
		table.insert(samples, os.clock() - start)
	end
	do
		local ui = subjects.make("Crucible B")
		local r = ui.realm
		local shared = ui.state(0)
		local texts = states(ui, N)
		local frame, dispose = ui.mountItems(texts, states(ui, N, shared))
		local nodes = {}
		for i = 1, N do
			for observer in texts[i].observers do
				table.insert(nodes, observer)
			end
		end
		local writes, flushes, oneWrite, oneFlush, sorts = {}, {}, {}, {}, {}
		local createdBefore = r.graph.createdBefore
		for run = -1, RUNS do
			local value = run + 0.5
			timed(writes, function()
				for i = 1, N do
					texts[i]:set(value)
				end
			end)
			timed(flushes, r.flush)
			timed(oneWrite, function()
				shared:set(value)
			end)
			timed(oneFlush, r.flush)
			local shuffled = table.clone(nodes)
			for i = #shuffled, 2, -1 do
				local j = math.random(i)
				shuffled[i], shuffled[j] = shuffled[j], shuffled[i]
			end
			timed(sorts, function()
				table.sort(shuffled, createdBefore)
			end)
		end
		row("B: write separate states", "mark+queue", writes, 1e6 / N, "")
		row("B: flush those", "run", flushes, 1e6 / N, "")
		row("B: write one state", "mark+queue", oneWrite, 1e6 / N, "")
		row("B: flush that", "run", oneFlush, 1e6 / N, "")
		row("B: the flush's sort of the queue", "sort", sorts, 1e6 / N, "")
		local labels = frame:GetChildren()
		local bare, guarded = {}, {}
		local function set(instance, value)
			instance.Text = value
		end
		for run = -1, RUNS do
			timed(bare, function()
				for i = 1, N do
					labels[i].Text = run
				end
			end)
			timed(guarded, function()
				for i = 1, N do
					pcall(set, labels[i], run)
				end
			end)
		end
		row("fake property write alone", "write", bare, 1e6 / N, "")
		row("property write under pcall", "write", guarded, 1e6 / N, "")
		dispose()
		ui.flush()
	end
	do
		local ui = subjects.make("Vide")
		local shared = ui.state(0)
		local texts = states(ui, N)
		local _, dispose = ui.mountItems(texts, states(ui, N, shared))
		local separate, one = {}, {}
		for run = -1, RUNS do
			local value = run + 0.5
			timed(separate, function()
				for i = 1, N do
					texts[i](value)
				end
			end)
			timed(one, function()
				shared(value)
			end)
		end
		row("Vide: write separate sources", "write+run", separate, 1e6 / N, "")
		row("Vide: write one source", "write+run", one, 1e6 / N, "")
		dispose()
	end
end

if section(6) then
	-- A bridge built on B's node: a binding whose target writes a Vide source instead of an Instance property. Each
	-- of the N labels is a Vide label on its own source, fed from its own Crucible state.
	print(`\n6. Crucible state into Vide UI through a binding-node bridge, {N} labels`)
	local r = runtime.crucible()
	local bind = require("./lib/binding")(r).bind
	local vide = runtime.vide()
	local ui = require("./lib/vide-ui")(vide)
	local texts = table.create(N)
	local sources = table.create(N)
	for i = 1, N do
		texts[i] = r.api.state(i)
		sources[i] = vide.source(i)
	end
	local _, unmountVide = ui.mountItems(sources, states(ui, N))
	local creates, updates = {}, {}
	for run = -1, RUNS do
		collect()
		local start = os.clock()
		local disposeBridge = r.api.scope(function()
			for i = 1, N do
				local source = sources[i]
				bind(
					setmetatable({}, {
						__newindex = function(_, _, value)
							source(value)
						end,
					}),
					"value",
					texts[i]
				)
			end
		end)
		local created = os.clock() - start
		local value = run + 0.5
		local before = fake.writes
		collect()
		start = os.clock()
		for i = 1, N do
			texts[i]:set(value)
		end
		r.flush()
		local updated = os.clock() - start
		expectWrites(before, N, "bridge")
		disposeBridge()
		if run > 0 then
			table.insert(creates, created)
			table.insert(updates, updated)
		end
	end
	row("create a bridge", "B-node", creates, 1e6 / N, "µs a label")
	row("update through it", "B-node", updates, 1e6 / N, "µs a label")
	unmountVide()
end
