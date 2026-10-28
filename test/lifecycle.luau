local spec = require("./lib/load")
local world = require("./lib/world")

local Phase, Want, Hook, Kind = world.Phase, world.Want, world.Hook, world.Kind

local function guardedEnables(limit: number)
	local enables = 0
	return function()
		enables += 1
		if enables > limit then
			error(`unbounded: more than {limit} enables`)
		end
	end
end

return {
	["a batch runs construct, awake, enable, join, start, then frame hooks"] = function()
		local w = world.new()
		local a = w.boot(w.make("A"))
		spec.list(w.events, { "construct:A", "awake:A", "onEnable:A", "join:A", "start:A", "tick:A" }, "events")
		spec.eq(a.phase, Phase.Enabled, "phase")
	end,

	["a throw in update while Enabled runs onDisable then onDestroy and ends Faulted"] = function()
		local w = world.new()
		local a = w.boot(w.make("A"))
		w.clear()
		w.life.fault(a, Hook.Frame, "update threw")
		spec.list(w.events, { "untick:A", "leave:A", "onDisable:A", "onDestroy:A", "faulted:A" }, "events")
		spec.eq(a.phase, Phase.Faulted, "phase")
		spec.eq(a.fault.hook, Hook.Frame, "primary fault")
		spec.eq(w.life.stateOf(a.object), nil, "state released")
		w.clear()
		w.life.request(a, Want.Enabled)
		w.life.fault(a, Hook.Frame, "again")
		spec.eq(#w.events, 0, "Faulted is terminal")
		spec.eq(#a.fault.cleanup, 0, "no failure attached after the end")
	end,

	["a throw part way through onEnable still runs exactly one onDisable"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			onEnable = function()
				error("half enabled")
			end,
		}))
		spec.list(
			w.events,
			{ "construct:A", "awake:A", "onEnable:A", "onDisable:A", "onDestroy:A", "faulted:A" },
			"events"
		)
		spec.eq(a.fault.hook, Hook.OnEnable, "primary fault")
	end,

	["failures during fault cleanup attach to the first fault and never stop cleanup"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			onEnable = function()
				error("first")
			end,
			onDisable = function()
				error("second")
			end,
			onDestroy = function()
				error("third")
			end,
		}))
		spec.eq(w.count("onDisable:A"), 1, "onDisable runs once")
		spec.eq(w.count("onDestroy:A"), 1, "onDestroy runs once")
		spec.eq(w.count("faulted:A"), 1, "ended once")
		spec.eq(a.fault.hook, Hook.OnEnable, "primary fault")
		spec.eq(#a.fault.cleanup, 2, "attached failures")
		spec.eq(a.fault.cleanup[1].hook, Hook.OnDisable, "first attached")
		spec.eq(a.fault.cleanup[2].hook, Hook.OnDestroy, "second attached")
	end,

	["a throw in a normal onDisable faults and still destroys once"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			onDisable = function()
				error("disable threw")
			end,
		}))
		w.clear()
		w.life.request(a, Want.Disabled)
		spec.list(w.events, { "untick:A", "leave:A", "onDisable:A", "onDestroy:A", "faulted:A" }, "events")
		spec.eq(a.fault.hook, Hook.OnDisable, "primary fault")
	end,

	["a constructor throw gives no hooks and no onDestroy, and its batch carries on"] = function()
		local w = world.new()
		local a, b = w.boot(
			w.make("A", {
				construct = function()
					error("constructor threw")
				end,
			}),
			w.make("B")
		)
		spec.list(
			w.events,
			{ "construct:A", "faulted:A", "construct:B", "awake:B", "onEnable:B", "join:B", "start:B", "tick:B" },
			"events"
		)
		spec.eq(a.phase, Phase.Faulted, "phase")
		spec.eq(a.fault.hook, Hook.Construct, "primary fault")
		spec.eq(a.object, nil, "no object")
		spec.eq(b.phase, Phase.Enabled, "peer")
	end,

	["a throw in awake still runs onDestroy, and no onEnable"] = function()
		local w = world.new()
		w.boot(w.make("A", {
			awake = function()
				error("awake threw")
			end,
		}))
		spec.list(w.events, { "construct:A", "awake:A", "onDestroy:A", "faulted:A" }, "events")
	end,

	["a throw in start faults with start as the primary"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			start = function()
				error("start threw")
			end,
		}))
		spec.list(w.events, {
			"construct:A",
			"awake:A",
			"onEnable:A",
			"join:A",
			"start:A",
			"leave:A",
			"onDisable:A",
			"onDestroy:A",
			"faulted:A",
		}, "events")
		spec.eq(a.fault.hook, Hook.Start, "primary fault")
	end,

	["destroy while start is suspended cancels it"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", { start = "suspend" }))
		spec.list(
			w.events,
			{ "construct:A", "awake:A", "onEnable:A", "join:A", "start:A" },
			"no frame hooks before start returns"
		)
		w.clear()
		w.life.request(a, Want.Destroyed)
		spec.list(w.events, { "leave:A", "onDisable:A", "cancel:A", "onDestroy:A", "destroyed:A" }, "events")
		w.clear()
		w.life.finishStart(a, true, nil)
		spec.eq(#w.events, 0, "a start that outlived its generation is ignored")
		spec.eq(a.phase, Phase.Destroyed, "phase")
	end,

	["disable while start is suspended keeps start running and ticks after re-enable"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", { start = "suspend" }))
		w.clear()
		w.life.request(a, Want.Disabled)
		spec.list(w.events, { "leave:A", "onDisable:A" }, "disable does not cancel start")
		w.clear()
		w.life.finishStart(a, true, nil)
		spec.eq(#w.events, 0, "a disabled object does not tick")
		w.life.request(a, Want.Enabled)
		spec.list(w.events, { "onEnable:A", "join:A", "tick:A" }, "re-enable")
	end,

	["re-entrant creation during awake drains before any enable or start"] = function()
		local w = world.new()
		w.boot(w.make("A", {
			awake = function()
				w.life.admit({ w.make("B") })
			end,
		}))
		spec.list(w.events, {
			"construct:A",
			"awake:A",
			"construct:B",
			"awake:B",
			"onEnable:A",
			"join:A",
			"onEnable:B",
			"join:B",
			"start:A",
			"tick:A",
			"start:B",
			"tick:B",
		}, "events")
	end,

	["creation during start runs as its own batch at once"] = function()
		local w = world.new()
		w.boot(w.make("A", {
			start = function()
				w.life.admit({ w.make("C") })
			end,
		}))
		spec.list(w.events, {
			"construct:A",
			"awake:A",
			"onEnable:A",
			"join:A",
			"start:A",
			"construct:C",
			"awake:C",
			"onEnable:C",
			"join:C",
			"start:C",
			"tick:C",
			"tick:A",
		}, "events")
	end,

	["an awake closure that never settles stops after 1,024 waves and drops the last wave"] = function()
		local w = world.new()
		local function chain(i)
			return w.make(`N{i}`, {
				awake = function()
					w.life.admit({ chain(i + 1) })
				end,
			})
		end
		local ok, err = pcall(w.life.admit, { chain(1) })
		spec.eq(ok, true, `admit returned: {err}`)
		spec.eq(#w.failures, 1, "one failure reported")
		spec.has(w.failures[1], "1024 waves", "the failure names the cap")
		local awakes, ticks = 0, 0
		for _, e in w.events do
			if string.sub(e, 1, 6) == "awake:" then
				awakes += 1
			elseif string.sub(e, 1, 5) == "tick:" then
				ticks += 1
			end
		end
		spec.eq(awakes, 1024, "awake waves run")
		spec.eq(ticks, 1024, "every member still enabled and started")
		spec.eq(w.count("construct:N1025"), 0, "the dropped wave is never constructed")
		spec.eq(w.count("destroyed:N1025"), 1, "the dropped wave ends with no hooks")
	end,

	["a request made twice converges to the same state"] = function()
		local w = world.new()
		local a = w.boot(w.make("A"))
		w.clear()
		w.life.request(a, Want.Disabled)
		w.life.request(a, Want.Disabled)
		spec.list(w.events, { "untick:A", "leave:A", "onDisable:A" }, "disable twice")
		spec.eq(a.phase, Phase.Disabled, "phase")
		w.clear()
		w.life.request(a, Want.Enabled)
		w.life.request(a, Want.Enabled)
		spec.list(w.events, { "onEnable:A", "join:A", "tick:A" }, "enable twice")
		w.clear()
		w.life.request(a, Want.Destroyed)
		w.life.request(a, Want.Destroyed)
		w.life.request(a, Want.Enabled)
		spec.list(
			w.events,
			{ "untick:A", "leave:A", "onDisable:A", "onDestroy:A", "destroyed:A" },
			"destroy twice, then enable"
		)
		spec.eq(a.phase, Phase.Destroyed, "phase")
	end,

	["a request from inside a hook takes effect after the hook returns"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			onEnable = function(s)
				w.life.request(s, Want.Disabled)
			end,
		}))
		spec.list(w.events, { "construct:A", "awake:A", "onEnable:A", "onDisable:A" }, "never joins or starts")
		spec.eq(a.phase, Phase.Disabled, "phase")
	end,

	["an object disabled before its enable sweep runs onEnable and start at its first enable"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			awake = function(s)
				w.life.request(s, Want.Disabled)
			end,
		}))
		spec.list(w.events, { "construct:A", "awake:A" }, "begins disabled with no hooks")
		w.clear()
		w.life.request(a, Want.Enabled)
		spec.list(w.events, { "onEnable:A", "join:A", "start:A", "tick:A" }, "first enable")
	end,

	["removal before awake drops the object with no hooks"] = function()
		local w = world.new()
		local b
		w.boot(w.make("A", {
			awake = function()
				b = w.make("B")
				w.life.admit({ b })
				w.life.request(b, Want.Destroyed)
			end,
		}))
		spec.eq(w.count("destroyed:B"), 1, "dropped")
		spec.eq(w.count("construct:B"), 0, "never constructed")
		spec.eq(b.phase, Phase.Destroyed, "phase")
	end,

	["terminal phases have no step for any request"] = function()
		local life = spec.fresh("internal/lifecycle")
		for _, phase in { Phase.Faulted, Phase.Destroyed } do
			for _, want in { Want.Enabled, Want.Disabled, Want.Destroyed } do
				local state = { phase = phase, start = 0, gate = 4, want = want, joined = false, ticking = false }
				spec.eq(life.nextStep(state), nil, "terminal")
			end
		end
	end,

	["destroy requested from a worker the start thread resumed completes teardown (reviewer B's cancel probe)"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.result = w.runner.call(function()
					w.life.request(s, Want.Destroyed)
				end, nil, nil)
			end,
		}))
		w.clear()
		local ok, err = coroutine.resume(w.threads.A)
		spec.eq(ok, true, `the start thread resumed without an error: {err}`)
		spec.eq(w.result, 0, "the destroying callback returned Ok")
		spec.list(w.events, { "leave:A", "onDisable:A", "onDestroy:A", "destroyed:A" }, "teardown")
		spec.eq(a.phase, Phase.Destroyed, "phase")
		spec.eq(a.queued, false, "not left on the work stack")
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
		spec.eq(coroutine.status(w.threads.A), "dead", "the start thread ran to its end")
		w.flush()
		spec.eq(w.count("cancel:A"), 0, "a finished start thread is not cancelled")
		w.clear()
		w.boot(w.make("B"))
		spec.eq(w.count("tick:B"), 1, "a later batch still runs")
	end,

	["a start thread that destroys its own object is cancelled at its next yield"] = function()
		local w = world.new()
		local reached = false
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.life.request(s, Want.Destroyed)
				coroutine.yield()
				reached = true
			end,
		}))
		w.clear()
		coroutine.resume(w.threads.A)
		spec.list(w.events, { "leave:A", "onDisable:A", "onDestroy:A", "destroyed:A" }, "teardown while start runs")
		w.flush()
		spec.eq(w.count("cancel:A"), 1, "cancelled once it suspended")
		spec.eq(coroutine.status(w.threads.A), "dead", "status")
		spec.eq(reached, false, "never resumed")
		spec.eq(a.phase, Phase.Destroyed, "phase")
	end,

	["a request from a hook for another object applies after that hook returns"] = function()
		local w = world.new()
		local a, b = w.boot(w.make("A"), w.make("B"))
		w.scripts.A.onDisable = function()
			w.life.request(b, Want.Destroyed)
			table.insert(w.events, "onDisable:A returned")
		end
		w.clear()
		w.life.request(a, Want.Disabled)
		spec.list(w.events, {
			"untick:A",
			"leave:A",
			"onDisable:A",
			"onDisable:A returned",
			"untick:B",
			"leave:B",
			"onDisable:B",
			"onDestroy:B",
			"destroyed:B",
		}, "events")
	end,

	["an onEnable and an onDisable that undo each other fault after the bound and still clean up"] = function()
		local w = world.new()
		local a = w.boot(w.make("A"))
		local guard = guardedEnables(500)
		w.scripts.A.onEnable = function(s)
			guard()
			w.life.request(s, Want.Disabled)
		end
		w.scripts.A.onDisable = function(s)
			w.life.request(s, Want.Enabled)
		end
		w.clear()
		w.life.request(a, Want.Disabled)
		spec.eq(a.phase, Phase.Faulted, "phase")
		spec.eq(a.fault.hook, Hook.Settle, "the primary fault names the bound")
		spec.eq(w.count("onDisable:A"), w.count("onEnable:A") + 1, "every enable interval closed exactly once")
		spec.eq(w.count("onDestroy:A"), 1, "onDestroy ran once")
		spec.eq(w.count("faulted:A"), 1, "ended once")
		spec.eq(w.count("onEnable:A") <= 32, true, `transitions stayed bounded: {w.count("onEnable:A")} enables`)
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
	end,

	["C5: 1,000 generations in one batch that each toggle a watcher once never fault"] = function()
		local w = world.new()
		local watcher = w.boot(w.make("W"))
		local function toggle()
			w.life.request(watcher, if watcher.want == Want.Enabled then Want.Disabled else Want.Enabled)
		end
		local batch = {}
		for i = 1, 1000 do
			table.insert(batch, w.make(`N{i}`, { join = toggle }))
		end
		w.life.admit(batch)
		local faulted = 0
		for _, s in batch do
			if s.fault ~= nil then
				faulted += 1
			end
		end
		spec.eq(faulted, 0, "faulted generations")
		spec.eq(watcher.fault, nil, "the watcher's fault")
		spec.eq(watcher.phase, Phase.Enabled, "the watcher ends enabled after an even number of toggles")
		spec.eq(w.count("onEnable:W"), 501, "the watcher's enables")
	end,

	["C5 gap: two objects that toggle each other stop at the chained-request cap with a logged failure"] = function()
		local w = world.new()
		local a, b = w.boot(w.make("A"), w.make("B"))
		local guard = guardedEnables(100000)
		w.scripts.A.onDisable = function()
			w.life.request(b, Want.Disabled)
		end
		w.scripts.B.onDisable = function()
			w.life.request(a, Want.Enabled)
		end
		w.scripts.A.onEnable = function()
			guard()
			w.life.request(b, Want.Enabled)
		end
		w.scripts.B.onEnable = function()
			guard()
			w.life.request(a, Want.Disabled)
		end
		w.life.request(a, Want.Disabled)
		local settled = (a.fault and a.fault.hook == Hook.Settle) or (b.fault and b.fault.hook == Hook.Settle)
		spec.eq(settled, true, `a Settle fault: A {a.fault and a.fault.error}, B {b.fault and b.fault.error}`)
		spec.eq(#w.failures, 1, "failures reported")
		spec.has(w.failures[1], "65536 requests", "the failure names the cap")
		for _, s in { a, b } do
			local name = s.record.name
			local open = if s.phase == Phase.Enabled then 1 else 0
			spec.eq(w.count(`onEnable:{name}`) - w.count(`onDisable:{name}`), open, `{name}'s open enable intervals`)
		end
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
	end,

	["C5 gap: a three-object cycle whose requests coalesce on queued objects stops at the chained-request cap"] = function()
		local w = world.new()
		local a, b, c = w.boot(w.make("A"), w.make("B"), w.make("C"))
		local guard = guardedEnables(200000)
		local function toggle(s)
			w.life.request(s, if s.want == Want.Enabled then Want.Disabled else Want.Enabled)
		end
		w.scripts.A.onEnable = function()
			guard()
			w.life.request(c, Want.Enabled)
		end
		w.scripts.A.onDisable = function()
			w.life.request(b, Want.Disabled)
			w.life.request(c, Want.Disabled)
			w.life.request(b, Want.Enabled)
		end
		w.scripts.B.onEnable = function()
			guard()
			w.life.request(b, Want.Disabled)
			w.life.request(a, Want.Enabled)
		end
		w.scripts.B.onDisable = function()
			w.life.request(c, Want.Disabled)
		end
		w.scripts.C.onEnable = function()
			guard()
			w.life.request(a, Want.Disabled)
		end
		w.scripts.C.onDisable = function()
			toggle(b)
			toggle(a)
		end
		w.life.request(a, Want.Disabled)
		local settled = 0
		for _, s in { a, b, c } do
			if s.fault ~= nil then
				spec.eq(s.fault.hook, Hook.Settle, `{s.record.name}'s fault is the bound, not the guard`)
				settled += 1
			end
		end
		spec.eq(settled > 0, true, "the cycle faulted with Settle")
		spec.eq(w.life.isDraining(), false, "the reconciler is idle")
	end,

	["C5 gap: a ring of 1,100 objects that each toggle the next stops at the chained-request cap"] = function()
		local w = world.new()
		local nodes = {}
		for i = 1, 1100 do
			nodes[i] = w.make(`N{i}`)
		end
		w.life.admit(nodes)
		local calls = 0
		for i = 1, 1100 do
			local function toggleNext()
				calls += 1
				if calls > 200000 then
					error("unbounded: more than 200000 hook calls")
				end
				local s = nodes[i % 1100 + 1]
				w.life.request(s, if s.want == Want.Enabled then Want.Disabled else Want.Enabled)
			end
			w.scripts[`N{i}`].onEnable = toggleNext
			w.scripts[`N{i}`].onDisable = toggleNext
		end
		w.life.request(nodes[1], Want.Disabled)
		local settled = 0
		for _, s in nodes do
			if s.fault ~= nil then
				spec.eq(s.fault.hook, Hook.Settle, `{s.record.name}'s fault is the bound, not the guard`)
				settled += 1
			end
		end
		spec.eq(settled, 1, "one generation faulted with Settle, which breaks the ring")
	end,

	["C5: 1,000 objects in one cascade that each toggle a shared target once never fault it"] = function()
		local w = world.new()
		local target = w.boot(w.make("T"))
		local nodes = {}
		for i = 1, 1000 do
			nodes[i] = w.make(`N{i}`)
		end
		w.life.admit(nodes)
		for i = 1, 1000 do
			w.scripts[`N{i}`].onDisable = function()
				w.life.request(target, if target.want == Want.Enabled then Want.Disabled else Want.Enabled)
				if i < 1000 then
					w.life.request(nodes[i + 1], Want.Disabled)
				end
			end
		end
		w.life.request(nodes[1], Want.Disabled)
		spec.eq(target.fault, nil, "the target's fault")
		spec.eq(w.count("onDisable:T"), 500, "the target's disables")
		spec.eq(nodes[1000].phase, Phase.Disabled, "the cascade reached its end")
		for i, n in nodes do
			spec.eq(n.fault, nil, `N{i} fault`)
		end
	end,

	["C5: an object that undoes itself inside a large cascade faults within its own bound"] = function()
		local w = world.new()
		local nodes = {}
		for i = 1, 200 do
			nodes[i] = w.make(`N{i}`)
		end
		local a = w.make("A")
		table.insert(nodes, a)
		w.life.admit(nodes)
		for i = 1, 199 do
			w.scripts[`N{i}`].onDisable = function()
				w.life.request(nodes[i + 1], Want.Disabled)
			end
		end
		w.scripts.N200.onDisable = function()
			w.life.request(a, Want.Disabled)
		end
		local guard = guardedEnables(40)
		w.scripts.A.onEnable = function(s)
			guard()
			w.life.request(s, Want.Disabled)
		end
		w.scripts.A.onDisable = function(s)
			w.life.request(s, Want.Enabled)
		end
		w.life.request(nodes[1], Want.Disabled)
		spec.eq(a.fault and a.fault.hook, Hook.Settle, `A's fault: {a.fault and a.fault.error}`)
		spec.eq(w.count("onDestroy:A"), 1, "A tore down once")
		for i = 1, 200 do
			spec.eq(nodes[i].fault, nil, `N{i} fault`)
		end
	end,

	["C5: 60 cascading sources that each toggle the same 60 targets never fault them"] = function()
		local w = world.new()
		local sources, targets = {}, {}
		for i = 1, 60 do
			sources[i] = w.make(`S{i}`)
			targets[i] = w.make(`T{i}`)
		end
		w.life.admit(sources)
		w.life.admit(targets)
		for i = 1, 60 do
			w.scripts[`S{i}`].onDisable = function()
				for _, t in targets do
					w.life.request(t, if t.want == Want.Enabled then Want.Disabled else Want.Enabled)
				end
				if i < 60 then
					w.life.request(sources[i + 1], Want.Disabled)
				end
			end
		end
		w.life.request(sources[1], Want.Disabled)
		for _, list in { sources, targets } do
			for _, n in list do
				spec.eq(n.fault, nil, `{n.record.name} fault`)
			end
		end
		spec.eq(sources[60].phase, Phase.Disabled, "the cascade reached its end")
		spec.eq(#w.failures, 0, "no drain failure")
	end,

	["C5 gap: the chained-request cap faults a member of the cycle, never a target that only receives requests"] = function()
		local w = world.new()
		local a, b = w.boot(w.make("A"), w.make("B"))
		local passive = {}
		for i = 1, 10 do
			passive[i] = w.make(`P{i}`)
		end
		w.life.admit(passive)
		local guard = guardedEnables(200000)
		local function toggle(s)
			w.life.request(s, if s.want == Want.Enabled then Want.Disabled else Want.Enabled)
		end
		local function act(other)
			return function()
				guard()
				for _, p in passive do
					toggle(p)
				end
				toggle(other)
			end
		end
		w.scripts.A.onEnable, w.scripts.A.onDisable = act(b), act(b)
		w.scripts.B.onEnable, w.scripts.B.onDisable = act(a), act(a)
		w.life.request(a, Want.Disabled)
		local members = (a.fault and a.fault.hook == Hook.Settle) or (b.fault and b.fault.hook == Hook.Settle)
		spec.eq(members, true, "a cycle member faulted with Settle")
		for _, p in passive do
			spec.eq(p.fault, nil, `{p.record.name} fault`)
		end
	end,

	["C5: a cascade of 40 objects that each disable the next never faults"] = function()
		local w = world.new()
		local nodes = {}
		for i = 1, 40 do
			nodes[i] = w.make(`N{i}`)
		end
		w.life.admit(nodes)
		for i = 1, 39 do
			w.scripts[`N{i}`].onDisable = function()
				w.life.request(nodes[i + 1], Want.Disabled)
			end
		end
		w.life.request(nodes[1], Want.Disabled)
		for i, s in nodes do
			spec.eq(s.fault, nil, `N{i} fault`)
			spec.eq(s.phase, Phase.Disabled, `N{i} phase`)
		end
	end,

	["C5: a start that toggles its own object across 100 yields never faults"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				for _ = 1, 100 do
					coroutine.yield()
					w.life.request(s, if s.want == Want.Enabled then Want.Disabled else Want.Enabled)
				end
			end,
		}))
		for _ = 1, 100 do
			coroutine.resume(w.threads.A)
		end
		spec.eq(a.fault, nil, "fault")
		spec.eq(a.phase, Phase.Enabled, "an even number of toggles ends enabled")
		spec.eq(w.count("onDisable:A"), 50, "disables")
	end,

	["act-on 5: a subscription disconnect that throws still disconnects the rest and ends the generation once"] = function()
		local w = world.new()
		local a = w.boot(w.make("A"))
		local second = false
		a.subscriptions = {
			function()
				error("disconnect threw")
			end,
			function()
				second = true
			end,
		}
		local ok, err = pcall(w.life.request, a, Want.Destroyed)
		spec.eq(ok, true, `no error reached the caller: {err}`)
		spec.eq(second, true, "the second subscription disconnected")
		spec.eq(w.count("destroyed:A"), 1, "ended once")
		spec.eq(a.fault, nil, "the generation is not faulted")
		spec.eq(w.diagnosed[1] and w.diagnosed[1].hook, Hook.Internal, "the disconnect failure is diagnosed")
		spec.has(w.diagnosed[1].error, "disconnect threw", "diagnosed error")
		spec.eq(#w.failures, 0, "no drain failure")
	end,

	["kind: a Subsystem gets construct, awake and start but no onEnable, onDisable or onDestroy"] = function()
		local w = world.new()
		local s = w.boot(w.make("S", {}, Kind.Subsystem))
		w.life.request(s, Want.Disabled)
		w.life.request(s, Want.Enabled)
		w.life.request(s, Want.Destroyed)
		local hooks = {}
		for _, e in w.events do
			local name = string.match(e, "^(%a+):")
			if table.find({ "construct", "awake", "start", "onEnable", "onDisable", "onDestroy" }, name) then
				table.insert(hooks, e)
			end
		end
		spec.list(hooks, { "construct:S", "awake:S", "start:S" }, "hooks")
		spec.eq(s.phase, Phase.Destroyed, "phase")
	end,

	["act-on 4: destroy whose start cancellation throws still tears down once and diagnoses Cancel"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", { start = "suspend" }))
		w.cancelError = "cancel refused"
		w.clear()
		local ok, err = pcall(w.life.request, a, Want.Destroyed)
		spec.eq(ok, true, `no error reached the caller: {err}`)
		spec.list(
			w.events,
			{ "leave:A", "onDisable:A", "cancel:A", "diagnose:A", "onDestroy:A", "destroyed:A" },
			"events"
		)
		spec.eq(a.phase, Phase.Destroyed, "phase")
		spec.eq(a.fault, nil, "the generation is not faulted")
		spec.eq(w.diagnosed[1].hook, Hook.Cancel, "diagnosed hook")
		spec.has(w.diagnosed[1].error, "cancel refused", "diagnosed error")
		spec.eq(#w.failures, 0, "no drain failure")
	end,

	["act-on 4: a faulted generation whose start cancellation throws attaches it to the fault"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", { start = "suspend" }))
		w.cancelError = "cancel refused"
		w.clear()
		local ok, err = pcall(w.life.fault, a, Hook.Frame, "update threw")
		spec.eq(ok, true, `no error reached the caller: {err}`)
		spec.eq(w.count("onDestroy:A"), 1, "onDestroy ran once")
		spec.eq(w.count("faulted:A"), 1, "ended once")
		spec.eq(a.fault.hook, Hook.Frame, "primary fault")
		spec.eq(#a.fault.cleanup, 1, "attached failures")
		spec.eq(a.fault.cleanup[1].hook, Hook.Cancel, "attached hook")
		spec.has(a.fault.cleanup[1].error, "cancel refused", "attached error")
		spec.eq(#w.diagnosed, 0, "nothing diagnosed separately")
	end,

	["act-on 4: a deferred start cancellation that throws is diagnosed after the generation ended"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.life.request(s, Want.Destroyed)
				coroutine.yield()
			end,
		}))
		w.cancelError = "cancel refused"
		coroutine.resume(w.threads.A)
		spec.eq(a.phase, Phase.Destroyed, "ended before the deferred cancel")
		local ok, err = pcall(w.flush)
		spec.eq(ok, true, `the deferred cancel did not throw: {err}`)
		spec.eq(w.count("cancel:A"), 1, "cancel attempted once")
		spec.eq(#w.diagnosed, 1, "diagnosed once")
		spec.eq(w.diagnosed[1].hook, Hook.Cancel, "diagnosed hook")
		spec.eq(w.count("destroyed:A"), 1, "ended once")
	end,

	["act-on 10: a throwing start's fault carries a traceback"] = function()
		local w = world.new()
		local function startThatThrows()
			error("start threw")
		end
		local a = w.boot(w.make("A", { start = startThatThrows }))
		spec.eq(a.fault.hook, Hook.Start, "primary fault")
		spec.has(a.fault.error, "start threw", "the message")
		spec.has(a.fault.error, "function startThatThrows", "the traceback names the throwing function")
	end,

	["C6: destroy during a running start defers its cancellation; code up to the next yield runs once and its result is ignored (assumes the deferred check runs before the start thread's second resumption)"] = function()
		local w = world.new()
		local ranAfterDestroy, resumedAfterYield = 0, false
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.life.request(s, Want.Destroyed)
				ranAfterDestroy += 1
				coroutine.yield()
				resumedAfterYield = true
			end,
		}))
		w.clear()
		coroutine.resume(w.threads.A)
		spec.list(w.events, { "leave:A", "onDisable:A", "onDestroy:A", "destroyed:A" }, "teardown with no cancel yet")
		spec.eq(ranAfterDestroy, 1, "code up to the next yield ran once")
		w.flush()
		spec.eq(w.count("cancel:A"), 1, "cancelled at its next yield")
		spec.eq(#w.diagnosed, 0, "nothing diagnosed")
		spec.eq(coroutine.status(w.threads.A), "dead", "the start thread is dead")
		spec.eq(resumedAfterYield, false, "never resumed past the yield")
		spec.eq(a.phase, Phase.Destroyed, "phase")
	end,

	["C6: a start resumption queued ahead of the deferred check runs one more segment, is ignored, and is cancelled at its following yield (fake-engine order, as S1 measured in Studio 0.745)"] = function()
		local w = world.new()
		local extraSegments, resumedPastExtra = 0, false
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.engine.port.defer(function()
					local ok, err = coroutine.resume(w.threads.A)
					spec.eq(ok, true, `the queued resumption ran: {err}`)
				end)
				w.life.request(s, Want.Destroyed)
				coroutine.yield()
				extraSegments += 1
				coroutine.yield()
				resumedPastExtra = true
			end,
		}))
		w.clear()
		coroutine.resume(w.threads.A)
		spec.list(w.events, { "leave:A", "onDisable:A", "onDestroy:A", "destroyed:A" }, "teardown with no cancel yet")
		w.flush()
		spec.eq(extraSegments, 1, "the queued resumption ran one more segment")
		spec.eq(w.count("cancel:A"), 1, "cancelled at its following yield")
		spec.eq(coroutine.status(w.threads.A), "dead", "the start thread is dead")
		spec.eq(resumedPastExtra, false, "never resumed past the following yield")
		spec.eq(a.fault, nil, "the ignored segment left no fault")
		spec.eq(a.phase, Phase.Destroyed, "phase")
		spec.eq(w.count("destroyed:A"), 1, "ended once")
		spec.eq(w.count("tick:A"), 0, "nothing ticked")
		spec.eq(a.ticking, false, "not ticking")
	end,

	["C6: a start that destroys its own object and then throws leaves no fault"] = function()
		local w = world.new()
		local a = w.boot(w.make("A", {
			start = function(s)
				w.threads.A = coroutine.running()
				coroutine.yield()
				w.life.request(s, Want.Destroyed)
				error("threw after its destroy")
			end,
		}))
		w.clear()
		local ok, err = coroutine.resume(w.threads.A)
		spec.eq(ok, true, `the start thread returned: {err}`)
		spec.eq(a.fault, nil, "no fault")
		spec.eq(a.phase, Phase.Destroyed, "phase")
		spec.list(w.events, { "leave:A", "onDisable:A", "onDestroy:A", "destroyed:A" }, "events")
	end,
}
