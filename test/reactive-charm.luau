-- Crucible's lifetimes on Charm 0.11.1, with Charm's strict mode on, as Studio runs it: each user callback Charm calls
-- runs on a new coroutine that must not yield.
local Link = require("./lib/network-link")
local life = require("./lib/reactive-life")
local loadPlace = require("./lib/place")
local realm = require("./lib/reactive-world")
local spec = require("./lib/load")

local function strictLife()
	return life(nil, true)
end

return {
	["P1 rule 20 an effect created after a yield in start ends when its object is destroyed, not before"] = function()
		local w = strictLife()
		local s = w.charm.atom(0)
		local runs = 0
		local object = w.make("Late", {
			start = function()
				coroutine.yield()
				w.api.effect(function()
					s()
					runs += 1
				end)
			end,
		})
		w.boot(object)
		assert(coroutine.resume(object.startThread))
		spec.eq(runs, 1, "created after the yield and run at once")
		s(1)
		spec.eq(runs, 2, "still running while its object lives")
		w.life.request(object, w.Want.Destroyed)
		s(2)
		spec.eq(runs, 2, "ended with its object")
	end,
	["P2 rule 13 a hook whose effect throws finishes its own code, and the failure names the hook with Charm's traceback"] = function()
		local w = strictLife()
		local hp, gold = w.charm.atom(100), w.charm.atom(0)
		local runs, reached = 0, false
		w.boot(w.make("HealthBar", {
			awake = function()
				w.api.effect(function()
					runs += 1
					if hp() <= 0 then
						error("health bar broke")
					end
				end)
			end,
		}))
		w.boot(w.make("Handler", {
			onEnable = function()
				hp(50)
				hp(0)
				gold(gold() + 10)
				reached = true
			end,
		}))
		spec.eq(reached, true, "the hook finished its own code")
		spec.eq(gold(), 10, "its later write landed")
		spec.eq(runs, 2, "the effect ran once more, after the hook")
		spec.eq(#w.effectFailures, 1, "one failure")
		local failure = w.effectFailures[1]
		spec.eq(failure.id, "Handler", "against the hook's object")
		spec.eq(failure.hook, w.Hook.OnEnable, "naming the hook")
		spec.has(failure.error, "health bar broke", "with the effect's error")
		spec.has(failure.error, "stack traceback", "and its traceback")
	end,
	["P3 rule 13 a yielding network handler holds no batch: another object's write runs its effect before the handler resumes"] = function()
		local link = Link.new()
		local charm = link.server.graph.package("@rbxts/charm/src")
		charm.flags.strict = true
		local api = link.server.graph.module("")
		local score = charm.atom(0)
		local seen, held, other = {}, nil, nil
		link:boot(function(s)
			local ping = s.emit.toServer("Charm.Ping", s.n.string)
			s.service("Handler", function(self)
				self.ping = s.emit.receive(self, "ping", ping, function()
					score(1)
					held = coroutine.running()
					coroutine.yield()
					table.insert(seen, "handler resumed")
				end)
			end)
			s.service("Watcher", function(self)
				other = self
			end, {
				awake = function()
					api.effect(function()
						table.insert(seen, `score {score()}`)
					end)
				end,
			})
		end)
		local ann = link:join("Ann", function(c)
			c.ping = c.emit.toServer("Charm.Ping", c.n.string)
		end)
		ann.Network.send(ann.ping, "go")
		link:flush()
		assert(held, "the handler yielded")
		local state = link.server.state(other)
		link.server.life.within(state, state, 6, function()
			score(2)
		end)
		spec.list(seen, { "score 0", "score 1", "score 2" }, "each write ran the effect while the handler waited")
		link:resume(held)
		spec.eq(seen[#seen], "handler resumed", "then the handler resumed")
		link:assertClean()
	end,
	["P4 the place holds one Charm, which the runtime finds with TS.getModule and the game imports"] = function()
		local h = loadPlace()
		local include = h.place:GetService("ReplicatedStorage").rbxts_include
		local copies = 0
		for _, descendant in include:GetDescendants() do
			if descendant.Name == "charm" and descendant.ClassName == "Folder" then
				copies += 1
			end
		end
		spec.eq(copies, 1, "one charm package folder in rbxts_include; vide-charm re-exports it")
		local charm = h.import(include.node_modules["@rbxts"].charm.src)
		charm.flags.strict = true
		local root = h.import(h.crucible)
		local value = charm.atom(0)
		local seen = {}
		local dispose = root.scope(function()
			root.effect(function()
				table.insert(seen, value())
			end)
		end)
		value(1)
		dispose()
		value(2)
		spec.list(seen, { 0, 1 }, "the runtime's effect tracked the game's atom, so both use one copy")
		h.place:Destroy()
	end,
	["S1 rule 26 strict: an effect created inside a run dies on its parent's rerun"] = function()
		local w = strictLife()
		local parent, child = w.charm.atom(0), w.charm.atom(0)
		local responding = 0
		w.boot(w.make("Nest", {
			awake = function()
				w.api.effect(function()
					parent()
					w.api.effect(function()
						child()
						responding += 1
					end)
				end)
			end,
		}))
		parent(1)
		parent(2)
		responding = 0
		child(1)
		spec.eq(responding, 1, "only the last run's child responds")
	end,
	["S2 rule 26 strict: onCleanup runs before each rerun"] = function()
		local w = strictLife()
		local s = w.charm.atom(1)
		local events = {}
		w.boot(w.make("Cleaner", {
			awake = function()
				w.api.effect(function()
					local n = s()
					w.api.onCleanup(function()
						table.insert(events, `clean {n}`)
					end)
					table.insert(events, `run {n}`)
				end)
			end,
		}))
		s(2)
		spec.list(events, { "run 1", "clean 1", "run 2" }, "the run's cleanup ran before the next run")
	end,
	["S3 rule 20 strict: owner lookup inside untracked, watch onChange and a cleanup finds the object on another's write"] = function()
		local w = strictLife()
		local trigger = w.charm.atom(0)
		local seen = {}
		local a = w.make("A", {
			awake = function()
				w.api.effect(function()
					trigger()
					w.charm.untracked(function()
						seen.untracked = w.life.current()
					end)
					w.api.onCleanup(function()
						seen.cleanup = w.life.current()
					end)
				end)
				w.api.watch(trigger, function()
					seen.onChange = w.life.current()
				end)
			end,
		})
		local b = w.make("B")
		w.boot(a, b)
		w.life.within(b, b, w.Hook.Frame, function()
			trigger(1)
		end)
		spec.eq(seen.untracked, a, "inside untracked")
		spec.eq(seen.onChange, a, "inside watch onChange")
		spec.eq(seen.cleanup, a, "inside a cleanup")
		spec.eq(w.life.current(), nil, "nothing leaks to the caller")
	end,
	["S4 rule 36 strict: an effect created during boot runs only after commit, and never after a rollback"] = function()
		local w = strictLife()
		local s = w.charm.atom(0)
		local runs = 0
		w.ownership.holdEffects()
		local committed = w.make("Committed", {
			awake = function()
				w.api.effect(function()
					s()
					runs += 1
				end)
			end,
		})
		w.boot(committed)
		s(1)
		spec.eq(runs, 0, "nothing ran while boot was open")
		w.ownership.releaseEffects(true)
		spec.eq(runs, 1, "one run at commit")
		s(2)
		spec.eq(runs, 2, "then it follows writes")
		local rolled = 0
		w.ownership.holdEffects()
		w.boot(w.make("RolledBack", {
			awake = function()
				w.api.effect(function()
					s()
					rolled += 1
				end)
			end,
		}))
		w.ownership.releaseEffects(false)
		s(3)
		spec.eq(rolled, 0, "a rolled-back boot's effect never runs")
	end,
	["S5 rule 13 strict: a batched entry that yields closes its batch"] = function()
		local w = strictLife()
		local x, y = w.charm.atom(0), w.charm.atom(0)
		local seen = {}
		w.boot(w.make("Watcher", {
			awake = function()
				w.api.effect(function()
					table.insert(seen, `x {x()}`)
				end)
				w.api.effect(function()
					table.insert(seen, `y {y()}`)
				end)
			end,
		}))
		local yielder = w.make("Yielder", {
			awake = function()
				x(1)
				coroutine.yield()
				x(100)
			end,
		})
		w.boot(yielder)
		spec.eq(yielder.phase, 5, "the yielding hook faulted its object")
		spec.list(seen, { "x 0", "y 0", "x 1" }, "its write ran its effect once the batch closed")
		y(1)
		spec.eq(seen[#seen], "y 1", "a later write runs its effect at once: no batch is left open")
	end,
	["A1 rule 15 strict: a watch's source runs as its owner when another object's write triggers it"] = function()
		local w = strictLife()
		local source = w.charm.atom(0)
		local owners, calls = {}, 0
		local a = w.make("A", {
			awake = function()
				w.api.watch(function()
					calls += 1
					owners[calls] = w.life.current() or "no owner"
					return source()
				end, function() end)
			end,
		})
		local b = w.make("B")
		w.boot(a, b)
		w.life.within(b, b, w.Hook.Frame, function()
			source(1)
			source(2)
		end)
		spec.eq(calls >= 3, true, "the source ran at the baseline, at its first run and on B's write")
		for index = 1, calls do
			spec.eq(owners[index], a, `source call {index} ran as A`)
		end
	end,
	["A2 rule 15 strict: what onChange creates dies on the next change"] = function()
		local w = strictLife()
		local source, child = w.charm.atom(0), w.charm.atom(0)
		local responding = 0
		w.boot(w.make("Changes", {
			awake = function()
				w.api.watch(source, function()
					w.api.effect(function()
						child()
						responding += 1
					end)
				end)
			end,
		}))
		source(1)
		source(2)
		responding = 0
		child(1)
		spec.eq(responding, 1, "only the last change's effect responds")
	end,
	["A3 rule 36 strict: a queued boot effect whose owner is destroyed before commit never runs"] = function()
		local w = strictLife()
		local s = w.charm.atom(0)
		local runs = 0
		w.ownership.holdEffects()
		local doomed = w.make("Doomed", {
			awake = function()
				w.api.effect(function()
					s()
					runs += 1
				end)
				w.api.watch(s, function()
					runs += 100
				end)
			end,
		})
		w.boot(doomed)
		w.life.request(doomed, w.Want.Destroyed)
		w.ownership.releaseEffects(true)
		s(1)
		spec.eq(runs, 0, "neither the effect nor the watch ran")
		spec.eq(w.engine.logged("failed at boot's commit"), 0, "and they were skipped without a report")
	end,
	["A4 rule 36 strict: a queued effect and a queued watch cancelled by their disposer before commit never run"] = function()
		local w = strictLife()
		local s = w.charm.atom(0)
		local runs = 0
		local disposers = {}
		w.ownership.holdEffects()
		w.boot(w.make("Cancelled", {
			awake = function()
				table.insert(
					disposers,
					w.api.effect(function()
						s()
						runs += 1
					end)
				)
				table.insert(
					disposers,
					w.api.watch(s, function()
						runs += 100
					end)
				)
			end,
		}))
		for _, dispose in disposers do
			spec.eq(type(dispose), "function", "a queued entry returns a disposer")
			dispose()
		end
		w.ownership.releaseEffects(true)
		s(1)
		spec.eq(runs, 0, "neither ran at commit or after")
		spec.eq(w.engine.logged("failed at boot's commit"), 0, "and they were skipped without a report")
	end,
	["A5 rule 36 strict: an object destroyed by an earlier entry's body during commit never runs its entries"] = function()
		local w = strictLife()
		local runs = 0
		local victim
		w.ownership.holdEffects()
		local killer = w.make("Killer", {
			awake = function()
				w.api.effect(function()
					w.life.request(victim, w.Want.Destroyed)
				end)
			end,
		})
		victim = w.make("Victim", {
			awake = function()
				w.api.effect(function()
					runs += 1
				end)
			end,
		})
		w.boot(killer, victim)
		w.ownership.releaseEffects(true)
		spec.eq(w.life.hasEnded(victim), true, "the earlier entry destroyed the victim")
		spec.eq(runs, 0, "the victim's entry was skipped")
		spec.eq(w.engine.logged("failed at boot's commit"), 0, "without a report")
	end,
	["B1 rule 36 strict: a queued effect and a queued watch's source that dispose themselves at commit run exactly once"] = function()
		local w = strictLife()
		local s = w.charm.atom(0)
		local effectRuns, sourceRuns, changes = 0, 0, 0
		local committing = false
		local disposeEffect, disposeWatch
		w.ownership.holdEffects()
		w.boot(w.make("SelfCancelling", {
			awake = function()
				disposeEffect = w.api.effect(function()
					s()
					effectRuns += 1
					disposeEffect()
				end)
				disposeWatch = w.api.watch(function()
					local value = s()
					if committing then
						sourceRuns += 1
						disposeWatch()
					end
					return value
				end, function()
					changes += 1
				end)
			end,
		}))
		s(1)
		committing = true
		w.ownership.releaseEffects(true)
		s(2)
		s(3)
		spec.eq(effectRuns, 1, "the effect ran once and stopped")
		spec.eq(sourceRuns, 1, "the watch's source ran once at commit and stopped")
		spec.eq(changes, 0, "onChange never ran for a watch its source disposed")
	end,
	["B2 rule 36 strict: a queued body that throws at commit is reported with its owner and traceback, and later entries start"] = function()
		local w = strictLife()
		local s = w.charm.atom(0)
		local later, released = 0, false
		w.ownership.holdEffects()
		w.boot(w.make("Broken", {
			awake = function()
				w.api.effect(function()
					w.api.onCleanup(function()
						released = true
					end)
					error("first run broke")
				end)
				w.api.effect(function()
					s()
					later += 1
				end)
			end,
		}))
		w.ownership.releaseEffects(true)
		spec.eq(w.engine.logged("Crucible: an effect of Broken failed at boot's commit"), 1, "reported with its owner")
		spec.eq(w.engine.logged("first run broke"), 1, "with the body's error")
		spec.eq(w.engine.logged("stack traceback"), 1, "and its traceback")
		spec.eq(released, true, "the failed entry ended")
		spec.eq(later, 1, "the next entry still started")
		s(1)
		spec.eq(later, 2, "and keeps running: the commit was not rolled back")
	end,
	["B3 rule 36 strict: an entry that disposed itself at commit is stopped at once, so no later write reaches it"] = function()
		-- Counts every call Charm makes into a Crucible effect, including reruns whose body is gone.
		local calls = 0
		local r = realm(function(graph)
			local charm = graph.package("@rbxts/charm/src")
			local effect = charm.effect
			charm.effect = function(fn)
				return effect(function()
					calls += 1
					return fn()
				end)
			end
		end, nil, true)
		local s = r.charm.atom(0)
		local dispose
		r.ownership.holdEffects()
		r.root(function()
			dispose = r.api.effect(function()
				s()
				dispose()
			end)
		end)
		r.ownership.releaseEffects(true)
		spec.eq(calls, 1, "Charm ran the entry once, at commit")
		s(1)
		spec.eq(calls, 1, "the write did not reach the stopped entry")
	end,
}
