local spec = require("./lib/load")
local world = require("./lib/boot-world")

local function setup()
	local w = world()
	w.api = w.graph.module("")
	w.limits = w.graph.module("internal/reactive/limits")
	function w.log(entry)
		table.insert(w.events, entry)
	end
	function w.since(mark)
		local out = {}
		for i = mark + 1, #w.events do
			table.insert(out, w.events[i])
		end
		return out
	end
	return w
end

return {
	["rule 36 T2 effects created during construction, awake and start run once, after the commit"] = function()
		local w = setup()
		local api = w.api
		local Gauge, record = w.service("Gauge", {
			awake = function(self)
				api.effect(function()
					w.log(self.label())
				end)
				self.level:set(1)
			end,
			start = function(self)
				self.level:set(2)
				coroutine.yield()
				self.level:set(3)
			end,
		}, {
			init = function(self)
				self.level = api.state(0)
				self.label = api.computed(function()
					return `L{self.level()}`
				end)
			end,
		})
		local _ = Gauge
		w.boot()
		w.engine.flush()
		spec.list(w.events, { "booted", "L2" }, "nothing ran before the commit; one run with the latest value")
		assert(coroutine.resume(record.state.startThread))
		w.engine.flush()
		spec.list(w.events, { "booted", "L2", "L3" }, "a later start segment's write runs on the next flush")
	end,
	["rule 76 E2 boot holds the barrier again before admission and releases it after booted"] = function()
		local w = setup()
		local api = w.api
		local reactive = w.graph.module("internal/reactive/flush")
		reactive.useReactivePort(w.engine.port)
		reactive.releaseBarrier()
		local requested
		w.service("Maker", {
			awake = function()
				local before = #w.engine.deferred
				api.effect(function()
					w.log("effect")
				end)
				requested = #w.engine.deferred - before
			end,
		})
		local deferredAtBooted
		w.bootModule.ignite(w.engine.port, {
			name = "server",
			renders = false,
			booted = function()
				deferredAtBooted = #w.engine.deferred
				w.log("booted")
			end,
		}, {})
		spec.eq(requested, 0, "a barrier released before boot is held again during boot")
		spec.eq(#w.engine.deferred, deferredAtBooted + 1, "released after booted, requesting one flush")
		w.engine.flush()
		spec.list(w.events, { "booted", "effect" }, "the effect ran after the commit")
	end,
	["rule 36 T11 a module-level watch made during load runs after the commit with the baseline"] = function()
		local w = setup()
		local api = w.api
		local device = api.state("keyboard")
		local calls = {}
		api.scope(function()
			api.watch(device, function(value, previous)
				table.insert(calls, `{previous}>{value}`)
			end)
		end)
		spec.eq(#w.engine.deferred, 0, "no flush requested before boot")
		w.service("Input", {
			awake = function()
				device:set("touch")
				device:set("gamepad")
			end,
		})
		w.boot()
		spec.list(calls, {}, "nothing ran during boot")
		w.engine.flush()
		spec.list(calls, { "keyboard>gamepad" }, "once, with the final and the baseline values")
	end,
	["rule 36 T11 a failed boot releases the barrier after rollback; module roots keep their effects"] = function()
		local w = setup()
		local api = w.api
		api.scope(function()
			api.effect(function()
				w.log("module root")
			end)
		end)
		w.service("Good", {
			awake = function()
				api.effect(function()
					w.log("rolled back")
				end)
			end,
		})
		w.service("Bad", {
			awake = function()
				error("bad awake")
			end,
		})
		local ok, err = pcall(w.boot)
		spec.eq(ok, false, "boot failed")
		spec.has(err, "bad awake", "with the fault")
		w.engine.flush()
		spec.list(w.events, { "module root" }, "only the module root's effect ran")
	end,
	["rule 13 rule 16 rule 32 a run is its own outermost within, owned by its generation"] = function()
		local w = setup()
		local api = w.api
		local life = w.graph.module("internal/lifecycle")
		local seen = {}
		local _, record = w.service("Owner", {
			awake = function()
				api.effect(function()
					seen.current = life.current()
					seen.draining = life.isDraining()
				end)
			end,
		})
		w.boot()
		w.engine.flush()
		spec.eq(seen.current, record.state, "current() is the generation at the root of the owner chain")
		spec.eq(seen.draining, true, "inside its own within")
	end,
	["rule 13 T3 an effect's lifecycle requests settle before the next effect runs"] = function()
		local w = setup()
		local api = w.api
		local x, y = api.state(0), api.state(0)
		local Door = w.define("Door", {
			onDisable = function()
				w.log("door onDisable")
				y:set(1)
			end,
		})
		local instance = w.tagged("D1", "Door")
		w.service("S", {
			awake = function()
				api.effect(function()
					w.log(`G y={y()}`)
				end)
				api.effect(function()
					Door:get(instance):setEnabled(false)
					x:set(1)
					return function()
						w.log("R")
					end
				end)
				api.effect(function()
					w.log(`F x={x()} enabled={tostring(Door:get(instance):isEnabled())}`)
				end)
			end,
		})
		w.boot()
		local mark = #w.events
		w.engine.flush()
		spec.list(w.since(mark), {
			"G y=0",
			"door onDisable",
			"F x=1 enabled=false",
			"G y=1",
		}, "the drain ran after E's completion; a hook's write joined the next round")
	end,
	["rule 13 T3 a returned cleanup is registered before its owner's Disable step, and a re-enable waits for it"] = function()
		local w = setup()
		local api = w.api
		local locks = 0
		local Mover = w.define("Mover", {
			onEnable = function(self)
				w.log("onEnable")
				api.effect(function()
					locks += 1
					w.log("lock")
					if locks == 1 then
						self:setEnabled(false)
					end
					return function()
						w.log("unlock")
					end
				end)
			end,
			onDisable = function(self)
				w.log("onDisable")
				if locks == 1 then
					self:setEnabled(true)
				end
			end,
		})
		local _ = Mover
		w.tagged("M1", "Mover")
		w.boot()
		w.engine.flush()
		spec.list(
			w.events,
			{ "onEnable", "booted", "lock", "onDisable", "unlock", "onEnable", "lock" },
			"the unlock ran before the next onEnable"
		)
	end,
	["rule 28 T3 a release's lifecycle request during a disable applies after the step"] = function()
		local w = setup()
		local api = w.api
		local Other = w.define("Other", {
			onDisable = function()
				w.log("other onDisable")
			end,
		})
		local otherInstance = w.tagged("O1", "Other")
		local Holder = w.define("Holder", {
			onEnable = function()
				api.onCleanup(function()
					w.log("first registered")
				end)
				api.onCleanup(function()
					w.log("requests")
					Other:get(otherInstance):setEnabled(false)
				end)
			end,
			onDisable = function()
				w.log("holder onDisable")
			end,
		})
		local instance = w.tagged("H1", "Holder")
		w.boot()
		local mark = #w.events
		Holder:get(instance):setEnabled(false)
		spec.list(
			w.since(mark),
			{ "holder onDisable", "requests", "first registered", "other onDisable" },
			"queued like a request from onDisable itself"
		)
	end,
	["rule 21 T5 enable, disable, re-enable and destroy end each interval and the generation in order"] = function()
		local w = setup()
		local api = w.api
		local s = api.state(1)
		local Prompt = w.define("Prompt", {
			awake = function()
				api.onCleanup(function()
					w.log("generation release")
				end)
				api.effect(function()
					w.log(`G {s()}`)
				end)
			end,
			onEnable = function()
				for i = 1, 4 do
					api.onCleanup(function()
						w.log(`R{i}`)
					end)
				end
				api.effect(function()
					w.log(`E {s()}`)
					return function()
						w.log("E cleanup")
					end
				end)
			end,
			onDisable = function()
				w.log("onDisable")
			end,
			onDestroy = function()
				w.log("onDestroy")
			end,
		})
		local instance = w.tagged("P1", "Prompt")
		w.boot()
		w.engine.flush()
		local prompt = Prompt:get(instance)
		local mark = #w.events
		prompt:setEnabled(false)
		spec.list(w.since(mark), { "onDisable", "E cleanup", "R4", "R3", "R2", "R1" }, "disable")
		s:set(2)
		w.engine.flush()
		spec.list(w.since(mark + 6), { "G 2" }, "the generation's effect kept running; the interval's did not")
		prompt:setEnabled(true)
		w.engine.flush()
		mark = #w.events
		s:set(3)
		w.engine.flush()
		spec.list(w.since(mark), { "G 3", "E cleanup", "E 3" }, "one fresh interval effect")
		mark = #w.events
		prompt:remove()
		spec.list(w.since(mark), {
			"onDisable",
			"E cleanup",
			"R4",
			"R3",
			"R2",
			"R1",
			"onDestroy",
			"generation release",
		}, "destroy from enabled")
		s:set(4)
		w.engine.flush()
		spec.eq(#w.since(mark), 8, "nothing of a destroyed object runs again")
	end,
	["rule 21 rule 22 T5 observe deliveries during onEnable join the interval; the subscription outlives it"] = function()
		local w = setup()
		local api = w.api
		local s = api.state(1)
		w.define("Member", {}, { provides = { "I" } })
		w.tagged("A", "Member")
		w.tagged("B", "Member")
		local members = w.emit.resolveAll("I")
		local Watcher = w.define("Watcher", {
			onEnable = function()
				members:observe(function(member)
					local name = member.instance.Name
					api.effect(function()
						w.log(`run {name} {s()}`)
					end)
					api.onCleanup(function()
						w.log(`release {name}`)
					end)
				end)
			end,
		})
		w.boot()
		local instance = w.tagged("W1", "Watcher")
		w.engine.flush()
		local watcher = Watcher:get(instance)
		watcher:setEnabled(false)
		local mark = #w.events
		s:set(2)
		w.engine.flush()
		spec.eq(#w.since(mark), 0, "per-member effects ended at disable")
		spec.eq(table.find(w.events, "release A") ~= nil and table.find(w.events, "release B") ~= nil, true, "released")
		watcher:setEnabled(true)
		w.engine.flush()
		mark = #w.events
		s:set(3)
		w.engine.flush()
		local runs = w.since(mark)
		table.sort(runs)
		spec.list(runs, { "run A 3", "run B 3" }, "exactly one effect per member after re-enable")
		watcher:setEnabled(false)
		mark = #w.events
		w.tagged("C", "Member")
		w.engine.flush()
		spec.eq(table.find(w.since(mark), "run C 3") ~= nil, true, "the observe made in onEnable outlived the disable")
	end,
	["rule 22 T5 a late joiner is delivered into the generation; a per-member scope map ends it at disable"] = function()
		local w = setup()
		local api = w.api
		w.define("Member", {}, { provides = { "I" } })
		w.tagged("A", "Member")
		local members = w.emit.resolveAll("I")
		local Highlights = w.define("Highlights", {
			onEnable = function()
				local perMember = {}
				api.onCleanup(function()
					for _, dispose in perMember do
						dispose()
					end
					table.clear(perMember)
				end)
				api.onCleanup(members:observe(function(member)
					local name = member.instance.Name
					perMember[member] = api.scope(function()
						w.log(`highlight {name}`)
						api.onCleanup(function()
							w.log(`unhighlight {name}`)
						end)
					end)
				end, function(member)
					local dispose = perMember[member]
					perMember[member] = nil
					if dispose then
						dispose()
					end
				end))
			end,
		})
		w.boot()
		local instance = w.tagged("H1", "Highlights")
		w.engine.flush()
		w.tagged("B", "Member")
		w.engine.flush()
		local highlights = Highlights:get(instance)
		local mark = #w.events
		highlights:setEnabled(false)
		local ended = w.since(mark)
		table.sort(ended)
		spec.list(ended, { "unhighlight A", "unhighlight B" }, "both the existing and the late member ended")
		mark = #w.events
		highlights:setEnabled(true)
		local started = w.since(mark)
		table.sort(started)
		spec.list(started, { "highlight A", "highlight B" }, "exactly one per member after re-enable")
	end,
	["rule 13 T6 a yielding effect is completed before the drain its requests cause"] = function()
		local w = setup()
		local api = w.api
		local x, y = api.state(0), api.state(0)
		local c = api.computed(function()
			local v = x()
			if v == 0 then
				coroutine.yield()
			end
			return v
		end)
		local Door = w.define("Door", {
			onDisable = function()
				w.log("onDisable")
				x:set(1)
				local ok, value = pcall(c)
				w.log(`read {tostring(ok)} {tostring(value)}`)
			end,
		})
		local instance = w.tagged("D1", "Door")
		local fRuns = 0
		w.service("S", {
			awake = function()
				api.effect(function()
					api.onCleanup(function()
						w.log("E release")
					end)
					Door:get(instance):setEnabled(false)
					c()
					w.log("E continued")
				end)
				api.effect(function()
					y()
					fRuns += 1
				end)
			end,
		})
		w.boot()
		local mark = #w.events
		w.engine.flush()
		spec.list(w.since(mark), { "E release", "onDisable", "read true 1" }, "no cycle error, no stale frame")
		spec.eq(w.engine.logged("yielded"), 1, "the effect stopped with one diagnostic")
		x:set(0)
		y:set(1)
		w.engine.flush()
		spec.eq(fRuns, 2, "the next effect tracks only what it reads")
		local ok, err = pcall(api.effect, function() end)
		spec.eq(ok, false, "no owner leaked to this thread")
		spec.has(err, "needs an owner", "ownerless")
	end,
	["rule 13 T6 a yielding scope setup, release and outermost computed are finished before the drain"] = function()
		local w = setup()
		local api = w.api
		local x = api.state(0)
		local handlers = {}
		local Door = w.define("Door", {
			onDisable = function(self)
				handlers[self.instance.Name]()
			end,
		})
		local d1, d2, d3 = w.tagged("D1", "Door"), w.tagged("D2", "Door"), w.tagged("D3", "Door")
		w.boot()
		local released = false
		handlers.D1 = function()
			w.log(`D1 off, setup released {tostring(released)}`)
		end
		local ok, err = pcall(api.scope, function()
			api.onCleanup(function()
				released = true
			end)
			Door:get(d1):setEnabled(false)
			coroutine.yield()
		end)
		spec.eq(ok, false, "the scope call rethrows")
		spec.has(err, "yielded", "the yield")
		local reported = -1
		handlers.D2 = function()
			reported = w.engine.logged("yielded")
		end
		local dispose = api.scope(function()
			api.onCleanup(function()
				w.log("older release")
			end)
			api.onCleanup(function()
				Door:get(d2):setEnabled(false)
				coroutine.yield()
			end)
		end)
		dispose()
		spec.eq(reported, 1, "the yielding release was reported before the drain ran")
		local c = api.computed(function()
			local v = x()
			if v == 0 then
				Door:get(d3):setEnabled(false)
				coroutine.yield()
			end
			return v
		end)
		handlers.D3 = function()
			x:set(1)
			local readOk, value = pcall(c)
			w.log(`D3 read {tostring(readOk)} {tostring(value)}`)
		end
		pcall(c)
		spec.list(w.events, {
			"booted",
			"D1 off, setup released true",
			"older release",
			"D3 read true 1",
		}, "each body was complete before its drain")
	end,
	["rule 37 T4 a lane write is fresh for later members; its effect runs on the flush"] = function()
		local w = setup()
		local api = w.api
		local speed = api.state(0)
		local wrote = false
		w.define("Car", {
			update = function()
				if wrote then
					w.log(`later member reads {speed()}`)
				else
					wrote = true
					speed:set(5)
				end
			end,
		})
		w.service("Gauge", {
			awake = function()
				api.effect(function()
					w.log(`needle {speed()}`)
				end)
			end,
		})
		w.tagged("Van", "Car")
		w.tagged("Trailer", "Car")
		w.boot()
		w.engine.flush()
		local mark = #w.events
		w.engine.cycle(1 / 60, false)
		spec.list(w.since(mark), { "later member reads 5" }, "fresh read; the needle has not moved")
		w.engine.flush()
		spec.list(w.since(mark), { "later member reads 5", "needle 5" }, "the effect ran after the callback")
	end,
	["rule 38 T13 the render commit is reserved: a client boot binds no render step"] = function()
		local w = setup()
		local api = w.api
		spec.eq(w.limits.RENDER_COMMIT_PRIORITY, 2001, "Enum.RenderPriority.Last.Value + 1")
		w.service("Ui", {
			awake = function()
				api.effect(function() end)
			end,
		})
		w.boot(nil, true)
		w.engine.flush()
		spec.eq(#w.engine.renderSteps, 0, "v1 binds no render step")
	end,
	["rule 39 onShutdown has no owner: effect throws, writes work and session effects keep running"] = function()
		local w = setup()
		local api = w.api
		local phase = api.state("running")
		w.service("Session", {
			awake = function()
				api.effect(function()
					w.log(`phase {phase()}`)
				end)
			end,
			onShutdown = function()
				phase:set("closing")
				api.effect(function() end)
			end,
		})
		w.boot()
		w.engine.flush()
		w.closers[1]()
		spec.eq(w.engine.logged("needs an owner"), 1, "effect threw in onShutdown")
		w.engine.flush()
		spec.list(w.events, { "booted", "phase running", "phase closing" }, "the write ran the Subsystem's effect")
	end,
}
