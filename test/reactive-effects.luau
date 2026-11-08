local spec = require("./lib/load")
local realm = require("./lib/reactive-world")

return {
	["rule 14 T1 an effect runs at creation and inside each write that changes what it read"] = function()
		local r = realm()
		local count = r.charm.atom(0)
		local doubled = r.charm.computed(function()
			return count() * 2
		end)
		local printed = {}
		r.root(function()
			r.api.effect(function()
				table.insert(printed, doubled())
			end)
		end)
		spec.list(printed, { 0 }, "run once inside scope()")
		count(1)
		spec.list(printed, { 0, 2 }, "run inside the write")
		count(1)
		spec.list(printed, { 0, 2 }, "equal write: nothing")
		count(2)
		spec.list(printed, { 0, 2, 4 }, "validated the computed, ran once")
	end,
	["rule 31 T12 writes made in one batch reach an effect as one run with every new value"] = function()
		local r = realm()
		local phase, score, payout = r.charm.atom("Waiting"), r.charm.atom(0), r.charm.atom(0)
		local seen = {}
		r.root(function()
			r.api.effect(function()
				table.insert(seen, `{phase()} {score()} {payout()}`)
			end)
		end)
		r.charm.batch(function()
			phase("Running")
			score(3)
			payout(30)
		end)
		spec.list(seen, { "Waiting 0 0", "Running 3 30" }, "no run observed part of the commit")
	end,
	["rule 14 dependencies are rebuilt every run"] = function()
		local r = realm()
		local flag, a, b = r.charm.atom(true), r.charm.atom(1), r.charm.atom(1)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				runs += 1
				if flag() then
					a()
				else
					b()
				end
			end)
		end)
		r.flush()
		b(2)
		r.flush()
		spec.eq(runs, 1, "b not read yet")
		flag(false)
		r.flush()
		a(2)
		r.flush()
		spec.eq(runs, 2, "a no longer read")
		b(3)
		r.flush()
		spec.eq(runs, 3, "b read now")
	end,
	["rule 15 T14 watch fires once with the new value and the baseline taken at the call"] = function()
		local r = realm()
		local source = r.charm.atom("a")
		local calls = {}
		r.root(function()
			r.api.watch(source, function(value, previous)
				table.insert(calls, `{previous}>{value}`)
			end)
		end)
		source("b")
		r.flush()
		r.flush()
		spec.list(calls, { "a>b" }, "a change before the first flush fires, once")
	end,
	["rule 15 watch coalesces per batch"] = function()
		local r = realm()
		local source = r.charm.atom(1)
		local calls = {}
		r.root(function()
			r.api.watch(source, function(value, previous)
				table.insert(calls, `{previous}>{value}`)
			end)
		end)
		r.charm.batch(function()
			source(2)
			source(1)
		end)
		spec.list(calls, {}, "changed and changed back in one batch: no call")
		r.charm.batch(function()
			source(2)
			source(3)
			source(4)
		end)
		spec.list(calls, { "1>4" }, "three writes: one call with the first and last values")
		source(1)
		spec.list(calls, { "1>4", "4>1" }, "previous moves with each call")
	end,
	["rule 15 a source that throws at the watch call is rethrown and creates nothing"] = function()
		local r = realm()
		local ran = false
		local ok, err = pcall(r.root, function()
			r.api.watch(function()
				error("no baseline")
			end, function()
				ran = true
			end)
		end)
		spec.eq(ok, false, "rethrown through the scope")
		spec.has(err, "no baseline", "the source's error")
		spec.eq(ran, false, "nothing ran")
	end,
	["rule 15 what onChange creates belongs to that run"] = function()
		local r = realm()
		local source = r.charm.atom(1)
		local events = {}
		r.root(function()
			r.api.watch(source, function(value)
				r.api.onCleanup(function()
					table.insert(events, `release {value}`)
				end)
				r.api.effect(function()
					table.insert(events, `child {value}`)
				end)
			end)
		end)
		source(2)
		r.flush()
		source(3)
		r.flush()
		spec.list(events, { "child 2", "release 2", "child 3" }, "the next run ended the previous run's work")
	end,
	["rule 15 T14 a waiter resumed by its watch disposes the watch mid-run"] = function()
		local r = realm()
		local phase = r.charm.atom(nil)
		local ready = r.charm.computed(function()
			return phase() ~= nil
		end)
		local calls, resumed = {}, false
		local waiter
		waiter = coroutine.create(function()
			if ready() then
				return
			end
			local dispose = r.api.scope(function()
				r.api.watch(ready, function(value, previous)
					table.insert(calls, `{previous}>{value}`)
					coroutine.resume(waiter)
				end)
			end)
			coroutine.yield()
			resumed = true
			dispose()
		end)
		coroutine.resume(waiter)
		phase("Waiting")
		r.flush()
		spec.list(calls, { "false>true" }, "resume(true, false)")
		spec.eq(resumed, true, "the waiter was resumed")
		phase(nil)
		r.flush()
		spec.list(calls, { "false>true" }, "the disposed watch never runs again")
	end,
	["rule 17 a run that returns neither nil nor a function throws to the code that wrote"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				runs += 1
				if s() == 2 then
					return true :: any
				end
				return nil
			end)
		end)
		local ok, err = pcall(s, 2)
		spec.eq(ok, false, "the write threw")
		spec.has(err, "returned a boolean", "naming the return")
		s(3)
		spec.eq(runs, 3, "the effect still runs on the next change")
		local created, first = pcall(r.root, function()
			r.api.effect(function()
				return 1 :: any
			end)
		end)
		spec.eq(created, false, "a first run's bad return throws at the effect call")
		spec.has(first, "returned a number", "naming that return")
	end,
	["rule 18 a failing run throws to the writer and keeps what it acquired until its next run"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events = {}
		r.root(function()
			r.api.effect(function()
				local n = s()
				r.api.onCleanup(function()
					table.insert(events, `release {n}`)
				end)
				table.insert(events, `run {n}`)
				if n == 2 then
					error("run failed")
				end
			end)
			r.api.effect(function()
				table.insert(events, `other {s()}`)
			end)
		end)
		local ok, err = pcall(s, 2)
		spec.eq(ok, false, "the write threw")
		spec.has(err, "run failed", "the run's error")
		spec.list(events, { "run 1", "other 1", "release 1", "run 2" }, "the failed run's release is held")
		table.clear(events)
		s(3)
		spec.eq(events[1], "release 2", "released before the next run")
		spec.eq(events[2], "run 3", "the failed effect runs again")
		spec.eq(events[#events], "other 3", "and so does the other")
	end,
	["rule 18 a yielding run is cancelled at the yield and thrown to the writer"] = function()
		local r = realm()
		local runs = 0
		local s = r.charm.atom(1)
		r.root(function()
			r.api.effect(function()
				runs += 1
				if s() == 2 then
					coroutine.yield()
					runs += 100
				end
			end)
		end)
		local ok, err = pcall(s, 2)
		spec.eq(ok, false, "the write threw")
		spec.has(err, "yielded", "naming the yield")
		spec.eq(runs, 2, "cancelled at the yield")
	end,
	["rule 19 a computed that throws throws into the run that reads it"] = function()
		local r = realm()
		local c = r.charm.computed(function()
			error("bad input")
		end)
		local after = false
		local ok, err = pcall(r.root, function()
			r.api.effect(function()
				c()
				after = true
			end)
		end)
		spec.eq(after, false, "the read threw into the run")
		spec.eq(ok, false, "and the first run threw at the effect call")
		spec.has(err, "bad input", "with the computed's error")
	end,
	["rule 26 T10 a rerun ends the run's scopes, then its releases newest first, then its child effects after the rerun"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events = {}
		r.root(function()
			r.api.effect(function()
				local n = s()
				r.api.effect(function()
					r.api.onCleanup(function()
						table.insert(events, `F release {n}`)
					end)
					return function()
						table.insert(events, `F cleanup {n}`)
					end
				end)
				r.api.onCleanup(function()
					table.insert(events, `R1 {n}`)
				end)
				r.api.scope(function()
					r.api.onCleanup(function()
						table.insert(events, `S release {n}`)
					end)
				end)
				table.insert(events, `run {n}`)
				return function()
					table.insert(events, `R2 {n}`)
				end
			end)
		end)
		table.clear(events)
		s(2)
		spec.list(
			events,
			{ "S release 1", "R2 1", "R1 1", "run 2", "F cleanup 1", "F release 1" },
			"Charm ends the previous run's child effects once the rerun returns"
		)
	end,
	["rule 28 T10 a cleanup that throws is reported once and later releases still run"] = function()
		local r = realm()
		local events = {}
		local dispose = r.root(function()
			r.api.onCleanup(function()
				table.insert(events, "first registered")
			end)
			r.api.onCleanup(function()
				error("release failed")
			end)
			r.api.onCleanup(function()
				table.insert(events, "last registered")
			end)
		end)
		dispose()
		spec.list(events, { "last registered", "first registered" }, "newest first, past the failure")
		spec.eq(r.logged("release failed"), 1, "reported once")
		spec.eq(r.logged("failed:"), 1, "the cleanups that returned are not reported")
		dispose()
		spec.eq(r.logged("release failed"), 1, "dispose is idempotent")
	end,
	["rule 25 T10 an effect that disposes its own scope mid-run ends at once and creates nothing after"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events = {}
		local dispose
		dispose = r.root(function()
			r.api.onCleanup(function()
				table.insert(events, "S release")
			end)
			r.api.effect(function()
				r.api.onCleanup(function()
					table.insert(events, "G release")
				end)
			end)
			r.api.effect(function()
				if s() == 1 then
					return nil
				end
				r.api.onCleanup(function()
					table.insert(events, "E run release")
				end)
				dispose()
				table.insert(events, "after dispose")
				r.api.effect(function()
					table.insert(events, "created after the end")
				end)
				return function()
					table.insert(events, "E cleanup")
				end
			end)
		end)
		s(2)
		spec.list(events, {
			"G release",
			"E run release",
			"S release",
			"after dispose",
			"E cleanup",
		}, "the scope ended inside dispose(); the returned cleanup ran at once")
	end,
	["rule 33 T7 a diamond reaches an effect once, with only new values"] = function()
		local r = realm()
		local a = r.charm.atom(1)
		local b = r.charm.computed(function()
			return a() + 1
		end)
		local c = r.charm.computed(function()
			return a() * 2
		end)
		local d = r.charm.computed(function()
			return c() + 1
		end)
		local seen = {}
		r.root(function()
			r.api.effect(function()
				table.insert(seen, `{b()}/{d()}`)
			end)
		end)
		r.flush()
		a(5)
		r.flush()
		spec.list(seen, { "2/3", "6/11" }, "one run per change, never new b with old d")
	end,
	["rule 33 T7 a run is skipped when every computed it read recomputed equal"] = function()
		local r = realm()
		local a = r.charm.atom(1)
		local positive = r.charm.computed(function()
			return a() > 0
		end)
		local parity = r.charm.computed(function()
			return a() % 2
		end)
		local scaled = r.charm.computed(function()
			return parity() * 10
		end)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				positive()
				scaled()
				runs += 1
			end)
		end)
		r.flush()
		a(3)
		r.flush()
		spec.eq(runs, 1, "both paths equal: skipped")
		a(2)
		r.flush()
		spec.eq(runs, 2, "one path moved: ran")
	end,
	["rule 33 T9 a cascade runs in rounds and later effects see the completed write"] = function()
		local r = realm()
		local price, qty, total = r.charm.atom(2), r.charm.atom(3), r.charm.atom(0)
		local labels = {}
		r.root(function()
			r.api.effect(function()
				total(price() * qty())
			end)
			r.api.effect(function()
				table.insert(labels, total())
			end)
		end)
		r.flush()
		price(3)
		r.flush()
		spec.list(labels, { 6, 9 }, "B ran after A's write each time, never with a stale total")
	end,
}
