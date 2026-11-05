-- Runs the compiled UI fixtures against the compiler-built runtime on the fake Instance. `<root>` holds the place's
-- folders: the runtime under ReplicatedStorage's rbxts_include, and the fixtures under StarterPlayerScripts.
local process = require("@lune/process")
local task = require("@lune/task")
local Engine = require("../../../../../test/lib/engine")
local fake = require("./instance")
local graph = require("./load")

local root = assert(process.args[1], "usage: ui.luau <root>")
local crucible = `{root}/ReplicatedStorage/rbxts_include/node_modules/@trioforge/crucible/out`
local client = `{root}/StarterPlayer/StarterPlayerScripts/TS`

local function world()
	local engine = Engine.new()
	local globals: { [string]: any } = { Instance = fake.Instance, typeof = fake.typeof, task = task }
	local modules = graph({}, globals)
	globals.game = {
		GetService = function(_, service: string)
			return modules.node(`{root}/{service}`)
		end,
	}
	modules.load(`{crucible}/internal/log-output`).installLoggingPort(engine.loggingPort)
	local flush = modules.load(`{crucible}/internal/reactive/flush`)
	flush.useReactivePort(engine.port)
	flush.releaseBarrier()
	return {
		state = modules.load(crucible).state,
		fixture = function(name: string)
			return modules.load(`{client}/{name}`)
		end,
		flush = engine.flush,
		logs = engine.logs,
	}
end

local function eq(actual: any, expected: any, label: string)
	if actual ~= expected then
		error(`{label}: expected {tostring(expected)}, got {tostring(actual)}`, 2)
	end
end

local function list(actual: { any }, expected: { any }, label: string)
	local a, e = table.concat(actual, " "), table.concat(expected, " ")
	if a ~= e then
		error(`{label}: expected {e}, got {a}`, 2)
	end
end

