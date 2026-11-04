-- A lifetime's releases run as one batch on a shared worker. These pin what each release still gets on its own: its
-- lifecycle requests, its owner and frame after a neighbour failed, its own blame, and nested releases.
local spec = require("./lib/load")
local realm = require("./lib/reactive-world")
local life = require("./lib/reactive-life")

local function logger(events)
	return function(label)
		return function()
			table.insert(events, label)
		end
	end
end

local function messages(engine, part: string): { string }
	local found = {}
	for _, entry in engine.logs do
		if string.find(entry.message, part, 1, true) then
			table.insert(found, entry.message)
		end
	end
	return found
end

return {
	["rule 28 outside a drain, each release's lifecycle request applies before the next release runs"] = function()
		local w = life()
		local other = w.make("Other")
		w.boot(other)
		local dispose = w.api.scope(function()
			w.api.onCleanup(function()
				w.log("older")
			end)
			w.api.onCleanup(function()
				w.life.request(other, w.Want.Disabled)
				w.log("requests")
			end)
			w.api.onCleanup(function()
				w.log("newest")
			end)
		end)
		local mark = #w.events
		dispose()
		spec.list(w.since(mark), { "newest", "requests", "onDisable:Other", "older" }, "drained between releases")
	end,
	["rule 28 after a release throws or yields, the next one still runs as its lifetime's owner"] = function()
		local w = life()
		local holder
		holder = w.make("Holder", {
			onEnable = function()
				w.api.onCleanup(function()
					w.api.onCleanup(function()
						w.log("oldest: registered at once")
					end)
					w.log(`oldest: owned by {if w.life.current() == holder then "Holder" else "another"}`)
				end)
				w.api.onCleanup(function()
					coroutine.yield()
				end)
				w.api.onCleanup(function()
					w.api.onCleanup(function()
						w.log("middle: registered at once")
					end)
					w.log(`middle: owned by {if w.life.current() == holder then "Holder" else "another"}`)
				end)
				w.api.onCleanup(function()
					error("newest failed")
				end)
			end,
		})
		w.boot(holder)
		local mark = #w.events
		w.life.request(holder, w.Want.Disabled)
		spec.list(w.since(mark), {
			"onDisable:Holder",
			"middle: registered at once",
			"middle: owned by Holder",
			"oldest: registered at once",
			"oldest: owned by Holder",
		}, "each later release owns into the ended interval")
		spec.eq(w.engine.logged("newest failed"), 1, "the throw reported once")
		spec.eq(w.engine.logged("yielded"), 1, "the yield reported once")
		spec.eq(w.engine.logged("(Holder) failed"), 2, "both blamed on the Holder")
	end,
	["rule 28 a failure in a batch is reported against the release that failed"] = function()
		local r = realm()
		local first = function()
			error("first failure")
		end
		local fine = function() end
		local second = function()
			error("second failure")
		end
		r.root(function()
			r.api.onCleanup(second)
			r.api.onCleanup(fine)
			r.api.onCleanup(first)
		end)()
		local function origin(callback)
			local source, line = debug.info(callback, "sl")
			return `cleanup at {source}:{line} (a root scope) failed`
		end
		local firstReports = messages(r.engine, "first failure")
		local secondReports = messages(r.engine, "second failure")
		spec.eq(#firstReports, 1, "the first failure reported once")
		spec.eq(#secondReports, 1, "the second failure reported once")
		spec.has(firstReports[1], origin(first), "named by the function that threw")
		spec.has(secondReports[1], origin(second), "named by the function that threw")
		spec.eq(r.logged("cleanup"), 2, "the release between them was not reported")
	end,
	["rule 28 a release that ends a scope or registers during a batch releases inside its call"] = function()
		local r = realm()
		local events = {}
		local log = logger(events)
		local inner = r.root(function()
			r.api.onCleanup(log("inner older"))
			r.api.onCleanup(function()
				error("inner failed")
			end)
			r.api.onCleanup(log("inner newer"))
		end)
		r.root(function()
			r.api.onCleanup(log("outer older"))
			r.api.onCleanup(function()
				table.insert(events, "outer start")
				inner()
				r.api.onCleanup(log("registered at once"))
				r.api.own(function()
					error("owned late failed")
				end)
				table.insert(events, "outer end")
			end)
		end)()
		spec.list(events, {
			"outer start",
			"inner newer",
			"inner older",
			"registered at once",
			"outer end",
			"outer older",
		}, "nested releases ran inside the release that caused them")
		spec.eq(r.logged("inner failed"), 1, "the nested failure reported")
		spec.eq(r.logged("owned late failed"), 1, "the late own's failure reported")
		spec.eq(r.logged("cleanup"), 2, "nothing else reported")
	end,
	["rule 12 rule 21 a pooled worker forgets a returned body's frame before its next callback"] = function()
		local w = life()
		local door = w.make("Door")
		w.boot(door)
		local dispose = w.api.scope(function() end)
		-- The runner's pool hands out the worker the setup just returned.
		w.life.within(door, door, w.Hook.Frame, function()
			w.api.onCleanup(function()
				w.log("frame callback's cleanup")
			end)
		end)
		local mark = #w.events
		dispose()
		spec.list(w.since(mark), {}, "the callback did not own into the disposed scope")
		w.life.request(door, w.Want.Destroyed)
		spec.eq(table.find(w.since(mark), "frame callback's cleanup") ~= nil, true, "it owned into the Door")
	end,
	["rule 13 rule 28 a release that yields inside a computed leaves it finished for the next release"] = function()
		local r = realm()
		local x = r.api.state(0)
		local c = r.api.computed(function()
			if x() == 0 then
				coroutine.yield()
			end
			return x()
		end)
		local seen
		r.root(function()
			r.api.onCleanup(function()
				local ok, err = pcall(c)
				seen = if ok then "a value" else tostring(err)
			end)
			r.api.onCleanup(function()
				c()
			end)
		end)()
		spec.eq(r.logged("yielded"), 1, "the yielding release reported")
		spec.has(seen, "yielded", "the older release read the computed's cached yield")
		spec.eq(string.find(seen, "reads itself", 1, true), nil, "not a cycle left by an open evaluation")
		x:set(1)
		spec.eq(c(), 1, "the computed recovers")
	end,
}
