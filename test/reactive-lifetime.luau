local spec = require("../packages/crucible/lune")
local realm = require("./lib/reactive-world")
local life = require("./lib/reactive-life")

local function record(events, label)
	return function()
		table.insert(events, label)
	end
end

return {
	["rule 23 rule 28 a root lifetime accepts releases from ownerless code and ends once"] = function()
		local r = realm()
		local handle = r.api.lifetime()
		local events = {}
		local release = record(events, "own")
		spec.eq(handle.ended, false, "a new root is live")
		spec.eq(handle:own(release), release, "own returns the value")
		handle:onCleanup(nil)
		handle:onCleanup(record(events, "cleanup"))
		spec.eq(#events, 0, "nothing releases before end")
		handle["end"](handle)
		spec.eq(handle.ended, true, "end latches the handle")
		spec.list(events, { "cleanup", "own" }, "releases run newest first")
		handle["end"](handle)
		spec.eq(#events, 2, "end is idempotent")
	end,
	["rule 23 behaviour lifetimes end after their onDisable and onDestroy hooks"] = function()
		local w = life()
		local awake, enabled
		local object = w.make("Door", {
			awake = function()
				awake = w.api.lifetime()
				awake:onCleanup(function()
					w.log("awake lifetime")
				end)
			end,
			onEnable = function()
				enabled = w.api.lifetime()
				enabled:onCleanup(function()
					w.log("enabled lifetime")
				end)
			end,
		})
		w.boot(object)
		local mark = #w.events
		w.life.request(object, w.Want.Disabled)
		spec.list(w.since(mark), { "onDisable:Door", "enabled lifetime" }, "the interval ends after its hook")
		spec.eq(enabled.ended, true, "the interval's handle ended")
		spec.eq(awake.ended, false, "the generation is still live")
		mark = #w.events
		w.life.request(object, w.Want.Destroyed)
		spec.list(
			w.since(mark),
			{ "onDestroy:Door", "awake lifetime", "ended:Door" },
			"the generation's handle ends after its hook"
		)
		spec.eq(awake.ended, true, "the generation's handle ended")
	end,
	["rule 23 an effect run ends its lifetime before the next run"] = function()
		local r = realm()
		local value = r.charm.atom(1)
		local events, handles = {}, {}
		local stop = r.root(function()
			r.api.effect(function()
				local n = value()
				local handle = r.api.lifetime()
				handles[n] = handle
				table.insert(events, `run {n}`)
				handle:onCleanup(record(events, `end {n}`))
			end)
		end)
		value(2)
		spec.list(events, { "run 1", "end 1", "run 2" }, "the first lifetime ended before the second run")
		spec.eq(handles[1].ended, true, "the previous run's handle ended")
		spec.eq(handles[2].ended, false, "the current run's handle is live")
		stop()
		spec.eq(handles[2].ended, true, "the current run's handle ended with the effect")
	end,
	["rule 23 a child lifetime ends with the parent or its own end call"] = function()
		local r = realm()
		local parent = r.api.lifetime()
		local early = parent:run(r.api.lifetime)
		local child = parent:run(r.api.lifetime)
		local events = {}
		parent:own(record(events, "parent"))
		early:own(record(events, "early"))
		child:own(record(events, "child"))
		early["end"](early)
		spec.list(events, { "early" }, "ending the child releases its own value")
		spec.eq(parent.ended, false, "ending a child leaves the parent live")
		parent["end"](parent)
		spec.list(events, { "early", "child", "parent" }, "the parent ends its live child and each value releases once")
		spec.eq(child.ended, true, "the child latch follows the parent")
	end,
	["rule 23 rule 28 an ended owner produces an ended handle whose run still executes"] = function()
		local r = realm()
		local parent = r.api.lifetime()
		parent["end"](parent)
		local child = parent:run(r.api.lifetime)
		spec.eq(child.ended, true, "the handle was born ended")
		local events = {}
		child:own(record(events, "own"))
		local stop = r.root(function()
			local value = child:run(function()
				local dispose = r.api.effect(record(events, "effect"))
				dispose()
				r.api.onCleanup(record(events, "cleanup"))
				table.insert(events, "returned")
				return 42
			end)
			spec.eq(value, 42, "an ended handle still returns the callback result")
			spec.list(events, { "own", "cleanup", "returned" }, "releases run immediately and the effect does nothing")
		end)
		stop()
		spec.eq(#events, 3, "nothing leaked to the caller")
	end,
	["rule 23 a lifetime run returns its result and keeps what it owns through a throw or yield"] = function()
		local r = realm()
		local handle = r.api.lifetime()
		local value = r.charm.atom(1)
		local events = {}
		local result = handle:run(function()
			r.api.effect(function()
				table.insert(events, `effect {value()}`)
				return record(events, "effect cleanup")
			end)
			return 42
		end)
		spec.eq(result, 42, "run returns the callback result")
		local ok, err = pcall(function()
			handle:run(function()
				r.api.onCleanup(record(events, "before throw"))
				error("run failed")
			end)
		end)
		spec.eq(ok, false, "run rethrows")
		spec.has(err, "run failed", "with the callback's error")
		local yielded, yieldErr = pcall(function()
			handle:run(function()
				r.api.onCleanup(record(events, "before yield"))
				coroutine.yield()
			end)
		end)
		spec.eq(yielded, false, "run rejects a yield")
		spec.has(yieldErr, "yielded", "with the yield failure")
		spec.eq(handle.ended, false, "the lifetime stays live")
		spec.list(events, { "effect 1" }, "nothing was released by the failed runs")
		handle["end"](handle)
		value(2)
		spec.list(events, { "effect 1", "effect cleanup", "before yield", "before throw" }, "end releases everything")
	end,
}
