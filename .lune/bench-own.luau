-- Times `own` against @rbxts/janitor on the same values. The mixed set holds 10,000 values, 2,000 each of
-- connections, Instances, functions, threads and destroyable tables; each kind is then timed alone, 2,000 values at
-- a time. A measurement owns or `Add`s every value, then releases them all. Values are rebuilt untimed for each
-- one, the subjects rotate which goes first, and every release is checked. Instances have no parent: Lune's Destroy
-- costs more the more siblings an Instance has, and more in one order than the other. Build the runtime first with
-- `pnpm build`, unpack Janitor outside the repository, and pass its package folder:
--
--   npm pack @rbxts/janitor@1.18.3-ts.0 && tar xzf rbxts-janitor-1.18.3-ts.0.tgz
--   lune run bench-own <unpacked>/package [runs]
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local spec = require("../test/lib/load")
local realm = require("../test/lib/reactive-world")
local life = require("../test/lib/reactive-life")
local ownable = require("../test/lib/ownable")

local PER_KIND = 2000
-- Janitor's source asks for level 2, so the runtime gets the same.
local OPTIMIZATION = 2

local janitorDir = assert(process.args[1], "usage: lune run bench-own <janitor package folder> [runs]")
local runs = tonumber(process.args[2]) or 15
assert(runs >= 7, "at least 7 runs")

local function compile(path: string): string
	return luau.compile(fs.readFile(path), { optimizationLevel = OPTIMIZATION })
end

local function loadJanitor()
	local modules = {
		FastDefer = luau.load(compile(`{janitorDir}/src/FastDefer.luau`), {
			debugName = "FastDefer",
			environment = { task = task },
		})(),
		-- Only AddPromise reads it, and the benchmark never calls that.
		Promise = {},
	}
	return luau.load(compile(`{janitorDir}/src/init.luau`), {
		debugName = "Janitor",
		environment = {
			script = { FastDefer = "FastDefer", Promise = "Promise" },
			require = function(name)
				return modules[name]
			end,
			game = {
				Destroy = function(instance)
					instance:Destroy()
				end,
			},
			task = task,
			typeof = ownable.typeof,
		},
	})().Janitor
end

local released = 0
local function count()
	released += 1
end

local Lower = {}
Lower.__index = Lower
function Lower.destroy(_self)
	released += 1
end

local Upper = {}
Upper.__index = Upper
function Upper.Destroy(_self)
	released += 1
end

-- Each kind makes one value, the method name Janitor's `Add` is given for it (nil for its default), and whether
-- releasing it moves `released`. Tables alternate between `destroy()`, which Janitor needs named, and `Destroy()`.
local KINDS = {
	{
		name = "connections",
		make = function()
			return ownable.connection(count), nil, true
		end,
	},
	{
		name = "Instances",
		make = function()
			return roblox.Instance.new("Folder"), nil, false
		end,
	},
	{
		name = "functions",
		make = function()
			return function()
				released += 1
			end, nil, true
		end,
	},
	{
		name = "threads",
		make = function()
			return coroutine.create(count), nil, false
		end,
	},
	{
		name = "tables",
		make = function(i: number)
			if i % 2 == 0 then
				return setmetatable({}, Lower), "destroy", true
			end
			return setmetatable({}, Upper), nil, true
		end,
	},
}

