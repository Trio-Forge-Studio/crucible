-- Where head B's update time goes, interleaved with Vide's whole write. Not part of the reported bench.
local process = require("@lune/process")
local subjects = require("./lib/subjects")
local gc = require("./lib/gc")
subjects.roots.head = process.args[1]
local N = 1000
local RUNS = 31
local function median(t) table.sort(t) return t[(#t + 1) // 2] * 1e3 end
local ui = subjects.make("head B")
local r = ui.realm
local shared = ui.state(0)
local texts = {}
for i = 1, N do texts[i] = ui.state(i) end
local frame = ui.mountItems(texts, table.create(N, shared))
local vui = subjects.make("Vide")
local vshared = vui.state(0)
local vtexts = {}
for i = 1, N do vtexts[i] = vui.state(i) end
vui.mountItems(vtexts, table.create(N, vshared))
local nodes = {}
for o in shared.observers do table.insert(nodes, o) end
local ordered = table.clone(nodes)
table.sort(ordered, function(a, b) return a.id < b.id end)
local labels = frame:GetChildren()
local t = { w = {}, f = {}, w1 = {}, f1 = {}, sort = {}, check = {}, vsep = {}, vone = {}, bare = {}, guarded = {} }
local function timed(key, fn)
	gc.cycle()
	local s = os.clock()
	fn()
	table.insert(t[key], os.clock() - s)
end
local createdBefore = r.graph.createdBefore
local function set(i, v) i.Text = v end
for run = 1, RUNS do
	local v = run + 0.5
	timed("w", function() for i = 1, N do texts[i]:set(v) end end)
	timed("f", r.flush)
	timed("vsep", function() for i = 1, N do vtexts[i](v) end end)
	timed("w1", function() shared:set(v) end)
	timed("f1", r.flush)
	timed("vone", function() vshared(v) end)
	local copy = table.clone(nodes)
	timed("check", function() for i = 2, #ordered do if createdBefore(ordered[i], ordered[i - 1]) then break end end end)
	timed("sort", function()
		local ids, byId = {}, {}
		for i = 1, #copy do ids[i] = copy[i].id; byId[copy[i].id] = copy[i] end
		table.sort(ids)
		for i = 1, #copy do copy[i] = byId[ids[i]] end
	end)
	timed("bare", function() for i = 1, N do labels[i].Text = v end end)
	timed("guarded", function() for i = 1, N do pcall(set, labels[i], v) end end)
end
print(string.format("head separate: write %.2f + flush %.2f = %.2f | Vide %.2f", median(t.w), median(t.f), median(t.w) + median(t.f), median(t.vsep)))
print(string.format("head one:      write %.2f + flush %.2f = %.2f | Vide %.2f", median(t.w1), median(t.f1), median(t.w1) + median(t.f1), median(t.vone)))
print(string.format("pieces: in-order check %.2f, numeric sort %.2f, fake write %.2f, pcall'd fake write %.2f (µs a prop)", median(t.check), median(t.sort), median(t.bare), median(t.guarded)))
