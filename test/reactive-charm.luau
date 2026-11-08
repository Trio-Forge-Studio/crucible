-- Crucible's lifetimes on Charm 0.11.1, with Charm's strict mode on, as Studio runs it: each user callback Charm calls
-- runs on a new coroutine that must not yield.
local Engine = require("./lib/engine")
local Link = require("./lib/network-link")
local life = require("./lib/reactive-life")
local loadPlace = require("./lib/place")
local realm = require("./lib/reactive-world")
local spec = require("./lib/load")
local SyncWorld = require("./lib/sync-world")

local function strictLife()
	return life(nil, true)
end

-- A strict realm whose `calls.n` counts every call Charm makes into a Crucible effect, including runs whose body is gone.
local function countingRealm()
	local calls = { n = 0 }
	local r = realm(function(graph)
		local charm = graph.package("@rbxts/charm/src")
		local effect = charm.effect
		charm.effect = function(fn)
			return effect(function()
				calls.n += 1
				return fn()
			end)
		end
	end, nil, true)
	return r, calls
end

-- J1's Charm: a failed flush resets a skipped effect's flags to `Watching | Recursed`, dropping its Dirty or Pending.
local SKIPPED_FLAGS_CLEARED = {
	"effect.flags = bit32.bor(effect.flags, bit32.bor(WATCHING, RECURSED))",
	"effect.flags = bit32.bor(WATCHING, RECURSED)",
}

