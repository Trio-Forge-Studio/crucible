local meter = require("./lib/meter")
local spec = require("./lib/load")
local world = require("./lib/world")

local Phase, Hook = world.Phase, world.Hook

local function assertIntervalsClosed(w, names: { string })
	for _, name in names do
		spec.eq(w.count(`onDisable:{name}`), w.count(`onEnable:{name}`), `{name}'s enable intervals closed`)
		spec.eq(w.count(`onDestroy:{name}`), w.count(`awake:{name}`), `{name}'s awake closed by onDestroy`)
	end
end

return {
	["act-on 1: binding that queues pairs on every round ends the drain with one failure naming the round cap"] = function()
		local w = world.new()
		local rounds = 0
		w.reconcile = function()
			rounds += 1
			if rounds > 5000 then
				error("unbounded: more than 5000 reconcile rounds")
			end
			w.life.notePairs()
			return {}
		end
		w.life.notePairs()
		local ok, err = pcall(w.life.drain)
		spec.eq(ok, true, `the drain returned: {err}`)
		spec.eq(rounds, 1024, "reconcile rounds")
		spec.eq(#w.failures, 1, "failures reported")
		spec.has(w.failures[1], "1024 reconcile rounds", "the failure names the cap")
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
	end,

	["act-on 5 (A4): a frame walk survives a batch cap reached inside one member's scope"] = function()
		local w = world.new()
		local function chain(i)
			return w.make(`N{i}`, {
				start = function()
					w.life.admit({ chain(i + 1) })
				end,
			})
		end
		local m1, m2, f1, f2, f3 = w.boot(w.make("M1"), w.make("M2"), w.make("F1"), w.make("F2"), w.make("F3"))
		local list = w.DenseList.new()
		for _, s in { m1, m2, f1, f2, f3 } do
			list:add(s)
		end
		local function frame(s)
			table.insert(w.events, `frame:{s.record.name}`)
			if s == m1 then
				w.life.admit({ chain(1) })
				for _, filler in { f1, f2, f3 } do
					list:remove(filler)
				end
			end
		end
		w.clear()
		local ok, err = pcall(list.forEach, list, function(s)
			w.life.within(s, s, Hook.Frame, frame, s, nil)
		end)
		spec.eq(ok, true, `the walk returned: {err}`)
		spec.eq(w.count("frame:M2"), 1, "the second member's callback ran")
		spec.eq(#w.failures, 1, "failures reported")
		spec.has(w.failures[1], "1024 batches", "the failure names the cap")
		spec.eq(m1.fault, nil, "the scope's owner is not blamed for the cap")
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
		local m = meter(list)
		local seen = {}
		list:forEach(function(s)
			table.insert(seen, s.record.name)
		end)
		spec.list(seen, { "M1", "M2" }, "members after the walk")
		spec.eq(m.reads <= 2 * list:size(), true, `the list compacted: a walk examined {m.reads} slots`)
	end,

	["act-on 5 (A11): an effect that throws mid-batch faults the batch with Internal and the reconciler recovers"] = function()
		local w = world.new()
		local thrown = false
		local a, b, c =
			w.make("A"), w.make("B", {
				join = function()
					if not thrown then
						thrown = true
						error("join threw")
					end
				end,
			}), w.make("C")
		local ok, err = pcall(w.life.admit, { a, b, c })
		spec.eq(ok, true, `no error reached the caller: {err}`)
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
		spec.eq(#w.failures, 1, "failures reported")
		spec.has(w.failures[1], "join threw", "the failure carries the internal error")
		for _, s in { a, b, c } do
			local name = s.record.name
			spec.eq(s.phase, Phase.Faulted, `{name} phase`)
			spec.eq(s.fault.hook, Hook.Internal, `{name} primary fault`)
			spec.eq(w.count(`faulted:{name}`), 1, `{name} ended once`)
			spec.eq(s.queued, false, `{name} is off the work stack`)
		end
		assertIntervalsClosed(w, { "A", "B", "C" })
		w.clear()
		local d = w.boot(w.make("D"))
		spec.eq(d.phase, Phase.Enabled, "a later batch runs")
		spec.eq(w.count("tick:D"), 1, "a later batch ticks")
		spec.eq(#w.failures, 1, "no stale failure")
	end,

	["act-on 5 (A11): a generation whose teardown fails again is force-ended without hooks and the others tear down"] = function()
		local w = world.new()
		local a, b, c =
			w.make("A"), w.make("B", {
				join = function()
					error("join threw")
				end,
				leave = function()
					error("leave threw")
				end,
			}), w.make("C")
		local ok, err = pcall(w.life.admit, { a, b, c })
		spec.eq(ok, true, `no error reached the caller: {err}`)
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
		spec.eq(#w.failures, 2, "both internal errors reported")
		spec.has(w.failures[1], "join threw", "the first failure")
		spec.has(w.failures[2], "leave threw", "the second failure")
		for _, s in { a, b, c } do
			local name = s.record.name
			spec.eq(s.phase, Phase.Faulted, `{name} phase`)
			spec.eq(s.fault.hook, Hook.Internal, `{name} primary fault`)
			spec.eq(w.count(`faulted:{name}`), 1, `{name} ended once`)
			spec.eq(s.joined, false, `{name} left its sets`)
			spec.eq(s.queued, false, `{name} is off the work stack`)
		end
		spec.eq(w.count("onDisable:B") + w.count("onDestroy:B"), 0, "the force-ended generation runs no user hooks")
		assertIntervalsClosed(w, { "A", "C" })
		w.clear()
		w.boot(w.make("D"))
		spec.eq(w.count("tick:D"), 1, "a later batch runs")
		spec.eq(#w.failures, 2, "no stale failure")
	end,

	["act-on 5: recovery that force-ends one generation still applies the other queued requests"] = function()
		local w = world.new()
		local a, b = w.boot(w.make("A"), w.make("B"))
		w.scripts.A.untick = function()
			error("untick threw")
		end
		w.scripts.A.leave = function()
			error("leave threw")
		end
		w.clear()
		w.life.within(nil, nil, Hook.Frame, function()
			w.life.request(a, world.Want.Destroyed)
			w.life.request(b, world.Want.Disabled)
		end, nil, nil)
		spec.eq(a.phase, Phase.Faulted, "A is force-ended")
		spec.eq(w.count("faulted:A"), 1, "A ended once")
		spec.eq(b.phase, Phase.Disabled, "B applied its request")
		spec.eq(b.ticking, false, "B stopped ticking")
		spec.eq(w.count("onDisable:B"), 1, "B ran onDisable")
		spec.eq(b.queued, false, "B is off the work stack")
		spec.eq(#w.failures, 2, "both internal errors reported")
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
	end,

	["act-on 5: force-ending a generation whose start is running defers the start's cancellation (C6)"] = function()
		local w = world.new()
		local after = false
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.life.request(s, world.Want.Destroyed)
				coroutine.yield()
				after = true
			end,
		}))
		w.scripts.A.untick = function()
			error("untick threw")
		end
		w.scripts.A.leave = function()
			error("leave threw")
		end
		local ok, err = coroutine.resume(w.threads.A)
		spec.eq(ok, true, `the start thread ran to its next yield: {err}`)
		spec.eq(a.phase, Phase.Faulted, "force-ended")
		spec.eq(w.count("faulted:A"), 1, "ended once")
		spec.eq(w.count("cancel:A"), 0, "no cancel while the start thread runs")
		w.flush()
		spec.eq(w.count("cancel:A"), 1, "the deferred check cancelled it")
		spec.eq(coroutine.status(w.threads.A), "dead", "the start thread is dead")
		spec.eq(after, false, "never resumed past the yield")
	end,
}
