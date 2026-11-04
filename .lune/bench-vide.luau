-- Times `useReadable` against a hand-written Vide source, and against the Crucible binding node it runs on with no
-- Vide at all. Each measurement makes `COUNT` bindings under one Vide root, each read by one Vide effect, the way a
-- component binds a property. The binding-only subject makes `COUNT` root bindings, each with a sink that counts.
--
--   create   makes the root and its bindings, then destroys the root. The baseline makes a Vide source from the
--            state's value; the bridge calls `useReadable(state)`.
--   update   writes a new value to each of `COUNT` states, then flushes. The baseline writes each Vide source
--            directly, which updates its effect at once; the bridge's effects update on the flush.
--   shared   writes one state that all `COUNT` bindings read, then flushes. The baseline is one Vide source read
--            by `COUNT` effects.
--
-- Vide's strict mode, which runs every effect twice below optimization level 2, is off. Build the runtime first with
-- `pnpm build`, then:
--
--   lune run bench-vide [runs]
local process = require("@lune/process")
local spec = require("../test/lib/load")
local realm = require("../test/lib/reactive-world")

local COUNT = 1000
local OPTIMIZATION = 2

local runs = tonumber(process.args[1]) or 15
assert(runs >= 7, "at least 7 runs")

spec.optimize(OPTIMIZATION)
local r = realm()
local vide = r.graph.package("@rbxts/vide/src")
vide.strict = false
local useReadable = r.graph.module("vide").useReadable
local state = r.api.state
local createRootBinding = r.reactive.createRootBinding
local retire = r.graph.module("internal/reactive/ownership").retire

local seen = 0
local function bind(read)
	vide.effect(function()
		seen += 1
		return read()
	end)
end

local function states(n: number)
	local list = table.create(n)
	for i = 1, n do
		list[i] = state(0)
	end
	return list
end

local function baselineCreate()
	local sources = states(COUNT)
	local start = os.clock()
	local destroy = vide.root(function()
		for i = 1, COUNT do
			bind(vide.source(sources[i]()))
		end
	end)
	destroy()
	return os.clock() - start
end

local function bridgeCreate()
	local sources = states(COUNT)
	local start = os.clock()
	local destroy = vide.root(function()
		for i = 1, COUNT do
			bind(useReadable(sources[i]))
		end
	end)
	r.flush()
	destroy()
	return os.clock() - start
end

local function count()
	seen += 1
end

local function bound(sources)
	local bindings = table.create(COUNT)
	for i = 1, COUNT do
		bindings[i] = createRootBinding(sources[i], sources[i](), count, nil)
	end
	return function()
		for i = 1, COUNT do
			retire(bindings[i])
		end
	end
end

local function bindingCreate()
	local sources = states(COUNT)
	local start = os.clock()
	local dispose = bound(sources)
	r.flush()
	dispose()
	return os.clock() - start
end

local function bindingUpdate()
	local sources = states(COUNT)
	local dispose = bound(sources)
	r.flush()
	seen = 0
	local start = os.clock()
	for i = 1, COUNT do
		sources[i]:set(1)
	end
	r.flush()
	local elapsed = os.clock() - start
	assert(seen == COUNT, `binding update: {seen} of {COUNT} sinks ran`)
	dispose()
	return elapsed
end

local function bindingShared()
	local shared = state(0)
	local sources = table.create(COUNT, shared)
	local dispose = bound(sources)
	r.flush()
	seen = 0
	local start = os.clock()
	shared:set(1)
	r.flush()
	local elapsed = os.clock() - start
	assert(seen == COUNT, `binding shared: {seen} of {COUNT} sinks ran`)
	dispose()
	return elapsed
end

local function baselineUpdate()
	local sources = table.create(COUNT)
	local destroy = vide.root(function()
		for i = 1, COUNT do
			sources[i] = vide.source(0)
			bind(sources[i])
		end
	end)
	seen = 0
	local start = os.clock()
	for i = 1, COUNT do
		sources[i](1)
	end
	local elapsed = os.clock() - start
	assert(seen == COUNT, `baseline update: {seen} of {COUNT} effects ran`)
	destroy()
	return elapsed
end

local function bridgeUpdate()
	local sources = states(COUNT)
	local destroy = vide.root(function()
		for i = 1, COUNT do
			bind(useReadable(sources[i]))
		end
	end)
	r.flush()
	seen = 0
	local start = os.clock()
	for i = 1, COUNT do
		sources[i]:set(1)
	end
	r.flush()
	local elapsed = os.clock() - start
	assert(seen == COUNT, `bridge update: {seen} of {COUNT} effects ran`)
	destroy()
	return elapsed
end

local function baselineShared()
	local shared
	local destroy = vide.root(function()
		shared = vide.source(0)
		for _ = 1, COUNT do
			bind(shared)
		end
	end)
	seen = 0
	local start = os.clock()
	shared(1)
	local elapsed = os.clock() - start
	assert(seen == COUNT, `baseline shared: {seen} of {COUNT} effects ran`)
	destroy()
	return elapsed
end

local function bridgeShared()
	local shared = state(0)
	local destroy = vide.root(function()
		for _ = 1, COUNT do
			bind(useReadable(shared))
		end
	end)
	r.flush()
	seen = 0
	local start = os.clock()
	shared:set(1)
	r.flush()
	local elapsed = os.clock() - start
	assert(seen == COUNT, `bridge shared: {seen} of {COUNT} effects ran`)
	destroy()
	return elapsed
end

local function median(samples: { number }): number
	table.sort(samples)
	local n = #samples
	return if n % 2 == 1 then samples[(n + 1) / 2] else (samples[n / 2] + samples[n / 2 + 1]) / 2
end

-- Runs every subject `runs` times after one untimed warm-up each, rotating which goes first.
local function measure(subjects)
	local samples = {}
	for i, subject in subjects do
		subject()
		samples[i] = {}
	end
	for run = 1, runs do
		for offset = 0, #subjects - 1 do
			local i = (run + offset - 1) % #subjects + 1
			table.insert(samples[i], subjects[i]())
		end
	end
	return samples
end

local function row(label: string, samples: { number })
	local middle = median(samples)
	print(
		string.format(
			"  %-12s %7.3f ms (%.3f to %.3f), %6.0f ns a binding",
			label,
			middle * 1e3,
			samples[1] * 1e3,
			samples[#samples] * 1e3,
			middle / COUNT * 1e9
		)
	)
end

print(`{_VERSION}, optimization level {OPTIMIZATION}, {COUNT} bindings, {runs} runs; median (min to max)`)
for _, case in
	{
		{ "create and destroy", baselineCreate, bindingCreate, bridgeCreate },
		{ "update, one state each", baselineUpdate, bindingUpdate, bridgeUpdate },
		{ "update, one shared state", baselineShared, bindingShared, bridgeShared },
	}
do
	local samples = measure({ case[2], case[3], case[4] })
	print(`\n{case[1]}`)
	row("Vide only", samples[1])
	row("binding only", samples[2])
	row("bridge", samples[3])
end
