local spec = require("./lib/load")
local life = require("./lib/reactive-life")

local ON_DISABLE, ON_DESTROY = 4, 5

local function intervalWork(w, label, s)
	w.api.onCleanup(function()
		w.log(`{label} release`)
	end)
	w.api.effect(function()
		s()
		w.log(`{label} run`)
		return function()
			w.log(`{label} cleanup`)
		end
	end)
end

return {
	["rule 21 rule 75 what onEnable creates ends right after onDisable; a re-enable rebuilds it once"] = function()
		local w = life()
		local s = w.api.state(1)
		local object = w.make("Door", {
			onEnable = function()
				intervalWork(w, "I", s)
			end,
		})
		w.boot(object)
		w.flush()
		local mark = #w.events
		w.life.request(object, w.Want.Disabled)
		spec.list(
			w.since(mark),
			{ "onDisable:Door", "I cleanup", "I release" },
			"the effect's run, then the releases, after onDisable"
		)
		s:set(2)
		w.flush()
		spec.list(w.since(mark), { "onDisable:Door", "I cleanup", "I release" }, "the interval's effect stopped")
		w.life.request(object, w.Want.Enabled)
		w.flush()
		mark = #w.events
		s:set(3)
		w.flush()
		spec.list(w.since(mark), { "I cleanup", "I run" }, "exactly one effect after re-enable")
	end,
	["rule 21 a fault in onEnable after registering releases what was registered"] = function()
		local w = life()
		local object = w.make("Door", {
			onEnable = function()
				for i = 1, 3 do
					w.api.onCleanup(function()
						w.log(`R{i}`)
					end)
				end
				error("closer failed")
			end,
		})
		w.boot(object)
		spec.list(
			w.events,
			{ "awake:Door", "onEnable:Door", "onDisable:Door", "R3", "R2", "R1", "onDestroy:Door", "ended:Door" },
			"the owed onDisable ends the interval; nothing leaks"
		)
	end,
	["rule 19 a computed's cached error read directly in a hook faults the hook's object"] = function()
		local w = life()
		local broken = w.api.computed(function()
			error("bad derived value")
		end)
		local object = w.make("Reader", {
			awake = function()
				broken()
			end,
		})
		w.boot(object)
		spec.eq(object.phase, 5, "faulted")
		spec.has(object.fault.error, "bad derived value", "by the computed's error")
	end,
	["rule 77 a drain requests the flush once, at the first enqueue, and runs no effect"] = function()
		local w = life()
		local runs = 0
		local object = w.make("Maker", {
			onEnable = function()
				for _ = 1, 3 do
					w.api.effect(function()
						runs += 1
					end)
				end
			end,
		})
		w.boot(object)
		spec.eq(runs, 0, "no effect ran inside the drain")
		spec.eq(#w.engine.deferred, 1, "one request; nothing at drain exit")
		w.flush()
		spec.eq(runs, 3, "all ran on the deferred flush")
	end,
	["rule 75 a cancelled yield in onEnable ends the interval"] = function()
		local w = life()
		local object = w.make("Door", {
			onEnable = function()
				w.api.onCleanup(function()
					w.log("released")
				end)
				coroutine.yield()
			end,
		})
		w.boot(object)
		spec.eq(table.find(w.events, "released") ~= nil, true, "the interval ended")
		spec.eq(object.phase, 5, "faulted")
	end,
	["rule 75 a force-end that skips onDisable still ends the interval through release"] = function()
		local w = life()
		local failing = false
		local object = w.make("Door", {
			lookup = function(hook)
				if failing and (hook == ON_DISABLE or hook == ON_DESTROY) then
					error("hook lookup failed")
				end
			end,
			onEnable = function()
				w.api.onCleanup(function()
					w.log("interval release")
				end)
			end,
		})
		w.boot(object)
		w.life.within(object, object, w.Hook.Frame, function()
			w.api.scope(function()
				w.api.onCleanup(function()
					w.log("later generation release")
				end)
			end)
		end)
		failing = true
		local mark = #w.events
		w.life.request(object, w.Want.Disabled)
		spec.eq(table.find(w.events, "onDisable:Door"), nil, "onDisable never ran")
		spec.list(
			w.since(mark),
			{ "interval release", "later generation release", "ended:Door" },
			"the interval ended first, before the generation's subscriptions"
		)
		spec.eq(object.phase, 5, "force-ended")
	end,
	["rule 21 T5 onEnable then onDisable on the same pooled worker: onDisable's creations are the generation's"] = function()
		local w = life()
		local s = w.api.state(1)
		local threads = {}
		local object = w.make("Door", {
			onEnable = function()
				threads.enable = coroutine.running()
			end,
			onDisable = function()
				threads.disable = coroutine.running()
				intervalWork(w, "D", s)
			end,
		})
		w.boot(object)
		w.life.request(object, w.Want.Disabled)
		spec.eq(threads.disable, threads.enable, "the same pooled worker ran both hooks")
		w.flush()
		local mark = #w.events
		s:set(2)
		w.flush()
		spec.list(w.since(mark), { "D cleanup", "D run" }, "survived the disable")
		w.life.request(object, w.Want.Enabled)
		w.life.request(object, w.Want.Disabled)
		spec.eq(table.find(w.since(mark), "D release"), nil, "not ended by a later disable")
		w.life.request(object, w.Want.Destroyed)
		spec.eq(table.find(w.since(mark), "D release") ~= nil, true, "ended at destroy")
	end,
	["rule 21 T5 a frame callback on the same worker after onEnable owns into the generation"] = function()
		local w = life()
		local s = w.api.state(1)
		local threads = {}
		local object = w.make("Door", {
			onEnable = function()
				threads.enable = coroutine.running()
			end,
		})
		w.boot(object)
		-- The drain that ran onEnable returned its own worker last, so the nested call takes onEnable's worker.
		w.life.within(nil, nil, w.Hook.Frame, function()
			w.life.within(object, object, w.Hook.Frame, function()
				threads.frame = coroutine.running()
				intervalWork(w, "F", s)
			end)
		end)
		spec.eq(threads.frame, threads.enable, "the same pooled worker")
		w.flush()
		w.life.request(object, w.Want.Disabled)
		local mark = #w.events
		s:set(2)
		w.flush()
		spec.list(w.since(mark), { "F cleanup", "F run" }, "the generation's effect keeps running while disabled")
		w.life.request(object, w.Want.Destroyed)
		spec.eq(table.find(w.since(mark), "F release") ~= nil, true, "ended at destroy")
	end,
	["rule 21 a disabled behaviour's generation effects keep running; destroy ends them"] = function()
		local w = life()
		local s = w.api.state(1)
		local object = w.make("Door", {
			awake = function()
				intervalWork(w, "G", s)
			end,
		})
		w.boot(object)
		w.flush()
		w.life.request(object, w.Want.Disabled)
		local mark = #w.events
		s:set(2)
		w.flush()
		spec.list(w.since(mark), { "G cleanup", "G run" }, "still running while disabled")
		w.life.request(object, w.Want.Destroyed)
		spec.list(
			w.since(mark),
			{ "G cleanup", "G run", "onDestroy:Door", "G cleanup", "G release", "ended:Door" },
			"after onDestroy: the effect, then the generation's releases"
		)
		s:set(3)
		w.flush()
		spec.eq(#w.since(mark), 6, "never runs again")
	end,
}
