local spec = require("./lib/load")
local realm = require("./lib/reactive-world")
local life = require("./lib/reactive-life")

return {
	["rule 20 effect, watch and onCleanup need an owner; scope makes a root"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		for name, call in
			{
				effect = function()
					r.api.effect(function() end)
				end,
				watch = function()
					r.api.watch(s, function() end)
				end,
				onCleanup = function()
					r.api.onCleanup(function() end)
				end,
			}
		do
			local ok, err = pcall(call)
			spec.eq(ok, false, name)
			spec.has(err, "effect() needs an owner", name)
		end
		local runs = 0
		local dispose = r.api.scope(function()
			r.api.effect(function()
				s()
				runs += 1
			end)
		end)
		r.flush()
		dispose()
		s(2)
		r.flush()
		spec.eq(runs, 1, "the root owned the effect and its dispose ended it")
	end,
	["rule 20 code a Crucible object runs owns into its generation, ended once through its subscriptions"] = function()
		local w = life()
		local s = w.charm.atom(1)
		local runs = 0
		local object = w.make("Holder", {
			awake = function()
				for _ = 1, 3 do
					w.api.effect(function()
						s()
						runs += 1
					end)
				end
				w.api.onCleanup(function()
					w.log("generation release")
				end)
			end,
		})
		w.boot(object)
		spec.eq(#object.subscriptions, 1, "one subscription for the generation, not one per effect")
		w.flush()
		spec.eq(runs, 3, "owned and run")
		w.life.request(object, w.Want.Destroyed)
		spec.eq(w.events[#w.events - 1], "generation release", "ended after onDestroy, before ended")
		s(2)
		w.flush()
		spec.eq(runs, 3, "effects owned by a destroyed object never run again")
	end,
	["rule 23 a child scope ends with its parent or its dispose, whichever first"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events = {}
		local disposeChild
		local disposeParent = r.root(function()
			disposeChild = r.api.scope(function()
				r.api.onCleanup(function()
					table.insert(events, "child")
				end)
			end)
			r.api.scope(function()
				r.api.onCleanup(function()
					table.insert(events, "sibling")
				end)
			end)
			r.api.onCleanup(function()
				table.insert(events, "parent")
			end)
		end)
		disposeChild()
		disposeChild()
		spec.list(events, { "child" }, "dispose is idempotent")
		disposeParent()
		spec.list(events, { "child", "sibling", "parent" }, "children before the parent's releases")
		s(2)
	end,
	["rule 23 a scope inside an effect run ends at the next run"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events = {}
		r.root(function()
			r.api.effect(function()
				local n = s()
				r.api.scope(function()
					r.api.onCleanup(function()
						table.insert(events, `scope {n}`)
					end)
				end)
			end)
		end)
		r.flush()
		s(2)
		r.flush()
		spec.list(events, { "scope 1" }, "the run's scope ended before the next run")
	end,
	["rule 23 rule 24 a setup that throws releases what it registered and rethrows"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events, runs = {}, 0
		local ok, err = pcall(r.api.scope, function()
			r.api.onCleanup(function()
				table.insert(events, "acquired before the throw")
			end)
			r.api.effect(function()
				s()
				runs += 1
			end)
			error("setup failed")
		end)
		spec.eq(ok, false, "rethrown at the scope call")
		spec.has(err, "setup failed", "the setup's error")
		spec.list(events, { "acquired before the throw" }, "released at completion")
		spec.eq(runs, 1, "the effect ran once when the setup created it")
		s(2)
		spec.eq(runs, 1, "and ended with the setup")
	end,
	["rule 27 under an ended owner nothing is created and onCleanup runs at once"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local events = {}
		local dispose
		dispose = r.root(function()
			r.api.effect(function()
				if s() == 1 then
					return
				end
				dispose()
				r.api.effect(function()
					table.insert(events, "effect created after the end")
				end)
				r.api.watch(function()
					return 1
				end, function() end)
				local inner = r.api.scope(function()
					table.insert(events, "setup ran")
				end)
				inner()
				r.api.onCleanup(function()
					table.insert(events, "ran at once")
				end)
				table.insert(events, "after onCleanup")
			end)
		end)
		s(2)
		spec.list(events, { "ran at once", "after onCleanup" }, "no effect, no watch, no setup")
	end,
	["rule 27 C6 a start segment after its generation ended creates nothing"] = function()
		local w = life()
		local runs = 0
		local object
		object = w.make("Starter", {
			start = function()
				coroutine.yield()
				w.life.request(object, w.Want.Destroyed)
				w.api.effect(function()
					runs += 1
				end)
				w.api.onCleanup(function()
					w.log("ran at once")
				end)
				w.log("segment continued")
			end,
		})
		w.boot(object)
		local mark = #w.events
		assert(coroutine.resume(object.startThread))
		w.flush()
		spec.list(w.since(mark), {
			"onDisable:Starter",
			"onDestroy:Starter",
			"ended:Starter",
			"ran at once",
			"segment continued",
		}, "the extra segment ran under an ended generation")
		spec.eq(runs, 0, "its effect was never created")
	end,
	["rule 27 a child that ends early leaves its parent; the rest end with the parent, oldest first"] = function()
		local w = life()
		local events = {}
		local object = w.make("Busy", {
			awake = function()
				for i = 1, 3 do
					local dispose = w.api.scope(function()
						w.api.onCleanup(function()
							table.insert(events, `early {i}`)
						end)
					end)
					dispose()
				end
				for i = 1, 2 do
					w.api.scope(function()
						w.api.onCleanup(function()
							table.insert(events, `child {i}`)
						end)
					end)
				end
			end,
		})
		w.boot(object)
		spec.list(events, { "early 1", "early 2", "early 3" }, "each ended at its dispose")
		w.life.request(object, w.Want.Destroyed)
		spec.list(events, { "early 1", "early 2", "early 3", "child 1", "child 2" }, "only the live children ended")
	end,
	["rule 27 the oldest child ending early leaves its younger siblings to end with the parent"] = function()
		local w = life()
		local events = {}
		local object = w.make("Busy", {
			awake = function()
				local disposers = {}
				for i = 1, 3 do
					disposers[i] = w.api.scope(function()
						w.api.onCleanup(function()
							table.insert(events, `child {i}`)
						end)
					end)
				end
				disposers[1]()
			end,
		})
		w.boot(object)
		spec.list(events, { "child 1" }, "the oldest ended at its dispose")
		w.life.request(object, w.Want.Destroyed)
		spec.list(events, { "child 1", "child 2", "child 3" }, "its siblings ended with the parent, oldest first")
	end,
	["rule 12 rule 29 a thread spawned inside an effect run belongs to the run until it first yields"] = function()
		local r = realm()
		local s = r.charm.atom(1)
		local runs, before, after, spawned = 0, nil, nil, nil
		r.root(function()
			r.api.effect(function()
				runs += 1
				if spawned then
					return
				end
				spawned = coroutine.create(function()
					s()
					local ok, err = pcall(r.api.effect, function() end)
					before = if ok then "created" else err
					coroutine.yield()
					ok, err = pcall(r.api.effect, function() end)
					after = if ok then "created" else err
				end)
				assert(coroutine.resume(spawned))
			end)
		end)
		spec.eq(before, "created", "owned by the run before it yields")
		assert(coroutine.resume(spawned))
		spec.has(after, "needs an owner", "no owner once resumed from outside the run")
		s(2)
		spec.eq(runs, 2, "its read before the yield was tracked by the run")
	end,
	["rule 28 a release outside a drain applies its lifecycle request before dispose returns"] = function()
		local w = life()
		local other = w.make("Other")
		w.boot(other)
		local dispose = w.api.scope(function()
			w.api.onCleanup(function()
				w.life.request(other, w.Want.Disabled)
				w.log("released")
			end)
		end)
		local mark = #w.events
		dispose()
		spec.list(w.since(mark), { "released", "onDisable:Other" }, "drained when the cleanup returned")
	end,
}
