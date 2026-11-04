-- Interleaved update timing of head-style runtimes at several roots, plus Vide. Not part of the reported bench.
local process = require("@lune/process")
local subjects = require("./lib/subjects")
local fake = require("./lib/instance")
local gc = require("./lib/gc")
local N = 1000
local RUNS = 31
local roots = { table.unpack(process.args) }
local setups = {}
local names = {}
local function setup(name, ui)
	local shared = ui.state(0)
	local texts = {}
	for i = 1, N do texts[i] = ui.state(i) end
	ui.mountItems(texts, table.create(N, shared))
	setups[name] = { ui = ui, shared = shared, texts = texts, sep = {}, one = {} }
	table.insert(names, name)
end
for _, root in roots do
	subjects.roots.head = root
	setup(string.match(root, "[^/\]+$"), subjects.make("head B"))
end
setup("Vide", subjects.make("Vide"))
local value = 0.5
for run = 0, RUNS do
	for offset = 0, #names - 1 do
		local s = setups[names[(run + offset) % #names + 1]]
		value += 1
		gc.cycle()
		local before = fake.writes
		local t = os.clock()
		for i = 1, N do s.ui.set(s.texts[i], value) end
		s.ui.flush()
		local sep = os.clock() - t
		gc.cycle()
		t = os.clock()
		s.ui.set(s.shared, value)
		s.ui.flush()
		local one = os.clock() - t
		assert(fake.writes - before >= 2 * N, "every prop written")
		if run > 0 then table.insert(s.sep, sep); table.insert(s.one, one) end
	end
end
local function med(t) table.sort(t) return t[(#t + 1) // 2] * 1e3 end
for _, name in names do
	local s = setups[name]
	print(string.format("%-10s separate %.2f (%.2f-%.2f)  one %.2f (%.2f-%.2f) µs/prop", name, med(s.sep), s.sep[1]*1e3, s.sep[#s.sep]*1e3, med(s.one), s.one[1]*1e3, s.one[#s.one]*1e3))
end