local function build(kinds)
	local size = PER_KIND * #kinds
	local set = { size = size, values = table.create(size), methods = table.create(size), counted = 0, checks = {} }
	for i = 1, size do
		local value, method, counted = kinds[(i - 1) % #kinds + 1].make(i)
		set.values[i] = value
		set.methods[i] = method
		if counted then
			set.counted += 1
		else
			table.insert(set.checks, value)
		end
	end
	return set
end

local probe = roblox.Instance.new("Folder")

local function verify(set, label: string)
	assert(released == set.counted, `{label}: {released} of {set.counted} counted releases`)
	for _, value in set.checks do
		if type(value) == "thread" then
			assert(coroutine.status(value) == "dead", `{label}: a thread was not cancelled`)
		else
			local reparented = pcall(function()
				value.Parent = probe
			end)
			assert(not reparented, `{label}: an Instance was not destroyed`)
		end
	end
	released = 0
end

spec.optimize(OPTIMIZATION)
local Janitor = loadJanitor()
local r = realm(nil, ownable.globals)
local own, scope = r.api.own, r.api.scope

local w = life(ownable.globals)
local NONE = { size = 0, values = {} }
local pending = NONE
local holder = w.make("Holder", {
	onEnable = function()
		local values, ownHere = pending.values, w.api.own
		for i = 1, pending.size do
			ownHere(values[i])
		end
	end,
})
w.boot(holder)
w.life.request(holder, w.Want.Disabled)

local function scoped(set)
	local values, size = set.values, set.size
	local start = os.clock()
	local dispose = scope(function()
		for i = 1, size do
			own(values[i])
		end
	end)
	local owned = os.clock()
	dispose()
	return owned - start, os.clock() - owned
end

local function behaviour(set)
	pending = set
	local start = os.clock()
	w.life.request(holder, w.Want.Enabled)
	local owned = os.clock()
	w.life.request(holder, w.Want.Disabled)
	local finished = os.clock()
	pending = NONE
	return owned - start, finished - owned
end

local function janitor(set)
	local values, methods, size = set.values, set.methods, set.size
	local start = os.clock()
	local j = Janitor.new()
	for i = 1, size do
		j:Add(values[i], methods[i])
	end
	local owned = os.clock()
	j:Cleanup()
	return owned - start, os.clock() - owned
end

local function median(samples: { number }): number
	table.sort(samples)
	local n = #samples
	return if n % 2 == 1 then samples[(n + 1) / 2] else (samples[n / 2] + samples[n / 2 + 1]) / 2
end

-- Runs every subject `runs` times on fresh sets of `kinds`, after one untimed warm-up each.
local function measure(subjects, kinds)
	for _, subject in subjects do
		local set = build(kinds)
		subject.run(set)
		verify(set, `{subject.name} warm-up`)
		subject.hold, subject.release, subject.total = {}, {}, {}
	end
	for run = 1, runs do
		for offset = 0, #subjects - 1 do
			local subject = subjects[(run + offset - 1) % #subjects + 1]
			local set = build(kinds)
			local hold, release = subject.run(set)
			verify(set, `{subject.name} run {run}`)
			table.insert(subject.hold, hold)
			table.insert(subject.release, release)
			table.insert(subject.total, hold + release)
		end
	end
end

local function range(samples: { number }, size: number): string
	local middle = median(samples)
	return string.format(
		"%7.2f ms (%.2f to %.2f), %5.0f ns a value",
		middle * 1e3,
		samples[1] * 1e3,
		samples[#samples] * 1e3,
		middle / size * 1e9
	)
end

print(`{_VERSION}, optimization level {OPTIMIZATION}, {runs} runs; median (min to max)`)
local mixed = {
	{ name = "own in a scope, disposed", run = scoped },
	{ name = "own in onEnable, disabled", run = behaviour },
	{ name = "Janitor Add, Cleanup", run = janitor },
}
measure(mixed, KINDS)
print(`\n{PER_KIND * #KINDS} mixed values`)
for _, subject in mixed do
	print(subject.name)
	print(`  own or Add  {range(subject.hold, PER_KIND * #KINDS)}`)
	print(`  release     {range(subject.release, PER_KIND * #KINDS)}`)
	print(`  total       {range(subject.total, PER_KIND * #KINDS)}`)
end

print(`\n{PER_KIND} values of one kind, total ns a value: own in a scope / Janitor`)
for _, kind in KINDS do
	local alone = { { name = "own", run = scoped }, { name = "Janitor", run = janitor } }
	measure(alone, { kind })
	print(
		string.format(
			"  %-12s %6.0f / %6.0f",
			kind.name,
			median(alone[1].total) / PER_KIND * 1e9,
			median(alone[2].total) / PER_KIND * 1e9
		)
	)
end
