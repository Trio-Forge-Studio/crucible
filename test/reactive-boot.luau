local spec = require("./lib/load")
local world = require("./lib/boot-world")

local function setup()
	local w = world()
	w.api = w.graph.module("")
	w.charm = w.graph.package("@rbxts/charm/src")
	w.ownership = w.graph.module("internal/reactive/ownership")
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
				self.level(1)
			end,
			start = function(self)
				self.level(2)
				coroutine.yield()
				self.level(3)
			end,
		}, {
			init = function(self)
				self.level = w.charm.atom(0)
				self.label = w.charm.computed(function()
					return `L{self.level()}`
				end)
			end,
		})
		local _ = Gauge
		w.boot()
		spec.list(w.events, { "booted", "L2" }, "nothing ran before the commit; one run with the latest value")
		assert(coroutine.resume(record.state.startThread))
		spec.list(w.events, { "booted", "L2", "L3" }, "a later start segment's write runs its effect at once")
	end,
	["rule 76 E2 an effect created during boot runs after booted, not inside effect()"] = function()
		local w = setup()
		local api = w.api
		local ranAtCreation
		w.service("Maker", {
			awake = function()
				local before = #w.events
				api.effect(function()
					w.log("effect")
				end)
				ranAtCreation = #w.events - before
			end,
		})
		w.boot()
		spec.eq(ranAtCreation, 0, "effect() only queued it")
		spec.list(w.events, { "booted", "effect" }, "the effect ran after the commit")
	end,
	["rule 76 E2 boot runs its effects after the network commit, not before it"] = function()
		local w = setup()
		local api = w.api
		w.service("Maker", {
			awake = function()
				api.effect(function()
					w.log("effect")
				end)
			end,
		})
		w.boot(nil, false, {
			commit = function()
				w.log("commit")
			end,
		})
		spec.list(w.events, { "booted", "commit", "effect" }, "the effect ran after the commit")
	end,
	["rule 76 E2 a network commit that throws rolls back, and the effects boot held never run"] = function()
		local w = setup()
		local api = w.api
		w.service("Maker", {
			awake = function()
				api.effect(function()
					w.log("effect")
				end)
				api.onCleanup(function()
					w.log("released")
				end)
			end,
		})
		local ok, err = pcall(w.boot, nil, false, {
			commit = function()
				error("commit fault")
			end,
		})
		spec.eq(ok, false, "boot failed")
		spec.has(err, "commit fault", "with the commit's fault")
		spec.list(w.events, { "booted", "released" }, "the rollback ended the service; its effect never ran")
	end,
	["rule 36 T11 a module-level watch made during load runs when boot's hold ends, with the baseline"] = function()
		local w = setup()
		local api = w.api
		w.ownership.holdEffects()
		local device = w.charm.atom("keyboard")
		local calls = {}
		api.scope(function()
			api.watch(device, function(value, previous)
				table.insert(calls, `{previous}>{value}`)
			end)
		end)
		w.service("Input", {
			awake = function()
				device("touch")
				device("gamepad")
				table.insert(calls, "awake")
			end,
		})
		w.boot()
		spec.list(calls, { "awake" }, "under a hold boot did not open, nothing ran at the commit")
		w.ownership.releaseEffects(true)
		spec.list(
			calls,
			{ "awake", "keyboard>gamepad" },
			"once, when the hold ended, with the final and the baseline values"
		)
	end,
	["rule 36 T11 a failed boot discards the effects it held, module roots included"] = function()
		local w = setup()
		local api = w.api
		w.ownership.holdEffects()
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
				api.onCleanup(function()
					w.log("released")
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
		spec.list(w.events, { "released" }, "the rollback ended the service, and no held effect ran")
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
		spec.eq(seen.current, record.state, "current() is the generation at the root of the owner chain")
		spec.eq(seen.draining, true, "inside its own within")
	end,
	["rule 13 T3 an effect's lifecycle requests settle before the next effect runs"] = function()
		local w = setup()
		local api = w.api
		local x, y = w.charm.atom(0), w.charm.atom(0)
		local Door = w.define("Door", {
			onDisable = function()
				w.log("door onDisable")
				y(1)
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
					x(1)
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
		spec.list(w.events, {
			"booted",
			"G y=0",
			"door onDisable",
			"F x=1 enabled=false",
			"G y=1",
		}, "the drain ran after E's run; a hook's write reached G when the commit's batch ended")
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
		local s = w.charm.atom(1)
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
		local prompt = Prompt:get(instance)
		local mark = #w.events
		prompt:setEnabled(false)
		spec.list(w.since(mark), { "onDisable", "E cleanup", "R4", "R3", "R2", "R1" }, "disable")
		s(2)
		spec.list(w.since(mark + 6), { "G 2" }, "the generation's effect kept running; the interval's did not")
		prompt:setEnabled(true)
		mark = #w.events
		s(3)
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
		s(4)
		spec.eq(#w.since(mark), 8, "nothing of a destroyed object runs again")
	end,
	["rule 21 rule 22 T5 observe deliveries during onEnable join the interval; the subscription outlives it"] = function()
		local w = setup()
		local api = w.api
		local s = w.charm.atom(1)
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
		s(2)
		spec.eq(#w.since(mark), 0, "per-member effects ended at disable")
		spec.eq(table.find(w.events, "release A") ~= nil and table.find(w.events, "release B") ~= nil, true, "released")
		watcher:setEnabled(true)
		mark = #w.events
		s(3)
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
	["rule 13 T6 a yielding effect run is cancelled before the drain its requests cause"] = function()
		local w = setup()
		local api = w.api
		local x = w.charm.atom(0)
		local Door = w.define("Door", {
			onDisable = function()
				w.log("onDisable")
			end,
		})
		local instance = w.tagged("D1", "Door")
		w.boot()
		api.scope(function()
			api.effect(function()
				api.onCleanup(function()
					w.log("E release")
				end)
				if x() == 1 then
					Door:get(instance):setEnabled(false)
					coroutine.yield()
					w.log("E continued")
				end
			end)
		end)
		local ok, err = pcall(x, 1)
		spec.eq(ok, false, "the write threw")
		spec.has(err, "yielded", "the yield")
		spec.list(w.events, { "booted", "E release", "onDisable" }, "the request applied once the run was cancelled")
		local created, ownerError = pcall(api.effect, function() end)
		spec.eq(created, false, "no owner leaked to this thread")
		spec.has(ownerError, "needs an owner", "ownerless")
	end,
	["rule 13 T6 a yielding scope setup and release are finished before the drain"] = function()
		local w = setup()
		local api = w.api
		local handlers = {}
		local Door = w.define("Door", {
			onDisable = function(self)
				handlers[self.instance.Name]()
			end,
		})
		local d1, d2 = w.tagged("D1", "Door"), w.tagged("D2", "Door")
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
		spec.list(w.events, {
			"booted",
			"D1 off, setup released true",
			"older release",
		}, "each body was complete before its drain")
	end,
	["rule 37 T4 a lane write is fresh for later members; its effect runs when the writer's callback returns"] = function()
		local w = setup()
		local api = w.api
		local speed = w.charm.atom(0)
		local wrote = false
		w.define("Car", {
			update = function()
				if wrote then
					w.log(`later member reads {speed()}`)
				else
					wrote = true
					speed(5)
					w.log("wrote")
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
		local mark = #w.events
		w.engine.cycle(1 / 60, false)
		spec.list(
			w.since(mark),
			{ "wrote", "needle 5", "later member reads 5" },
			"the writer's batch ended before the next member ran"
		)
	end,
	["rule 38 T13 a client boot binds no render step"] = function()
		local w = setup()
		local api = w.api
		w.service("Ui", {
			awake = function()
				api.effect(function() end)
			end,
		})
		w.boot(nil, true)
		spec.eq(#w.engine.renderSteps, 0, "v1 binds no render step")
	end,
	["rule 39 onShutdown has no owner: effect throws, writes work and session effects keep running"] = function()
		local w = setup()
		local api = w.api
		local phase = w.charm.atom("running")
		w.service("Session", {
			awake = function()
				api.effect(function()
					w.log(`phase {phase()}`)
				end)
			end,
			onShutdown = function()
				phase("closing")
				api.effect(function() end)
			end,
		})
		w.boot()
		w.closers[1]()
		spec.eq(w.engine.logged("needs an owner"), 1, "effect threw in onShutdown")
		spec.list(w.events, { "booted", "phase running", "phase closing" }, "the write ran the Subsystem's effect")
	end,
}
