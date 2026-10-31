local spec = require("./lib/load")
local realm = require("./lib/reactive-world")

return {
	["rule 1 state needs no owner and set stores a function without calling it"] = function()
		local r = realm()
		local handler = r.api.state(nil)
		local calls = 0
		local function f()
			calls += 1
		end
		handler:set(f)
		spec.eq(handler(), f, "the function itself is the value")
		spec.eq(calls, 0, "set never calls its argument")
	end,
	["rule 1 update calls change once with the current value, untracked, and stores the result"] = function()
		local r = realm()
		local a, b = r.api.state(1), r.api.state(10)
		local calls, runs, seen = 0, 0, nil
		r.root(function()
			r.api.effect(function()
				runs += 1
				b:update(function(current)
					calls += 1
					seen = current
					return current + a()
				end)
			end)
		end)
		r.flush()
		spec.eq(b(), 11, "stored the result")
		spec.eq(seen, 10, "passed the current value")
		spec.eq(calls, 1, "called once")
		a:set(5)
		r.flush()
		spec.eq(runs, 1, "a read inside change is not a dependency of the running effect")
	end,
	["rule 1 a write inside update's callback throws and stores nothing"] = function()
		local r = realm()
		local s, other = r.api.state(1), r.api.state(0)
		local ok, err = pcall(function()
			s:update(function(v)
				other:set(5)
				return v + 1
			end)
		end)
		spec.eq(ok, false, "throws")
		spec.has(err, "update's callback must not write state", "names the rule")
		spec.eq(other(), 0, "inner write rejected")
		spec.eq(s(), 1, "update stored nothing")
	end,
	["rule 2 a state is a callable table; a computed is a plain function"] = function()
		local r = realm()
		local s = r.api.state(3)
		local c = r.api.computed(function()
			return s() + 1
		end)
		spec.eq(type(s), "table", "state")
		spec.eq(type(c), "function", "computed")
		spec.eq(s(), 3, "state read")
		spec.eq(c(), 4, "computed read")
		local function read(x)
			return if type(x) == "function" then x() else x
		end
		spec.eq(read(c), 4, "a function-testing reader reads a computed")
		spec.eq(
			read(function()
				return s()
			end),
			3,
			"and a wrapped state"
		)
	end,
	["rule 2 a plain getter tracks what it reads"] = function()
		local r = realm()
		local open, locked = r.api.state(true), r.api.state(false)
		local function usable()
			return open() and not locked()
		end
		local seen = {}
		r.root(function()
			r.api.effect(function()
				table.insert(seen, tostring(usable()))
			end)
		end)
		r.flush()
		locked:set(true)
		r.flush()
		open:set(false)
		r.flush()
		spec.list(seen, { "true", "false", "false" }, "both sources tracked through the getter")
	end,
	["rule 3 an equal write does nothing and requests no flush"] = function()
		local r = realm()
		local t = { n = 1 }
		local s = r.api.state(t)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				s()
				runs += 1
			end)
		end)
		r.flush()
		s:set(t)
		spec.eq(#r.engine.deferred, 0, "same reference: no flush requested")
		t.n = 2
		s:set(t)
		s:update(function(v)
			return v
		end)
		r.flush()
		spec.eq(runs, 1, "a mutated table stored again changes nothing")
		s:set({ n = 2 })
		r.flush()
		spec.eq(runs, 2, "a new table is a change")
	end,
	["rule 30 a write is read at once through a chain and runs no user code"] = function()
		local r = realm()
		local a = r.api.state(1)
		local evals = 0
		local b = r.api.computed(function()
			evals += 1
			return a() + 1
		end)
		local c = r.api.computed(function()
			evals += 1
			return b() * 10
		end)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				c()
				runs += 1
			end)
		end)
		r.flush()
		spec.eq(runs, 1, "first run")
		local before = evals
		a:set(2)
		spec.eq(evals, before, "the write evaluated nothing")
		spec.eq(runs, 1, "the write ran no effect")
		spec.eq(c(), 30, "read in the same call sees the write through both computeds")
		r.flush()
		spec.eq(runs, 2, "the effect runs on the flush")
	end,
	["rule 11 untracked reads are not dependencies"] = function()
		local r = realm()
		local a, b = r.api.state(1), r.api.state(1)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				runs += 1
				b()
				r.api.untracked(function()
					return a()
				end)
			end)
		end)
		r.flush()
		a:set(2)
		r.flush()
		spec.eq(runs, 1, "untracked source changed: no rerun")
		b:set(2)
		r.flush()
		spec.eq(runs, 2, "tracked source changed: rerun")
	end,
	["rule 11 untracked keeps ownership: a rerun ends an effect created inside it"] = function()
		local r = realm()
		local a = r.api.state(1)
		local events = {}
		r.root(function()
			r.api.effect(function()
				local n = a()
				r.api.untracked(function()
					r.api.effect(function()
						table.insert(events, `child {n}`)
						return function()
							table.insert(events, `end child {n}`)
						end
					end)
				end)
			end)
		end)
		r.flush()
		a:set(2)
		r.flush()
		spec.list(events, { "child 1", "end child 1", "child 2" }, "the child belonged to the parent's run")
	end,
	["rule 11 untracked keeps purity: a write inside it in a computed throws"] = function()
		local r = realm()
		local s = r.api.state(0)
		local c = r.api.computed(function()
			r.api.untracked(function()
				s:set(1)
			end)
			return 1
		end)
		local ok, err = pcall(c)
		spec.eq(ok, false, "the computed failed")
		spec.has(err, "computed must not write state", "purity carried into untracked")
		spec.eq(s(), 0, "nothing written")
	end,
	["rule 11 untracked returns its result and rethrows its error"] = function()
		local r = realm()
		spec.eq(
			r.api.untracked(function()
				return 7
			end),
			7,
			"result"
		)
		local ok, err = pcall(r.api.untracked, function()
			error("inner failure")
		end)
		spec.eq(ok, false, "rethrown")
		spec.has(err, "inner failure", "the original error")
		local okOwner, ownerErr = pcall(r.api.effect, function() end)
		spec.eq(okOwner, false, "no frame left behind on this thread")
		spec.has(ownerErr, "needs an owner", "an ownerless effect still throws")
	end,
}
