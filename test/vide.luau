local spec = require("./lib/load")
local realm = require("./lib/reactive-world")
local World = require("./lib/sync-world")

-- Vide's graph module, with the strict mode it enables below optimization level 2 turned off. Strict mode runs
-- every Vide effect twice, which would double each value a spec records.
local function loadVide(graph)
	local vide = graph.package("@rbxts/vide/src")
	vide.strict = false
	return vide
end

-- A reactive realm with Vide and the bridge loaded in its graph, so the bridge and the spec share one Vide.
local function world()
	local r = realm()
	r.vide = loadVide(r.graph)
	r.useReadable = r.graph.module("vide").useReadable
	return r
end

local function observers(readable): number
	local count = 0
	for _ in readable.observers do
		count += 1
	end
	return count
end

-- Mounts `component` under a Vide root and records every value a Vide effect reads from the source it returns.
local function mount(r, component)
	local seen = {}
	local read
	local destroy = r.vide.root(function()
		read = component()
		r.vide.effect(function()
			table.insert(seen, read())
		end)
	end)
	return read, seen, destroy
end

return {
	["the first value is read at the call, before any flush"] = function()
		local r = world()
		local coins = r.api.state(3)
		local read, seen = mount(r, function()
			return r.useReadable(coins)
		end)
		spec.eq(read(), 3, "the source holds the value at the call")
		spec.list(seen, { 3 }, "the first Vide run saw it")
	end,
	["a write reaches Vide on the next flush, not at the write"] = function()
		local r = world()
		local coins = r.api.state(3)
		local read, seen = mount(r, function()
			return r.useReadable(coins)
		end)
		coins:set(5)
		spec.eq(read(), 3, "the source is unchanged until the flush")
		r.flush()
		spec.eq(read(), 5, "the flush updated the source")
		spec.list(seen, { 3, 5 }, "the Vide effect ran once more")
	end,
	["several writes before a flush reach Vide once, and a write undone before it not at all"] = function()
		local r = world()
		local coins = r.api.state(0)
		local _, seen = mount(r, function()
			return r.useReadable(coins)
		end)
		coins:set(1)
		coins:set(2)
		coins:set(3)
		r.flush()
		spec.list(seen, { 0, 3 }, "one update with the last value")
		coins:set(4)
		coins:set(3)
		r.flush()
		spec.list(seen, { 0, 3 }, "no update for a value that came back")
	end,
	["destroying the Vide owner ends the watch"] = function()
		local r = world()
		local coins = r.api.state(1)
		local reads = 0
		local read, seen, destroy = mount(r, function()
			return r.useReadable(function()
				reads += 1
				return coins()
			end)
		end)
		r.flush()
		spec.eq(observers(coins), 1, "the watch subscribes to the state")
		destroy()
		spec.eq(observers(coins), 0, "no watch is left on the state")
		local before = reads
		coins:set(2)
		r.flush()
		spec.eq(reads, before, "the source is not read again")
		spec.eq(read(), 1, "the Vide source keeps its last value")
		spec.list(seen, { 1 }, "no Vide update after the unmount")
	end,
	["a component unmounted by its parent ends its watch"] = function()
		local r = world()
		local open = r.api.state(true)
		local coins = r.api.state(1)
		local inner
		r.vide.root(function()
			local shown = r.useReadable(open)
			r.vide.show(shown, function()
				inner = r.useReadable(coins)
				return {}
			end)
		end)
		spec.eq(inner(), 1, "the inner component mounted")
		r.flush()
		spec.eq(observers(coins), 1, "the inner watch runs")
		open:set(false)
		r.flush()
		spec.eq(observers(coins), 0, "hiding the component ended its watch")
	end,
	["a component mounted by a Crucible update keeps its watch past the next update"] = function()
		local r = world()
		local open = r.api.state(false)
		local coins = r.api.state(1)
		local inner
		r.vide.root(function()
			local shown = r.useReadable(open)
			r.vide.show(shown, function()
				inner = r.useReadable(coins)
				return {}
			end)
		end)
		open:set(true)
		r.flush()
		spec.eq(inner(), 1, "the update mounted the inner component")
		coins:set(2)
		r.flush()
		spec.eq(inner(), 2, "its watch saw a later write")
		coins:set(3)
		r.flush()
		spec.eq(inner(), 3, "and the one after")
	end,
	["the call is untracked and its watch outlives the Crucible owner around it"] = function()
		local r = world()
		local coins = r.api.state(1)
		local rerun = r.api.state(0)
		local runs = 0
		local reads = {}
		r.root(function()
			r.api.effect(function()
				rerun()
				runs += 1
				r.vide.root(function()
					table.insert(reads, r.useReadable(coins))
				end)
			end)
		end)
		r.flush()
		coins:set(2)
		r.flush()
		spec.eq(runs, 1, "the effect did not track the read")
		spec.eq(reads[1](), 2, "the binding updated")
		rerun:set(1)
		r.flush()
		coins:set(3)
		r.flush()
		spec.eq(reads[1](), 3, "the first binding outlived the effect run that created it")
		spec.eq(reads[2](), 3, "and the second one updates")
	end,
	["outside a Vide owner it throws and creates nothing"] = function()
		local r = world()
		local coins = r.api.state(1)
		local ok, err = pcall(r.useReadable, coins)
		spec.eq(ok, false, "the call threw")
		spec.has(err, "useReadable() needs a Vide owner: call it inside a component or root()", "the message")
		spec.eq(#r.engine.deferred, 0, "no flush was requested")
		coins:set(2)
		r.flush()
		spec.eq(observers(coins), 0, "no watch was made")
	end,
	["a computed source updates on the flush and an equal result does not reach Vide"] = function()
		local r = world()
		local price = r.api.state(4)
		local quantity = r.api.state(3)
		local total = r.api.computed(function()
			return price() * quantity()
		end)
		local read, seen = mount(r, function()
			return r.useReadable(total)
		end)
		spec.eq(read(), 12, "the first value")
		price:set(5)
		r.flush()
		spec.eq(read(), 15, "the computed value after the flush")
		price:set(3)
		quantity:set(5)
		r.flush()
		spec.list(seen, { 12, 15 }, "an equal result made no update")
	end,
	["a source that throws at the call throws from the call and leaves nothing"] = function()
		local r = world()
		local broken = r.api.state(false)
		local ok, err = pcall(r.vide.root, function()
			r.useReadable(function()
				if broken() == false then
					error("no value yet")
				end
				return 1
			end)
		end)
		spec.eq(ok, false, "the root failed")
		spec.has(err, "no value yet", "with the source's error")
		r.flush()
		spec.eq(observers(broken), 0, "no watch was made")
	end,
	["a source that throws on a flush stops its watch, reported once, and the value stays"] = function()
		local r = world()
		local coins = r.api.state(1)
		local read = mount(r, function()
			return r.useReadable(function()
				if coins() < 0 then
					error("negative coins")
				end
				return coins()
			end)
		end)
		coins:set(-1)
		r.flush()
		spec.eq(read(), 1, "the last good value stays")
		spec.eq(r.logged("negative coins"), 1, "the failure is reported once")
		coins:set(5)
		r.flush()
		spec.eq(read(), 1, "the stopped watch makes no update")
		spec.eq(observers(coins), 0, "and holds no subscription")
	end,
	["a Vide effect that throws on an update stops that binding's watch, reported once"] = function()
		local r = world()
		local coins = r.api.state(1)
		local read
		r.vide.root(function()
			read = r.useReadable(coins)
			r.vide.effect(function()
				if read() < 0 then
					error("cannot show negative coins")
				end
			end)
		end)
		coins:set(-1)
		r.flush()
		spec.eq(r.logged("cannot show negative coins"), 1, "the failure is reported once")
		coins:set(5)
		r.flush()
		spec.eq(read(), -1, "the stopped watch makes no further update")
		spec.eq(observers(coins), 0, "and holds no subscription")
	end,
	["a getter is read once at the call"] = function()
		local r = world()
		local coins = r.api.state(1)
		local reads = 0
		local read, seen = mount(r, function()
			return r.useReadable(function()
				reads += 1
				return coins()
			end)
		end)
		spec.eq(reads, 1, "one read at the call")
		r.flush()
		spec.eq(reads, 2, "and one when the watch first runs")
		spec.eq(read(), 1, "the value")
		spec.list(seen, { 1 }, "an unchanged value made no update")
	end,
	["a getter that builds a new table updates Vide on the first flush"] = function()
		local r = world()
		local coins = r.api.state(1)
		local _, seen = mount(r, function()
			return r.useReadable(function()
				return { coins() }
			end)
		end)
		r.flush()
		spec.eq(#seen, 2, "the first flush's table is a new value")
		r.flush()
		spec.eq(#seen, 2, "no update while nothing it reads changes")
	end,
	["a Vide effect that yields on an update stops that binding's watch and leaves Vide's scopes balanced"] = function()
		local r = world()
		local scopes = r.graph.package("@rbxts/vide/src/graph").scopes
		local coins = r.api.state(1)
		local gems = r.api.state(1)
		local read, other
		r.vide.root(function()
			read = r.useReadable(coins)
			other = r.useReadable(gems)
			r.vide.effect(function()
				if read() == 2 then
					coroutine.yield()
				end
			end)
		end)
		coins:set(2)
		r.flush()
		spec.eq(scopes.n, 0, "Vide's scope stack is empty again")
		spec.eq(r.logged("attempt to yield"), 1, "the yield is reported once")
		spec.eq(pcall(r.useReadable, coins), false, "useReadable outside an owner still throws")
		coins:set(3)
		gems:set(2)
		r.flush()
		spec.eq(read(), 2, "the stopped watch makes no further update")
		spec.eq(other(), 2, "another binding still updates")
	end,
	["in Vide's strict mode a yielding Vide effect stops the watch the same way"] = function()
		local r = world()
		r.vide.strict = true
		local scopes = r.graph.package("@rbxts/vide/src/graph").scopes
		local coins = r.api.state(1)
		r.vide.root(function()
			local read = r.useReadable(coins)
			r.vide.effect(function()
				if read() == 2 then
					coroutine.yield()
				end
			end)
		end)
		coins:set(2)
		r.flush()
		spec.eq(scopes.n, 0, "Vide's scope stack is empty again")
		spec.eq(r.logged("attempt to yield"), 1, "the yield is reported once")
		spec.eq(observers(coins), 0, "the watch stopped")
	end,
	["a replicated declaration updates its client binding after each applied frame"] = function()
		local w = World.new({
			declare = function(r)
				return { Score = r.emit.replicated("Job.Score", 0, r.n.f64) }
			end,
			serve = function(s, d, syncWorld)
				s.service("JobService", function()
					syncWorld.score = s.entry.publish(d.Score)
				end)
			end,
		})
		local ann = w:join("Ann")
		local vide = loadVide(ann.graph)
		local useReadable = ann.graph.module("vide").useReadable
		local seen = {}
		vide.root(function()
			local score = useReadable(ann.d.Score)
			vide.effect(function()
				table.insert(seen, score())
			end)
		end)
		w.link:flush()
		w.score:set(5)
		w:heartbeat()
		w.score:set(6)
		w:heartbeat()
		spec.list(seen, { 0, 5, 6 }, "one update per applied frame")
		w:finish()
	end,
}