-- Until `spec.rewrite(nil)`, each graph loads a copy of Charm whose init.luau has each `{from, to}` change applied
-- once. Returns how many changes applied so far.
local function rewriteCharm(changes: { { string } }): () -> number
	local applied = 0
	spec.rewrite(function(path, source)
		if string.sub(path, -#"@rbxts/charm/src") ~= "@rbxts/charm/src" then
			return source
		end
		for _, change in changes do
			local start, finish = string.find(source, change[1], 1, true)
			if start then
				applied += 1
				source = string.sub(source, 1, start - 1) .. change[2] .. string.sub(source, finish + 1)
			end
		end
		return source
	end)
	return function()
		return applied
	end
end

-- Loads a strict realm on a copy of Charm changed as `rewriteCharm` does. Returns pcall's result.
local function changedCharm(changes: { { string } })
	local applied = rewriteCharm(changes)
	local ok, result = pcall(realm, nil, nil, true)
	spec.rewrite(nil)
	spec.eq(applied(), #changes, "every change applied to Charm")
	return ok, result
end

-- The runtime's first load on a strict copy of Charm changed as `rewriteCharm` does, inside a batch or inside an
-- effect's run in a flush. Checks that the load fails, restores the active subscriber and stops the check's effects,
-- that Charm runs effects once the batch or flush ends, and that the next check runs again.
local function firstLoadInside(context: string, changes: { { string } })
	local label = `{if #changes == 0 then "Charm 0.11.1" else "J1's Charm"}, first loaded in a {context}`
	local applied = rewriteCharm(changes)
	local graph = spec.runtime(nil, Engine.new().loggingPort)
	local charm = graph.package("@rbxts/charm/src")
	spec.rewrite(nil)
	spec.eq(applied(), #changes, "every change applied to Charm")
	charm.flags.strict = true
	local effect = charm.effect
	local nodes = {}
	charm.effect = function(fn)
		return effect(function()
			nodes[charm.getActiveSub()] = true
			return fn()
		end)
	end
	local function census()
		local count, live = 0, 0
		for node in nodes do
			count += 1
			if node.flags ~= 0 or node.deps ~= nil or node.subs ~= nil then
				live += 1
			end
		end
		return count, live
	end
	local ok, err, outer, after, count, live
	local function load()
		outer = charm.getActiveSub()
		ok, err = pcall(graph.module, "")
		after = charm.getActiveSub()
		count, live = census()
	end
	if context == "batch" then
		charm.batch(load)
	else
		local x = charm.atom(0)
		local stop = effect(function()
			if x() == 1 then
				load()
			end
		end)
		x(1)
		stop()
		spec.eq(outer ~= nil, true, `{label}: the load ran inside an effect`)
	end
	spec.eq(ok, false, `{label}: the runtime did not load`)
	spec.has(err, "require @trioforge/crucible outside both", `{label}: the failure says where to load it`)
	spec.eq(after, outer, `{label}: the check restored the active subscriber`)
	spec.eq(count, 2, `{label}: the check stopped at its first write, which ran no effect`)
	spec.eq(live, 0, `{label}: the check left no effect with a flag, a dependency or a subscriber`)
	for node in nodes do
		spec.eq(node.deps == nil and node.subs == nil, true, `{label}: no check effect is linked after the {context}`)
	end
	local y = charm.atom(0)
	local seen
	local stopSeen = effect(function()
		seen = y()
	end)
	y(2)
	spec.eq(seen, 2, `{label}: Charm runs a write's effects after the {context}`)
	stopSeen()
	table.clear(nodes)
	local checked, failure = pcall(graph.module("internal/reactive/charm").checkCharm)
	if #changes == 0 then
		spec.eq(checked, true, `{label}: the next check passes: {failure}`)
		count, live = census()
		spec.eq(count, 4, `{label}: the next check ran in full`)
		spec.eq(live, 0, `{label}: and stopped every effect it created`)
		local loaded, api = pcall(graph.module, "")
		spec.eq(loaded, true, `{label}: the runtime then loads: {api}`)
	else
		spec.eq(checked, false, `{label}: the next check refuses J1's Charm`)
		spec.has(failure, "forgets a write", `{label}: on the write it forgets`)
	end
end

-- An effect whose first run reads a source, holds a cleanup and writes an atom that an earlier effect throws on, then
-- runs `after`. Checks that it keeps nothing, and returns what effect() threw and the realm.
local function reachingFirstRun(after: () -> ())
	local r, calls = countingRealm()
	local source, trigger = r.charm.atom(0), r.charm.atom(0)
	local node, err
	local released = 0
	local stopRoot = r.root(function()
		r.api.effect(function()
			if trigger() ~= 0 then
				error("a reached effect broke")
			end
		end)
		local ok
		ok, err = pcall(r.api.effect, function()
			node = r.charm.getActiveSub()
			source()
			r.api.onCleanup(function()
				released += 1
			end)
			trigger(1)
			after()
		end)
		spec.eq(ok, false, "effect threw")
	end)
	spec.eq(released, 1, "the new effect released what it acquired")
	spec.eq(node.deps, nil, "the new effect's Charm node holds no dependency")
	local before = calls.n
	source(1)
	spec.eq(calls.n - before, 0, "a write does not reach the new effect")
	stopRoot()
	spec.eq(released, 1, "ending the root released nothing twice")
	return err, r
end

-- Three sibling scopes, each holding an effect and a cleanup; the first one's cleanup disposes the second. `mode` ends
-- their owner by destruction, at disable, or by destruction while boot holds the effects and then a rollback.
local function siblingEnd(mode: string)
	local w = strictLife()
	local x = w.charm.atom(0)
	local disposeSecond
	local runs, cleaned = 0, { 0, 0, 0 }
	local function setup()
		for i = 1, 3 do
			local dispose = w.api.scope(function()
				w.api.onCleanup(function()
					cleaned[i] += 1
					if i == 1 then
						disposeSecond()
					end
				end)
				w.api.effect(function()
					x()
					runs += 1
				end)
			end)
			if i == 2 then
				disposeSecond = dispose
			end
		end
	end
	if mode == "rollback" then
		w.ownership.holdEffects()
	end
	local object = w.make("Siblings", if mode == "interval" then { onEnable = setup } else { awake = setup })
	w.boot(object)
	w.life.request(object, if mode == "interval" then w.Want.Disabled else w.Want.Destroyed)
	if mode == "rollback" then
		w.ownership.releaseEffects(false)
	end
	spec.list(cleaned, { 1, 1, 1 }, "each scope's cleanup ran once, the third included")
	local before = runs
	x(1)
	spec.eq(runs - before, 0, "no effect responds after its owner ended")
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
		-- The runtime ran effects of its own as it loaded, to check Charm.
		calls = 0
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
	["C1 rule 23 strict: a cleanup that disposes a later sibling at destruction still ends every child"] = function()
		siblingEnd("generation")
	end,
	["C2 rule 21 rule 23 strict: a cleanup that disposes a later sibling at disable still ends every child"] = function()
		siblingEnd("interval")
	end,
	["C3 rule 36 rule 23 strict: a cleanup that disposes a later sibling before a rollback still ends every child"] = function()
		siblingEnd("rollback")
	end,
	["D1 rule 26 strict: disposing a nested effect ends what it created at once, and no later write reaches either"] = function()
		local r, calls = countingRealm()
		local trigger, child = r.charm.atom(0), r.charm.atom(0)
		local disposeNested
		local childRuns, cleaned = 0, 0
		local stopRoot = r.root(function()
			r.api.effect(function()
				disposeNested = r.api.effect(function()
					trigger()
					r.api.effect(function()
						child()
						childRuns += 1
						r.api.onCleanup(function()
							cleaned += 1
						end)
					end)
				end)
			end)
		end)
		disposeNested()
		spec.eq(cleaned, 1, "the nested effect's child ended inside the dispose")
		local before = calls.n
		child(1)
		trigger(1)
		spec.eq(calls.n - before, 0, "neither the disposed effect nor its child ran again")
		spec.eq(childRuns, 1, "the child's body ran only at creation")
		stopRoot()
		spec.eq(cleaned, 1, "ending the root released nothing twice")
	end,
	["D2 rule 26 strict: a nested effect disposed inside a batched hook ends what it created before the hook returns"] = function()
		local w = strictLife()
		local child = w.charm.atom(0)
		local disposeNested
		local cleaned, seen = 0, nil
		local object = w.make("Nest", {
			awake = function()
				w.api.effect(function()
					disposeNested = w.api.effect(function()
						w.api.effect(function()
							child()
							w.api.onCleanup(function()
								cleaned += 1
							end)
						end)
					end)
				end)
			end,
		})
		w.boot(object)
		w.life.within(object, object, w.Hook.Frame, function()
			disposeNested()
			seen = cleaned
		end)
		spec.eq(seen, 1, "the child's cleanup ran inside the hook's batch, at the dispose")
		spec.eq(cleaned, 1, "and only once")
	end,
	["E1 rule 13 strict: an effect that throws on a write does not leave a per-player projection of that write unsent"] = function()
		local w = SyncWorld.new({
			declare = function(r)
				return { Value = r.emit.replicatedPerPlayer("Charm.Value", 0, r.n.f64) }
			end,
			serve = function(s, d, world)
				s.charm.flags.strict = true
				world.x = s.charm.atom(0)
				world.pulls = 0
				s.service("Publisher", function(_self)
					s.entry.publish(d.Value, function()
						world.pulls += 1
						return world.x()
					end)
				end, {
					awake = function()
						s.api.effect(function()
							if world.x() == 1 then
								error("a UI effect broke")
							end
						end)
					end,
				})
			end,
		})
		local ann = w:join("Ann")
		w:heartbeat()
		spec.eq(ann.d.Value(), 0, "the join's round sent the first value")
		local pulls = w.pulls
		local ok = pcall(w.x, 1)
		spec.eq(ok, false, "the earlier effect threw to the writer")
		w:heartbeat()
		spec.eq(ann.d.Value(), 1, "the next Heartbeat still sent the written value")
		spec.eq(w.pulls - pulls, 1, "after one pull")
		w:heartbeat()
		spec.eq(w.pulls - pulls, 1, "a quiet Heartbeat pulls nothing")
		w:finish()
	end,
	["F1 rule 15 strict: an effect that catches a watch's throwing baseline still tracks what it reads after"] = function()
		local w = strictLife()
		local tail = w.charm.atom(0)
		local runs, caught = 0, 0
		w.boot(w.make("Catcher", {
			awake = function()
				w.api.effect(function()
					runs += 1
					local ok = pcall(w.api.watch, function()
						error("baseline broke")
					end, function() end)
					if not ok then
						caught += 1
					end
					tail()
				end)
			end,
		}))
		tail(1)
		spec.eq(caught, 2, "each run caught the watch's failure")
		spec.eq(runs, 2, "the write after the catch reran the effect")
	end,
	["F2 rule 17 strict: an effect whose first run throws keeps no subscription, at the root or nested"] = function()
		local r, calls = countingRealm()
		local source = r.charm.atom(0)
		local released = 0
		local function failing()
			local ok = pcall(r.api.effect, function()
				source()
				r.api.onCleanup(function()
					released += 1
				end)
				error("first run broke")
			end)
			spec.eq(ok, false, "effect threw the first run's failure")
		end
		local stopRoot = r.root(failing)
		spec.eq(released, 1, "the failed root effect released what it acquired")
		local before = calls.n
		source(1)
		spec.eq(calls.n - before, 0, "a write does not reach the failed root effect")
		local stopNested = r.root(function()
			r.api.effect(failing)
		end)
		spec.eq(released, 2, "the failed nested effect released what it acquired")
		before = calls.n
		source(2)
		spec.eq(calls.n - before, 0, "a write does not reach the failed nested effect")
		stopRoot()
		stopNested()
	end,
	["G1 rule 21 rule 36 strict: an enable interval that ends before boot commits cancels its queued effect and watch"] = function()
		local w = strictLife()
		local x = w.charm.atom(0)
		local runs, changes = 0, 0
		w.ownership.holdEffects()
		local object = w.make("Brief", {
			onEnable = function()
				w.api.effect(function()
					x()
					runs += 1
				end)
				w.api.watch(x, function()
					changes += 1
				end)
			end,
		})
		w.boot(object)
		w.life.request(object, w.Want.Disabled)
		w.ownership.releaseEffects(true)
		x(1)
		spec.eq(runs, 0, "the queued effect never ran")
		spec.eq(changes, 0, "the queued watch never changed")
		spec.eq(w.engine.logged("failed at boot's commit"), 0, "and both were skipped without a report")
	end,
	["G2 rule 20 strict: raw Charm subscribe and listen callbacks get no Crucible owner from the object that made them"] = function()
		local w = strictLife()
		local x = w.charm.atom(0)
		local runner = w.graph.module("internal/runner")
		local seen = {}
		local function who(): string
			local owner = w.life.current()
			return if owner then owner.record.id else "none"
		end
		local a = w.make("A", {
			awake = function()
				w.charm.subscribe(function()
					seen.source = who()
					return x()
				end, function()
					seen.subscribe = who()
				end)
				w.charm.listen(x, function()
					seen.listen = who()
				end)
			end,
		})
		local b = w.make("B")
		w.boot(a, b)
		spec.eq(seen.listen, "A", "listen ran its callback at once, inside A's hook")
		runner.call(function()
			x(1)
		end, nil, nil, b)
		spec.list(
			{ seen.source, seen.subscribe, seen.listen },
			{ "B", "B", "B" },
			"a write from B's code runs each callback as B"
		)
		x(2)
		spec.list(
			{ seen.source, seen.subscribe, seen.listen },
			{ "none", "none", "none" },
			"a write from no Crucible code runs each with no owner"
		)
	end,
	["H1 rule 17 strict: an effect whose first run reaches an effect that throws ends, and throws that failure"] = function()
		local err = reachingFirstRun(function() end)
		spec.has(err, "a reached effect broke", "effect threw the reached effect's failure")
	end,
	["H2 rule 17 strict: an effect whose first run throws after reaching an effect that throws ends, and throws its own failure"] = function()
		local err, r = reachingFirstRun(function()
			error("first run broke")
		end)
		spec.has(err, "first run broke", "effect threw the first run's failure")
		spec.eq(r.logged("a reached effect broke"), 1, "and the reached effect's failure was reported")
	end,
	["I1 rule 26 strict: nested effects disposed in a batch that a throwing effect ends keep no subscription"] = function()
		local r, calls = countingRealm()
		local y, x, z = r.charm.atom(0), r.charm.atom(0), r.charm.atom(0)
		local child, leaf, disposeChild
		local childRuns, cleaned = 0, 0
		local stopRoot = r.root(function()
			r.api.effect(function()
				if y() ~= 0 then
					error("an earlier effect broke")
				end
			end)
			-- The child is its parent's first dependency, where Charm's own stop would corrupt the parent's list.
			r.api.effect(function()
				disposeChild = r.api.effect(function()
					child = r.charm.getActiveSub()
					childRuns += 1
					x()
					r.api.effect(function()
						leaf = r.charm.getActiveSub()
						x()
						r.api.onCleanup(function()
							cleaned += 1
						end)
					end)
				end)
				z()
			end)
		end)
		r.charm.startBatch()
		y(1)
		disposeChild()
		spec.eq(cleaned, 1, "the descendant's cleanup ran inside the batch")
		spec.eq(pcall(r.charm.endBatch), false, "the earlier effect threw as the batch closed")
		spec.eq(child.deps, nil, "the disposed effect holds no dependency")
		spec.eq(leaf.deps, nil, "nor does the effect it created")
		local before = calls.n
		x(1)
		spec.eq(calls.n - before, 0, "a write reaches neither")
		z(1)
		spec.eq(childRuns, 2, "the parent's next run creates its child again")
		x(2)
		spec.eq(childRuns, 3, "which a write reaches")
		z(2)
		spec.eq(childRuns, 4, "and the parent still reruns")
		stopRoot()
		before = calls.n
		x(3)
		z(3)
		spec.eq(calls.n - before, 0, "ending the root stops every effect")
	end,
	["I2 rule 26 strict: disposing a nested effect ends the effects it created before its own cleanups"] = function()
		local r = realm(nil, nil, true)
		local order = {}
		local disposeNested
		local stopRoot = r.root(function()
			r.api.effect(function()
				disposeNested = r.api.effect(function()
					r.api.onCleanup(function()
						table.insert(order, "nested")
					end)
					r.api.effect(function()
						r.api.onCleanup(function()
							table.insert(order, "child")
						end)
					end)
				end)
			end)
		end)
		disposeNested()
		spec.list(order, { "child", "nested" }, "the child ended first, then the nested effect's cleanup ran")
		stopRoot()
		spec.list(order, { "child", "nested" }, "ending the root released nothing twice")
	end,
	["J1 rule 13 strict: the runtime refuses to load on a Charm whose failed flush clears a skipped effect's flags"] = function()
		local ok, err = changedCharm({ SKIPPED_FLAGS_CLEARED })
		spec.eq(ok, false, "the runtime did not load")
		spec.has(err, "Crucible needs @rbxts/charm 0.11.1", "the failure names the version")
		spec.has(err, "forgets a write", "and what differs")
	end,
	["J2 rule 26 strict: the runtime refuses to load on a Charm whose stop keeps a nested effect's dependencies"] = function()
		local ok, err = changedCharm({
			{ "\tpurgeDeps(effect)\n\n\t-- Unlink from the parent effect", "\t-- Unlink from the parent effect" },
		})
		spec.eq(ok, false, "the runtime did not load")
		spec.has(err, "stops a nested effect differently", "the failure says what differs")
	end,
	["J3 rule 13 strict: the runtime loads on a Charm whose failed flush still runs every queued effect"] = function()
		local ok, r = changedCharm({
			{
				"\tlocal success, err = pcall(function()\n",
				"\tlocal firstFailure\n\tlocal success, err = pcall(function()\n",
			},
			{
				"\t\t\trun(effect)\n\t\tend\n",
				"\t\t\tlocal ran, failure = pcall(run, effect)\n\t\t\tif not ran and firstFailure == nil then\n\t\t\t\tfirstFailure = failure\n\t\t\tend\n\t\tend\n\t\tif firstFailure ~= nil then\n\t\t\terror(firstFailure, 0)\n\t\tend\n",
			},
		})
		spec.eq(ok, true, `the runtime loaded: {r}`)
		local x = r.charm.atom(0)
		local seen = 0
		r.root(function()
			r.api.effect(function()
				if x() ~= 0 then
					error("an earlier effect broke")
				end
			end)
			r.api.effect(function()
				seen = x()
			end)
		end)
		spec.eq(pcall(x, 1), false, "the earlier effect threw to the writer")
		spec.eq(seen, 1, "and the later one still ran")
	end,
	["J4 rule 13 strict: a first load inside a Charm batch or flush fails, stops the check's effects and checks again next time"] = function()
		spec.readAhead(true)
		local ok, err = pcall(function()
			for _, changes in { {}, { SKIPPED_FLAGS_CLEARED } } do
				firstLoadInside("batch", changes)
				firstLoadInside("flush", changes)
			end
		end)
		spec.readAhead(false)
		if not ok then
			error(err, 0)
		end
	end,
	["J5 rule 13 strict: the runtime refuses to load on a Charm whose batch end runs no effect"] = function()
		local ok, err = changedCharm({ { "\tbatchDepth -= 1\n\tflush()\n", "\tbatchDepth -= 1\n" } })
		spec.eq(ok, false, "the runtime did not load")
		spec.has(err, "Charm ran no effect for the check's write", "the failure says the check could not run")
	end,
	["K1 rule 70 strict: a quiet Heartbeat reads a bounded share of projections, and a sweep still finds a skipped one"] = function()
		local PEERS, DECLARATIONS = 9, 256
		local frames = {}
		local w = SyncWorld.new({
			declare = function(r)
				local declared = {}
				for i = 1, DECLARATIONS do
					declared[i] = r.emit.replicatedPerPlayer(`Scan.Value{i}`, 0, r.n.f64)
				end
				return declared
			end,
			serve = function(s, d, world)
				s.charm.flags.strict = true
				world.own = {}
				for i = 1, PEERS do
					world.own[`Peer{i}`] = s.charm.atom(0)
				end
				s.service("Publisher", function()
					for i = 1, DECLARATIONS do
						s.entry.publish(d[i], function(player)
							return world.own[player.Name]() + i
						end)
					end
				end, {
					awake = function()
						s.api.effect(function()
							for _, own in world.own do
								if own() ~= 0 then
									error("a UI effect broke")
								end
							end
						end)
					end,
				})
			end,
		})
		w.server.graph.module("internal/sync/server-sync").installServerSync(function(player, frame)
			table.insert(frames, { player = player.Name, frame = frame })
			return true
		end)
		local sync = w:sync()
		for i = 1, PEERS do
			sync:snapshot({ Name = `Peer{i}`, UserId = i })
		end
		local reads = 0
		for _, view in sync.order do
			for _, projection in view.projections do
				local node = projection.node
				projection.node = setmetatable({}, {
					__index = function(_, key)
						if key == "flags" then
							reads += 1
						end
						return node[key]
					end,
				})
			end
		end
		w:heartbeat()
		for _ = 1, 3 do
			reads = 0
			w:heartbeat()
			assert(reads > 0 and reads < PEERS * DECLARATIONS, `a quiet Heartbeat read {reads} projection flags`)
			local limit = w.server.reactiveLimits.SYNC_HEARTBEAT_SCAN
			assert(reads <= limit + DECLARATIONS - 1, `{reads} reads stay within SYNC_HEARTBEAT_SCAN's whole peers`)
			spec.eq(sync.spent, reads, "each read is a visit")
			spec.eq(sync.examined, 0, "and no peer is examined")
		end
		local function sweep()
			return math.ceil(#sync.order / math.max(1, w.server.reactiveLimits.SYNC_HEARTBEAT_SCAN // DECLARATIONS))
		end
		local function stall(name)
			local before = #frames
			local own = w.own[name]
			spec.eq(pcall(own, own() + 1), false, `the earlier effect threw to {name}'s writer`)
			local waited = 0
			while #frames == before and waited < 10 do
				w:heartbeat()
				waited += 1
			end
			spec.eq(#frames - before, 1, `{name}'s skipped projections were sent in one frame`)
			spec.eq(frames[#frames].player, name, "to the peer whose projections were skipped")
			spec.eq(#frames[#frames].frame.values, DECLARATIONS, "with every skipped projection's value")
			return waited
		end
		local waited = stall("Peer6")
		assert(
			waited <= sweep(),
			`Peer6's projections were sent after {waited} Heartbeats, within a sweep of {sweep()}`
		)
		assert(sync.scanned >= 1 and sync.scanned < #sync.order, "the scan stopped inside the order")
		local nextPeer = sync.order[sync.scanned + 1].player.Name
		sync["end"](sync, sync.order[1])
		spec.eq(stall(nextPeer), 1, "a peer that left before the cursor moves no peer past the next scan")
		local behind = sync.order[sync.scanned].player.Name
		waited = stall(behind)
		assert(waited <= sweep(), `the peer the scan just read waited {waited} Heartbeats, within a sweep of {sweep()}`)
		w:finish()
	end,
}