local function quiet(w)
	eq(#w.logs, 0, `no diagnostics ({w.logs[1] and w.logs[1].message})`)
end

local specs = {}

specs["a mount shows a reactive prop's value on the first frame, before any flush"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	w.fixture("counter").mountCounter(parent, w.state(3), function() end)
	local counter = fake.named(parent, "Counter")[1]
	eq(counter and counter.ClassName, "TextButton", "the component's instance is mounted")
	eq(counter.Text, "Coins 3", "the reactive Text has its value without a flush")
	eq(counter.TextSize, 18, "a fixed prop is set")
	quiet(w)
end

specs["an update reaches the instance on the next flush"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	local coins = w.state(3)
	w.fixture("counter").mountCounter(parent, coins, function() end)
	local counter = fake.named(parent, "Counter")[1]
	coins:set(5)
	eq(counter.Text, "Coins 3", "the write waits for the flush")
	w.flush()
	eq(counter.Text, "Coins 5", "the flush updates the bound prop")
	quiet(w)
end

specs["an event prop connects its handler"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	local bought = 0
	w.fixture("counter").mountCounter(parent, w.state(0), function()
		bought += 1
	end)
	fake.fire(fake.named(parent, "Counter")[1], "Activated")
	eq(bought, 1, "Activated runs the handler")
end

specs["an omitted optional prop is left at its default and a given one applies"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	local size, clicks = w.state(14), 0
	w.fixture("optional").mountButtons(parent, size, function()
		clicks += 1
	end)
	local buttons = fake.named(parent, "Button")
	eq(#buttons, 2, "both buttons mount")
	local bare, full = buttons[1], buttons[2]
	if bare.Text ~= nil then
		bare, full = full, bare
	end
	eq(bare.Text, nil, "an omitted Text is never assigned")
	eq(bare.TextSize, nil, "an omitted Readable is never bound")
	eq(full.Text, "Buy", "a given Text is set")
	eq(full.TextSize, 14, "a given Readable is bound")
	size:set(20)
	w.flush()
	eq(full.TextSize, 20, "the bound prop updates")
	fake.fire(bare, "Activated")
	fake.fire(full, "Activated")
	eq(clicks, 1, "only the given handler is connected")
	quiet(w)
end

specs["an unmount destroys the instances and ends their bindings and connections"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	local coins = w.state(3)
	local bought = 0
	local cleanup = w.fixture("counter").mountCounter(parent, coins, function()
		bought += 1
	end)
	local counter = fake.named(parent, "Counter")[1]
	cleanup()
	eq(fake.isDestroyed(counter), true, "the instance is destroyed")
	eq(#parent:GetChildren(), 0, "the parent is empty")
	coins:set(9)
	w.flush()
	eq(counter.Text, "Coins 3", "the binding ended")
	fake.fire(counter, "Activated")
	eq(bought, 0, "the connection ended")
	quiet(w)
end

specs["Show mounts its first branch at the call and swaps branches on a flush"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	local open, title = w.state(false), w.state("Hello")
	w.fixture("panel").mountPanel(parent, open, title)
	local panel = fake.named(parent, "Panel")[1]
	local closed = fake.named(panel, "Closed")[1]
	eq(closed and closed.Text, "closed", "the fallback shows on the first frame")
	eq(#fake.named(panel, "Open"), 0, "the open branch is not mounted")
	open:set(true)
	eq(#fake.named(panel, "Open"), 0, "the toggle waits for the flush")
	w.flush()
	local shown = fake.named(panel, "Open")[1]
	eq(shown and shown.Text, "Hello", "the open branch mounts with its bound value")
	eq(fake.isDestroyed(closed), true, "the fallback is destroyed")
	title:set("Bye")
	w.flush()
	eq(shown.Text, "Bye", "the branch's binding updates")
	open:set(false)
	w.flush()
	eq(fake.isDestroyed(shown), true, "the open branch is destroyed")
	title:set("Again")
	w.flush()
	eq(shown.Text, "Bye", "the ended branch's binding stopped")
	eq(#fake.named(panel, "Closed"), 1, "a new fallback mounts")
	quiet(w)
end

specs["Show that starts open shows its branch on the first frame"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	w.fixture("panel").mountPanel(parent, w.state(true), w.state("Hello"))
	local panel = fake.named(parent, "Panel")[1]
	eq(fake.named(panel, "Open")[1] and fake.named(panel, "Open")[1].Text, "Hello", "the branch has its value")
	eq(#fake.named(panel, "Closed"), 0, "no fallback")
	quiet(w)
end

specs["For keeps a row per item through a reorder, an insert and a removal"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	local items = w.state({ "a", "b", "c" })
	local cleanup = w.fixture("list").mountList(parent, items)
	local frame = fake.named(parent, "List")[1]
	eq(#fake.named(frame, "Layout"), 1, "an intrinsic child is parented")
	list(fake.texts(frame, "Row"), { "a", "b", "c" }, "rows on the first frame")
	list(fake.texts(frame, "Row", "LayoutOrder"), { 0, 1, 2 }, "indexes count from 0")
	local a, b, c = table.unpack(fake.named(frame, "Row"))

	items:set({ "c", "a", "b" })
	w.flush()
	list(fake.texts(frame, "Row"), { "c", "a", "b" }, "a reorder moves the rows")
	local rows = fake.named(frame, "Row")
	eq(rows[1] == c and rows[2] == a and rows[3] == b, true, "a reorder keeps each row's instance")

	items:set({ "c", "d", "a", "b" })
	w.flush()
	list(fake.texts(frame, "Row"), { "c", "d", "a", "b" }, "an insert mounts one row")
	list(fake.texts(frame, "Row", "LayoutOrder"), { 0, 1, 2, 3 }, "an insert shifts the later indexes")

	items:set({ "d", "b" })
	w.flush()
	list(fake.texts(frame, "Row"), { "d", "b" }, "a removal ends rows")
	eq(fake.isDestroyed(a) and fake.isDestroyed(c), true, "removed rows are destroyed")
	eq(fake.isDestroyed(b), false, "a kept row stays")

	cleanup()
	eq(fake.isDestroyed(b), true, "an unmount destroys the rows")
	quiet(w)
end

specs["a component receives its children, and a fragment mounts each element"] = function()
	local w = world()
	local parent = fake.new("ScreenGui")
	w.fixture("cards").mountCards(parent)
	local a = fake.named(parent, "A")[1]
	eq(a and a.ClassName, "Frame", "the first card mounts")
	eq(#fake.named(parent, "B"), 1, "the second card mounts")
	eq(fake.named(a, "Body")[1] and fake.named(a, "Body")[1].Text, "a", "the card appends its children")
	quiet(w)
end

local names = {}
for name in specs do
	table.insert(names, name)
end
table.sort(names)
local failures = 0
for _, name in names do
	local ok, message = pcall(specs[name])
	if not ok then
		failures += 1
		print(`FAIL {name}: {message}`)
	end
end
if failures > 0 then
	process.exit(1)
end
print(`ui specs pass ({#names})`)
