local spec = require("./lib/load")
local world = require("./lib/boot-world")

return {
	["ignite wires all protected frame callers and behaviour membership lifecycle effects"] = function()
		local w = world()
		local events = {}
		local B = w.define("B", {
			renderPriority = 17,
			preRender = function(_, dt)
				table.insert(events, "render " .. dt)
			end,
			fixedUpdate = function(_, dt)
				table.insert(events, "fixed " .. dt)
			end,
			update = function(_, dt)
				table.insert(events, "update " .. dt)
			end,
			lateUpdate = function(_, dt)
				table.insert(events, "late " .. dt)
			end,
		})
		local instance = w.tagged("B1", "B")
		w.boot({ fixedRate = 40 }, true)
		local set = w.emit.resolveAll("B")
		spec.eq(set:size(), 1, "joined")
		w.engine.cycle(0.031, true)
		spec.list(events, { "render 0.031", "fixed 0.025", "update 0.031", "late 0.031" }, "caller wiring")
		local object = B:get(instance)
		object:setEnabled(false)
		spec.eq(set:size(), 0, "left on disable")
		table.clear(events)
		w.engine.cycle(0.031, true)
		spec.eq(#events, 0, "unticked")
		object:setEnabled(true)
		spec.eq(set:size(), 1, "rejoined")
		object:remove()
		spec.eq(set:size(), 0, "removed")
	end,
	["act-on 2 MAX_CHAINED_REQUESTS rejects boot and rolls back all generations"] = function()
		local w = world()
		local a, b
		local cycling = false
		local calls = 0
		local function guard()
			calls += 1
			assert(calls < 100000, "fixture guard must not win")
		end
		local _, ar = w.define("A", {
			onEnable = function()
				if cycling then
					guard()
					w.life.request(b, 0)
				end
			end,
			onDisable = function()
				if cycling then
					w.life.request(b, 1)
				end
			end,
			start = function(self)
				a = w.life.stateOf(self)
				cycling = true
				w.life.request(a, 1)
			end,
		})
		local _, br = w.define("B", {
			onEnable = function()
				if cycling then
					guard()
					w.life.request(a, 1)
				end
			end,
			onDisable = function()
				if cycling then
					w.life.request(a, 0)
				end
			end,
			awake = function(self)
				b = w.life.stateOf(self)
			end,
		})
		w.tagged("A1", "A")
		w.tagged("B1", "B")
		local ok, err = pcall(w.boot)
		spec.eq(ok, false, "request cap rejected")
		spec.has(err, "65536 requests", "cap cause")
		assert(calls < 100000, "cap beat guard")
		w.empty()
		spec.eq(next(ar.bound), nil, "A removed")
		spec.eq(next(br.bound), nil, "B removed")
	end,
	["boot constructs dependencies in order and initially tagged behaviours share the first batch"] = function()
		local w = world()
		w.service("Consumer", {
			start = function(self)
				table.insert(w.events, "sees " .. self.zones:size())
			end,
		}, {
			deps = { "Store", { all = "Zone" } },
			init = function(self, store, zones)
				assert(store.ready, "dependency constructed first")
				self.zones = zones
			end,
		})
		w.service("Store", {
			awake = function()
				table.insert(w.events, "awake Store")
			end,
		}, {
			init = function(self)
				self.ready = true
				table.insert(w.events, "construct Store")
			end,
		})
		w.define("Zone", {
			awake = function()
				table.insert(w.events, "awake Zone")
			end,
			onEnable = function()
				table.insert(w.events, "enable Zone")
			end,
		})
		w.tagged("Z1", "Zone")
		w.tagged("Z2", "Zone")
		w.boot()
		spec.list(w.events, {
			"construct Store",
			"awake Store",
			"awake Zone",
			"awake Zone",
			"enable Zone",
			"enable Zone",
			"sees 2",
			"booted",
		}, "one batch")
		local ok = pcall(w.boot)
		spec.eq(ok, false, "boot once")
	end,
	["a Subsystem awake fault rejects boot and ends the boot batch before dependents wake"] = function()
		local w = world()
		w.service("Broken", {
			awake = function()
				error("no config")
			end,
		})
		w.service("Dependent", {
			awake = function()
				error("dependent woke")
			end,
		}, { deps = { "Broken" } })
		w.define("Zone", {
			awake = function()
				error("zone woke")
			end,
		})
		local zone = w.tagged("Z1", "Zone")
		local ok, err = pcall(w.boot)
		spec.eq(ok, false, "rejected")
		spec.has(err, "Broken faulted in awake", "fault context")
		spec.eq(w.engine.logged("dependent woke"), 0, "dependent skipped")
		spec.eq(w.engine.logged("zone woke"), 0, "behaviour skipped")
		w.engine.removeTag(zone, "Zone")
		w.engine.addTag(zone, "Zone")
		w.engine.flush()
		w.empty()
		spec.eq(#w.events, 0, "no booted")
	end,
	["a Subsystem constructor that throws rejects boot before its dependent is constructed"] = function()
		local w = world()
		w.service("Broken", {}, {
			init = function()
				error("constructor threw")
			end,
		})
		w.service("Dependent", {}, {
			deps = { "Broken" },
			init = function()
				table.insert(w.events, "constructed")
			end,
		})
		local ok, err = pcall(w.boot)
		spec.eq(ok, false, "rejected")
		spec.has(err, "constructor threw", "cause")
		spec.eq(#w.events, 0, "dependent skipped")
		w.empty()
	end,
	["act-on 2 reviewer A and B rollback includes completed child batches and stops admission during teardown"] = function()
		local w = world()
		local child
		local suspended
		local Sleeper = w.define("Sleeper", {
			start = function()
				suspended = coroutine.running()
				coroutine.yield()
				error("cancelled start resumed")
			end,
		})
		local Ticker
		Ticker = w.define("Ticker", {
			update = function()
				error("tick survived")
			end,
			onDestroy = function()
				w.engine.addTag(w.engine.instance("late"), "Ticker")
				Ticker:get(child)
			end,
		})
		w.service("A", {
			start = function()
				child = w.tagged("child", "Ticker")
				Ticker:get(child)
				Sleeper:get(w.tagged("sleeping", "Sleeper"))
			end,
		})
		w.service("B", {
			start = function()
				error("B broke")
			end,
		})
		local ok, err = pcall(w.boot)
		spec.eq(ok, false, "rejected")
		spec.has(err, "B broke", "cause")
		w.engine.flush()
		w.engine.cycle(0.1, true)
		w.empty()
		spec.eq(coroutine.status(suspended), "dead", "rollback cancels suspended child start")
		spec.eq(w.engine.logged("tick survived"), 0, "no ticking")
	end,
	["act-on 10 start fault output retains its traceback and behaviour faults do not reject boot"] = function()
		local w = world()
		w.define("Broken", {
			start = function()
				error("start trace sentinel")
			end,
		})
		w.tagged("bad", "Broken")
		w.boot()
		spec.eq(w.engine.logged("start trace sentinel"), 1, "logged once")
		local message = w.engine.logs[1].message
		spec.has(message, "function start", "trace source")
		spec.has(message, "faulted in start", "hook")
		spec.eq(w.engine.logs[1].level, 1, "Error")
	end,
	["fixedRate defaults to 60 and a custom 40 Hz changes the fixed step"] = function()
		for _, rate in { 60, 40 } do
			local w = world()
			local steps = {}
			w.service("Fixed", {
				fixedUpdate = function(_, dt)
					table.insert(steps, dt)
				end,
			})
			w.boot(if rate == 60 then {} else { fixedRate = rate })
			w.engine.frame(1, 1 / rate)
			spec.eq(#steps, 1, "one step")
			spec.eq(steps[1], 1 / rate, "step")
		end
	end,
	["act-on 2 awake wave and child batch caps reject and roll back every admitted generation"] = function()
		for _, hook in { "awake", "start" } do
			local w = world()
			local count = 0
			local record
			local _, defined = w.define("Growing", {
				[hook] = function()
					count += 1
					if count < 2000 then
						local state = w.life.track(record, w.engine.instance(tostring(count)))
						record.bound[state.instance] = state
						w.life.admit({ state })
					end
				end,
				update = function()
					error("cap survivor ticked")
				end,
			})
			record = defined
			w.tagged("first", "Growing")
			local ok, err = pcall(w.boot)
			spec.eq(ok, false, "cap rejects " .. hook)
			spec.has(err, "kept creating objects", "cap message")
			assert(count < 2000, "runtime cap beat fixture guard")
			w.engine.flush()
			w.empty()
		end
	end,
	["rule 2 shutdown waits for dependents permits yields and runs independent hooks in parallel"] = function()
		local w = world()
		w.service("Store", {
			onShutdown = function()
				table.insert(w.events, "Store")
			end,
		})
		w.service("Save", {
			onShutdown = function()
				table.insert(w.events, "Save begin")
				coroutine.yield()
				table.insert(w.events, "Save end")
			end,
		}, { deps = { "Store" } })
		w.service("Other", {
			onShutdown = function()
				table.insert(w.events, "Other begin")
				coroutine.yield()
				table.insert(w.events, "Other end")
			end,
		})
		w.boot()
		spec.eq(#w.closers, 1, "one callback")
		table.clear(w.events)
		w.closers[1]()
		spec.list(
			w.events,
			{ "Save begin", "Other begin", "Save end", "Other end", "Store" },
			"dependent completion order"
		)
	end,
	["rule 2 S1 deadline is 25 seconds and starts remaining hooks below the engine 30 second limit"] = function()
		local w = world()
		local released, completed
		spec.eq(w.bootModule.SHUTDOWN_DEADLINE_SECONDS, 25, "margin")
		spec.eq(w.bootModule.SHUTDOWN_TIMEOUT_SECONDS, 28, "callback margin")
		w.service("Store", {
			onShutdown = function()
				released = w.now
				coroutine.yield()
				completed = w.now
				table.insert(w.events, "Store")
			end,
		})
		w.service("Stuck", {
			onShutdown = function()
				while true do
					coroutine.yield()
				end
			end,
		}, { deps = { "Store" } })
		w.boot()
		table.clear(w.events)
		w.now = 3600
		w.closers[1]()
		spec.list(w.events, { "Store" }, "remaining started")
		spec.eq(w.now, 3628, "bounded callback")
		spec.eq(released, 3625, "dependency released at deadline")
		spec.eq(completed, 3625.5, "released hook can finish after yielding")
	end,
	["post-commit Subsystem fault keeps binding live for newly tagged instances"] = function()
		local w = world()
		w.service("Faulted", {
			update = function()
				error("post-commit frame fault")
			end,
		})
		local _, record = w.define("Zone", {
			awake = function(self)
				table.insert(w.events, self.instance)
			end,
		})
		w.boot()
		spec.list(w.events, { "booted" }, "committed before fault")
		w.engine.frame(2, 0.031)
		spec.eq(w.engine.logged("post-commit frame fault"), 1, "Subsystem fault ran")
		local instance = w.tagged("late", "Zone")
		w.engine.flush()
		assert(record.bound[instance], "newly tagged instance bound after fault")
		spec.eq(#w.events, 2, "one new behaviour awakened after fault")
		spec.eq(w.events[2], instance, "new behaviour awakened on tagged instance")
	end,
	["rule 2 throwing shutdown logs continues and runs for faulted Subsystems but never binds on client"] = function()
		local w = world()
		w.service("Store", {
			onShutdown = function()
				table.insert(w.events, "Store")
			end,
		})
		w.service("Faulted", {
			update = function()
				error("frame fault")
			end,
			onShutdown = function()
				error("shutdown fault")
			end,
		}, { deps = { "Store" } })
		w.boot()
		w.engine.frame(2, 0.031)
		w.closers[1]()
		spec.eq(w.engine.logged("shutdown fault"), 1, "logged")
		spec.eq(w.events[2], "Store", "continued")
		local client = world()
		client.boot({}, true)
		spec.eq(#client.closers, 0, "S1 client never binds")
	end,
}
