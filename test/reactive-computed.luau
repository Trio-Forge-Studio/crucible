local spec = require("./lib/load")
local realm = require("./lib/reactive-world")
local world = require("./lib/boot-world")

return {
	["rule 4 a computed is lazy and evaluates again only when read after a change"] = function()
		local r = realm()
		local a = r.api.state(2)
		local evals = 0
		local c = r.api.computed(function()
			evals += 1
			return a() * 3
		end)
		spec.eq(evals, 0, "not evaluated at creation")
		spec.eq(c(), 6, "first read")
		spec.eq(c(), 6, "second read")
		spec.eq(evals, 1, "cached")
		a:set(5)
		spec.eq(evals, 1, "a write does not evaluate")
		spec.eq(c(), 15, "read after a change")
		spec.eq(evals, 2, "evaluated once more")
	end,
	["rule 5 an equal result keeps the cache, so nothing downstream reruns"] = function()
		local r = realm()
		local a = r.api.state(1)
		local parity = r.api.computed(function()
			return a() % 2
		end)
		local downstream = 0
		local label = r.api.computed(function()
			downstream += 1
			return `p{parity()}`
		end)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				label()
				runs += 1
			end)
		end)
		r.flush()
		a:set(3)
		r.flush()
		spec.eq(downstream, 1, "parity recomputed equal: label not recomputed")
		spec.eq(runs, 1, "effect skipped")
		a:set(4)
		r.flush()
		spec.eq(runs, 2, "a real change reaches the effect")
		spec.eq(label(), "p0", "new value")
	end,
	["rule 6 a failure is cached and rethrown until a dependency changes"] = function()
		local r = realm()
		local a = r.api.state(1)
		local broken = true
		local evals = 0
		local c = r.api.computed(function()
			evals += 1
			local v = a()
			if broken then
				error("broken value")
			end
			return v
		end)
		local ok1, err1 = pcall(c)
		local ok2, err2 = pcall(c)
		spec.eq(ok1 or ok2, false, "both reads throw")
		spec.has(err1, "broken value", "the original error")
		local _, frames = string.gsub(err1, "reactive%-computed:%d+", "")
		assert(frames >= 2, `the message carries the original traceback: {err1}`)
		spec.eq(err1, err2, "the same message every read")
		spec.eq(evals, 1, "cached, not re-evaluated")
		broken = false
		spec.eq(pcall(c), false, "no dependency changed: still the cached failure")
		a:set(2)
		spec.eq(c(), 2, "recovered once a dependency changed")
	end,
	["rule 6 a failure keeps the previous dependencies, so either kind of change recovers it"] = function()
		local r = realm()
		local mode, a, b = r.api.state("a"), r.api.state(1), r.api.state(1)
		local failing = true
		local c = r.api.computed(function()
			if mode() == "b" then
				local v = b()
				if failing then
					error("b failed")
				end
				return v * 100
			end
			return a()
		end)
		spec.eq(c(), 1, "first value through a")
		mode:set("b")
		spec.eq(pcall(c), false, "fails reading b")
		failing = false
		a:set(7)
		spec.eq(c(), 100, "recovered by a change to a, a dependency of the previous evaluation only")
		failing = true
		b:set(2)
		spec.eq(pcall(c), false, "fails again")
		failing = false
		b:set(3)
		spec.eq(c(), 300, "recovered by a change to b, read before the failure")
	end,
	["rule 6 the version moves only when the cached outcome changes"] = function()
		local r = realm()
		local a = r.api.state(1)
		local c = r.api.computed(function()
			a()
			error("always")
		end)
		local downstream = 0
		local d = r.api.computed(function()
			downstream += 1
			return (pcall(c))
		end)
		spec.eq(d(), false, "downstream sees the failure")
		a:set(2)
		spec.eq(d(), false, "same failure")
		spec.eq(downstream, 1, "same message: version unchanged, downstream kept its cache")
	end,
	["rule 6 a failure that read nothing never recomputes; a cached nil is a value"] = function()
		local r = realm()
		local evals = 0
		local c = r.api.computed(function()
			evals += 1
			error("nothing read")
		end)
		local nilEvals = 0
		local n = r.api.computed(function()
			nilEvals += 1
			return nil
		end)
		local other = r.api.state(1)
		spec.eq(pcall(c), false, "fails")
		other:set(2)
		spec.eq(pcall(c), false, "still fails")
		spec.eq(evals, 1, "never recomputed")
		spec.eq(n(), nil, "nil")
		spec.eq(n(), nil, "nil again")
		other:set(3)
		spec.eq(n(), nil, "nil after an unrelated write")
		spec.eq(nilEvals, 1, "a cached nil is a cache")
	end,
	["rule 7 writes, effects, watches, scopes and cleanups throw inside a computed"] = function()
		local r = realm()
		local s = r.api.state(0)
		local attempts = {
			{
				"write state",
				function()
					s:set(1)
				end,
			},
			{
				"create effects",
				function()
					r.api.effect(function() end)
				end,
			},
			{
				"create effects",
				function()
					r.api.watch(s, function() end)
				end,
			},
			{
				"create scopes",
				function()
					r.api.scope(function() end)
				end,
			},
			{
				"register cleanups",
				function()
					r.api.onCleanup(function() end)
				end,
			},
		}
		for _, attempt in attempts do
			local c = r.api.computed(function()
				attempt[2]()
				return true
			end)
			local ok, err = pcall(c)
			spec.eq(ok, false, attempt[1])
			spec.has(err, `computed must not {attempt[1]}`, attempt[1])
		end
		spec.eq(s(), 0, "nothing written")
	end,
	["rule 7 a yield fails the evaluation with the runner's message"] = function()
		local r = realm()
		local c = r.api.computed(function()
			coroutine.yield()
			return 1
		end)
		local ok, err = pcall(c)
		spec.eq(ok, false, "fails")
		spec.has(err, "yielded", "runner's yield message")
		local ok2 = pcall(c)
		spec.eq(ok2, false, "cached")
	end,
	["rule 12 rule 13 rule 32 a yield in a nested computed suspends the outer evaluation, and its boundary completes both"] = function()
		local r = realm()
		local reached = false
		local inner = r.api.computed(function()
			coroutine.yield()
			return 1
		end)
		local outer = r.api.computed(function()
			local ok = pcall(inner)
			reached = true
			return ok
		end)
		local ok, err = pcall(outer)
		spec.eq(ok, false, "the outer evaluation failed")
		spec.has(err, "yielded", "with the runner's yield message")
		spec.eq(reached, false, "the yield suspended the outer read, so its pcall never returned")
		local innerOk, innerErr = pcall(inner)
		spec.eq(innerOk, false, "the nested computed was completed too")
		spec.has(innerErr, "yielded", "with the same message")
	end,
	["rule 7 X.get inside a computed takes the inside-drain branch"] = function()
		local w = world()
		local api = w.graph.module("")
		local Gate = w.define("Gate", {})
		w.boot()
		local instance = w.tagged("G1", "Gate")
		local constructed = false
		local lookup = api.computed(function()
			return Gate.get(Gate, instance)
		end)
		spec.eq(lookup(), nil, "inside the evaluation get does not reconcile")
		constructed = Gate.get(Gate, instance) ~= nil
		spec.eq(constructed, true, "outside, get reconciles and finds it")
	end,
	["rule 8 a computed that reads itself caches a cycle error for both"] = function()
		local r = realm()
		local evals = 0
		local self
		self = r.api.computed(function()
			evals += 1
			return self()
		end)
		local ok, err = pcall(self)
		spec.eq(ok, false, "fails")
		spec.has(err, "reads itself", "cycle error")
		spec.eq(pcall(self), false, "cached")
		spec.eq(evals, 1, "not evaluated again")
		local a, b
		a = r.api.computed(function()
			return b()
		end)
		b = r.api.computed(function()
			return a()
		end)
		local okA, errA = pcall(a)
		local okB, errB = pcall(b)
		spec.eq(okA or okB, false, "both fail")
		spec.has(errA, "reads itself", "a")
		spec.has(errB, "reads itself", "b")
	end,
	["rule 8 a cycle that appears through a branch is a cycle error, never a stale value"] = function()
		local r = realm()
		local flag = r.api.state(false)
		local x, y
		y = r.api.computed(function()
			return x() + 1
		end)
		x = r.api.computed(function()
			return if flag() then y() else 0
		end)
		spec.eq(x(), 0, "x")
		spec.eq(y(), 1, "y read x")
		flag:set(true)
		local ok, err = pcall(x)
		spec.eq(ok, false, "x now reads y, which reads x")
		spec.has(err, "reads itself", "a real cycle reports itself")
		flag:set(false)
		spec.eq(x(), 0, "x recovers when the branch flips back")
	end,
	["rule 9 T8 an unobserved computed validates by epoch and holds no subscription"] = function()
		local r = realm()
		local x = r.api.state(1)
		local evals = 0
		local c = r.api.computed(function()
			evals += 1
			return x() * 2
		end)
		spec.eq(c(), 2, "evaluates")
		spec.eq(c(), 2, "epoch unchanged")
		spec.eq(evals, 1, "returned at once")
		x:set(2)
		spec.eq(#r.engine.deferred, 0, "an unattached computed is not marked: nothing queued")
		spec.eq(c(), 4, "cross-judge B1: epoch moved, dependency moved, recomputed")
		local other = r.api.state(0)
		other:set(1)
		spec.eq(c(), 4, "epoch moved but no dependency did")
		spec.eq(evals, 2, "validated without recomputing")
	end,
	["rule 10 T8 the first observer attaches a computed and the last detaches it"] = function()
		local r = realm()
		local x = r.api.state(1)
		local evals = 0
		local c = r.api.computed(function()
			evals += 1
			return x() * 2
		end)
		local d = r.api.computed(function()
			return c() + 1
		end)
		local seen = {}
		local dispose = r.root(function()
			r.api.effect(function()
				table.insert(seen, d())
			end)
		end)
		r.flush()
		x:set(2)
		r.flush()
		spec.list(seen, { 3, 5 }, "marks reach the effect through two attached computeds")
		local before = evals
		spec.eq(d(), 5, "attached and clean: O(1)")
		spec.eq(evals, before, "no recompute while clean")
		dispose()
		x:set(3)
		r.flush()
		spec.list(seen, { 3, 5 }, "the stopped effect never runs again")
		spec.eq(evals, before, "detached: the write marked nothing")
		spec.eq(d(), 7, "a detached computed still validates through its kept dependencies")
	end,
	["rule 10 an effect that reads a computed after writing its source runs again"] = function()
		local r = realm()
		local x = r.api.state(1)
		local c = r.api.computed(function()
			return x() * 10
		end)
		local seen = {}
		r.root(function()
			r.api.effect(function()
				local v = c()
				table.insert(seen, v)
				if v < 30 then
					x:set(v // 10 + 1)
				end
			end)
		end)
		r.flush()
		spec.list(seen, { 10, 20, 30 }, "each run saw the write the previous one made")
	end,
}
