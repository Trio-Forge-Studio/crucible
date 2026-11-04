-- Splits one runtime's binding update into write and flush, next to Vide's whole write, plus the pieces inside the
-- flush. Not part of the reported bench. lune run spike/ui/profile2 <runtime out>
local process = require("@lune/process")
local subjects = require("./lib/subjects")
local gc = require("./lib/gc")
subjects.roots.head = process.args[1]
local N = 1000
local RUNS = 31
local function median(t)
	table.sort(t)
	return t[(#t + 1) // 2] * 1e3
end
local ui = subjects.make("head B")
local r = ui.realm
local shared = ui.state(0)
local texts = {}
for i = 1, N do
	texts[i] = ui.state(i)
end
local frame = ui.mountItems(texts, table.create(N, shared))
local vui = subjects.make("Vide")
local vshared = vui.state(0)
local vtexts = {}
for i = 1, N do
	vtexts[i] = vui.state(i)
end
vui.mountItems(vtexts, table.create(N, vshared))
local labels = frame:GetChildren()
local t = { w = {}, f = {}, w1 = {}, f1 = {}, vsep = {}, vone = {}, bare = {}, guarded = {}, call = {} }
local function timed(key, fn)
	gc.cycle()
	local s = os.clock()
	fn()
	table.insert(t[key], os.clock() - s)
end
local function set(i, v)
	i.Text = v
end
for run = 1, RUNS do
	local v = run + 0.5
	timed("w", function()
		for i = 1, N do
			texts[i]:set(v)
		end
	end)
	timed("f", r.flush)
	timed("vsep", function()
		for i = 1, N do
			vtexts[i](v)
		end
	end)
	timed("w1", function()
		shared:set(v)
	end)
	timed("f1", r.flush)
	timed("vone", function()
		vshared(v)
	end)
	timed("bare", function()
		for i = 1, N do
			labels[i].Text = v
		end
	end)
	timed("call", function()
		for i = 1, N do
			set(labels[i], v)
		end
	end)
	timed("guarded", function()
		for i = 1, N do
			pcall(set, labels[i], v)
		end
	end)
end
print(string.format("separate: write %.2f + flush %.2f = %.2f | Vide %.2f", median(t.w), median(t.f), median(t.w) + median(t.f), median(t.vsep)))
print(string.format("one:      write %.2f + flush %.2f = %.2f | Vide %.2f", median(t.w1), median(t.f1), median(t.w1) + median(t.f1), median(t.vone)))
print(string.format("fake write %.2f, called %.2f, pcall'd %.2f (µs a prop)", median(t.bare), median(t.call), median(t.guarded)))
