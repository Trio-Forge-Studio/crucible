-- Checks that trunk B, head B and Vide render, update and clean up the same way before any of them is timed, plus
-- the guarantees the spike's binding keeps on a trunk runtime. Head's binding node has its own Lune specs in
-- test/reactive-bind.luau. Build both runtimes first, then from the repository root:
--
--   lune run spike/ui/check <trunk out> <head out>
local process = require("@lune/process")
local subjects = require("./lib/subjects")
local lists = require("./lib/lists")
local fake = require("./lib/instance")
local runtime = require("./lib/runtime")
local CrucibleUI = require("./lib/crucible-ui")
local Binding = require("./lib/binding")

subjects.roots.trunk = assert(process.args[1], "the trunk runtime's out folder")
subjects.roots.head = assert(process.args[2], "the head runtime's out folder")

local failures = 0
local function eq(actual, expected, label: string)
	if actual ~= expected then
		failures += 1
		print(`  FAIL {label}: expected {tostring(expected)}, got {tostring(actual)}`)
	end
end

local function labels(frame): string
	local out = {}
	for _, child in fake.ordered(frame) do
		table.insert(out, tostring(child.Text))
	end
	return table.concat(out, " ")
end

local function expected(list): string
	local out = {}
	for _, item in list do
		table.insert(out, tostring(item.label()))
	end
	return table.concat(out, " ")
end

local function count(t): number
	local n = 0
	for _ in t do
		n += 1
	end
	return n
end

for _, name in subjects.names do
	print(name)
	local ui = subjects.make(name)

	-- Items: initial values, one update each way, unmount.
	local texts, colors = {}, {}
	local shared = ui.state("red")
	for i = 1, 3 do
		texts[i] = ui.state(`t{i}`)
		colors[i] = shared
	end
	local frame, dispose = ui.mountItems(texts, colors)
	local items = frame:GetChildren()
	eq(#frame:GetChildren(), 3, "three items")
	eq(items[1].BackgroundTransparency, 1, "fixed prop")
	local writes = fake.writes
	ui.set(texts[2], "changed")
	ui.flush()
	eq(fake.writes - writes, 1, "one write for one changed state")
	local found = false
	for _, item in frame:GetChildren() do
		found = found or item.Text == "changed"
	end
	eq(found, true, "the changed text reached its label")
	ui.set(shared, "blue")
	ui.flush()
	for _, item in frame:GetChildren() do
		eq(item.TextColor3, "blue", "the shared colour reached every label")
	end
	local children = frame:GetChildren()
	dispose()
	ui.flush()
	eq(fake.isDestroyed(frame), true, "unmount destroys the root")
	eq(fake.isDestroyed(children[1]), true, "unmount destroys the items")
	if name ~= "Vide" then
		eq(count(texts[1].observers), 0, "unmount unsubscribes from a text state")
		eq(count(shared.observers), 0, "unmount unsubscribes from the shared state")
	end
	writes = fake.writes
	ui.set(shared, "green")
	ui.flush()
	eq(fake.writes - writes, 0, "no write after unmount")

	-- Show.
	local open = ui.state(false)
	local panel, closePanel = ui.mountShow(open, ui.state("hi"), ui.state("red"))
	eq(#panel:GetChildren(), 0, "closed show renders nothing")
	ui.set(open, true)
	ui.flush()
	eq(#panel:GetChildren(), 1, "open show renders its branch")
	local branch = panel:GetChildren()[1]
	eq(branch and branch.Text, "hi", "branch props")
	ui.set(open, false)
	ui.flush()
	eq(#panel:GetChildren(), 0, "closing removes the branch")
	if name ~= "Vide" then
		eq(fake.isDestroyed(branch), true, "closing destroys the branch")
	end
	closePanel()

	-- For: every operation keeps the rows in list order.
	local base = lists.items(ui, 20)
	local cases = {
		{ "reverse", lists.reverse(base) },
		{ "insert", lists.insertMiddle(base, lists.items(ui, 5, 100)) },
		{ "remove", lists.removeMiddle(base, 5) },
		{ "swap", lists.swap(base, 2, 19) },
	}
	for _, case in cases do
		local list = ui.state(base)
		local listFrame, closeList = ui.mountList(list)
		eq(labels(listFrame), expected(base), "initial rows")
		local before = {}
		for _, child in listFrame:GetChildren() do
			before[child] = true
		end
		ui.set(list, case[2])
		ui.flush()
		eq(labels(listFrame), expected(case[2]), `{case[1]} keeps list order`)
		local kept = 0
		for _, child in listFrame:GetChildren() do
			if before[child] then
				kept += 1
			end
		end
		local survivors = math.min(#base, #case[2])
		eq(kept, survivors, `{case[1]} reuses the rows of kept items`)
		local row = fake.ordered(listFrame)[1]
		ui.set(case[2][1].label, "relabelled")
		ui.flush()
		eq(row.Text, "relabelled", `{case[1]}: a row's label still tracks its item`)
		closeList()
		ui.flush()
		eq(fake.isDestroyed(listFrame), true, `{case[1]}: unmount destroys the list`)
	end
end

print("trunk B binding guarantees")
do
	local r = runtime.crucible({ root = subjects.roots.trunk, patch = true })
	local ui = CrucibleUI(r, "B")
	local binding = Binding(r)
	local api = r.api

	-- A binding through a computed writes the computed's value, once per flush, in Crucible's flush.
	local a, b = api.state(1), api.state(2)
	local sum = binding.computedNode(function()
		return a() + b()
	end)
	local label = fake.new("TextLabel")
	local dispose = api.scope(function()
		binding.bind(label, "Text", sum)
	end)
	eq(label.Text, 3, "initial computed value at creation")
	a:set(10)
	b:set(20)
	eq(label.Text, 3, "no write before the flush")
	r.flush()
	eq(label.Text, 30, "one flush writes the new value")

	-- A computed that throws stops the binding with a diagnostic; the rest of the flush still runs.
	local other = fake.new("TextLabel")
	api.scope(function()
		binding.bind(other, "Text", a)
	end)
	local logged = #r.engine.logs
	b:set(error)
	a:set(5)
	r.flush()
	eq(#r.engine.logs > logged, true, "a failed source is reported")
	eq(other.Text, 5, "a sibling binding still updated")
	b:set(1)
	r.flush()
	eq(label.Text, 30, "a stopped binding stays stopped")

	-- A computed that yields is cached as a failure by the computed, not by the binding.
	local yields = binding.computedNode(function()
		coroutine.yield()
		return 1
	end)
	local yielded = fake.new("TextLabel")
	local ok = pcall(api.scope, function()
		binding.bind(yielded, "Text", yields)
	end)
	eq(ok, false, "creating a binding on a yielding computed throws to the setup")

	-- A property write that throws stops that binding only.
	local strict = fake.new("TextLabel")
	local value = api.state(1)
	api.scope(function()
		binding.bind(strict, "Text", value)
	end)
	getmetatable(strict).__newindex = function()
		error("bad property")
	end
	value:set(2)
	r.flush()
	eq(count(value.observers), 0, "a binding whose write threw unsubscribed")

	-- Ending the owner unsubscribes.
	dispose()
	eq(count(a.observers), 1, "only the other binding still reads a")
	ui.flush()
end

if failures > 0 then
	error(`{failures} checks failed`)
end
print("all checks passed")
