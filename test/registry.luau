local spec = require("./lib/load")
local world = require("./lib/registry-world")
local meter = require("./lib/meter")
local tests = {}

tests["scheduler admission and server seal keep the registered hook mask after class mutation"] = function()
	local w = world()
	local class, record = w.define("RegisteredWithoutHooks", "behaviour")
	local seen = {}
	class.update = function()
		table.insert(seen, "update")
	end
	class.preRender = function()
		table.insert(seen, "preRender")
	end
	local scheduler = w.graph
		.module("internal/scheduler").Scheduler
		.new(w.engine.port, { renders = true, fixedRate = 60 }, w.hooks.hooks().frame)
	local state = w.track(record)
	state.object = setmetatable({}, class)
	scheduler:add(state)
	w.engine.cycle(1 / 60, true)
	spec.list(seen, {}, "late overrides do not enter scheduler lanes")
	spec.eq(#w.engine.renderSteps, 0, "late preRender does not bind a render step")
	local ok, err = pcall(w.seal)
	spec.eq(ok, true, `server seal accepts the registered hook mask: {tostring(err)}`)
end

tests["server seal rejects renderPriority without a preRender hook and names the class"] = function()
	local w = world()
	local _, record = w.define("PriorityOnly", "subsystem", { renderPriority = 100 })
	local result = w.registry.plan({ record }, { renders = false })
	spec.eq(result.ok, false, "priority alone is a server render problem")
	spec.eq(#result.problems, 1, "one server render problem")
	spec.eq(result.problems[1].kind, "render-on-server", "problem kind")
	spec.eq(result.problems[1].id, "PriorityOnly", "problem names its class")
	local ok, err = pcall(w.seal)
	spec.eq(ok, false, "server seal rejects a priority-only class")
	spec.has(err, "PriorityOnly renders but is loaded on the server", "seal names the render class")
end

tests["defineClass records override masks once and scheduler lanes follow registered overrides"] = function()
	local Engine = require("./lib/engine")
	for mask = 0, 15 do
		local w = world()
		local seen = {}
		local methods = { renderPriority = 100 }
		local names = { "preRender", "fixedUpdate", "update", "lateUpdate" }
		for index, name in names do
			if bit32.band(mask, 2 ^ (index - 1)) ~= 0 then
				methods[name] = function(_, dt)
					spec.eq(dt, 1 / 60, name .. " dt")
					table.insert(seen, name)
				end
			end
		end
		local class, record = w.define("Registered", "behaviour", methods)
		spec.eq(record.hooks, mask, "registration mask")
		spec.eq(record.renderPriority, 100, "registered priority")
		local scheduler = w.graph
			.module("internal/scheduler").Scheduler
			.new(w.engine.port, { renders = true, fixedRate = 60 }, w.hooks.hooks().frame)
		local state = w.track(record)
		state.object = setmetatable({}, class)
		scheduler:add(state)
		w.engine.cycle(1 / 60, true)
		local expected = {}
		for index, name in names do
			if bit32.band(mask, 2 ^ (index - 1)) ~= 0 then
				table.insert(expected, name)
			end
		end
		spec.list(seen, expected, "only overridden lanes run")
		spec.eq(#w.engine.renderSteps, if bit32.band(mask, 1) ~= 0 then 1 else 0, "priority lane")
		scheduler:remove(state)
		table.clear(seen)
		w.engine.cycle(1 / 60, true)
		spec.list(seen, {}, "registered object removed from every lane")
		class.update = function() end
		spec.eq(record.hooks, mask, "mask stays fixed after registration")
		spec.eq(#w.engine.frames[Engine.FrameEvent.PostSimulation].list, 0, "simulation disconnected")
	end
end

tests["registration preserves the wire contract, resolves dependencies and seals once"] = function()
	local w = world()
	local _, a = w.define("A", "subsystem", nil, { deps = { "B", { all = "I" } }, provides = { "IA" } })
	local _, b = w.define("B", "subsystem")
	local check = w.emit.t.number
	local class, c = w.define(
		"C",
		"behaviour",
		nil,
		{ tag = "Doors", attributes = { speed = { name = "Speed", check = check, required = true } } }
	)
	spec.eq(c.kind, 1, "behaviour kind")
	spec.eq(c.tag, "Doors", "wire tag")
	spec.eq(setmetatable({}, class).tag, "Doors", "instance lookup")
	spec.list(c.instanceClasses, { "Model" }, "instance wire")
	spec.eq(c.attributes.speed.name, "Speed", "attribute name")
	spec.eq(c.attributes.speed.check, check, "attribute check")
	spec.eq(c.attributes.speed.required, true, "attribute requirement")
	spec.list(a.provides, { "A", "IA" }, "provides")
	local order = w.seal()
	spec.eq(order[1], b, "dependency first")
	spec.eq(order[2], a, "dependent second")
	spec.eq(a.resolved[1].record, b, "resolved provider")
	spec.eq(a.resolved[2].set, w.emit.resolveAll("I"), "resolved All identity")
	spec.eq(a.memberOf[2], w.emit.resolveAll("IA"), "membership identity")
	local ok, err = pcall(w.define, "Late", "subsystem")
	spec.eq(ok, false, "late registration rejected")
	spec.has(err, "Late", "late type named")
	spec.eq(pcall(w.seal), false, "double seal rejected")
end

tests["seal aggregates missing, ambiguous, behaviour-only, cycle, duplicate and server-render failures"] = function()
	local w = world()
	w.define("A", "subsystem", nil, { deps = { "Missing", "I", "B", "Cycle" } })
	w.define("P", "subsystem", nil, { provides = { "I" } })
	w.define("Q", "subsystem", nil, { provides = { "I" } })
	w.define("B", "behaviour")
	w.define("Cycle", "subsystem", nil, { deps = { "A" } })
	w.define("P", "subsystem")
	w.define("Render", "subsystem", { preRender = function() end })
	local ok, err = pcall(w.seal)
	spec.eq(ok, false, "invalid plan rejected")
	for _, text in { "Missing", "P, Q", "only behaviours", "dependency cycle", "registered twice", "renders" } do
		spec.has(err, text, "aggregated problem")
	end
end

tests["inject before boot or awake rejects, then resolves an awake provider"] = function()
	local w = world()
	local _, record = w.define("Provider", "subsystem", nil, { provides = { "RequestedService" } })
	spec.eq(pcall(w.emit.resolve, "RequestedService"), false, "before seal")
	w.seal()
	spec.eq(pcall(w.emit.resolve, "RequestedService"), false, "before construction")
	local state = w.track(record)
	state.object = {}
	state.phase = 1
	local ok, err = pcall(w.emit.resolve, "RequestedService")

	spec.eq(ok, false, "constructed")
	spec.has(err, "RequestedService", "requested type named")
	spec.has(err, "not awake yet", "reason named")
	state.phase = 2
	spec.eq(w.emit.resolve("RequestedService"), state.object, "awake")
	spec.eq(pcall(w.emit.resolve, "Missing"), false, "missing provider")
end

tests["act-on 10: resolve rejects a Faulted provider and names its concrete class"] = function()
	local w = world()
	local _, record = w.define("Broken", "subsystem", {
		awake = function()
			error("broken")
		end,
	}, { provides = { "I" } })
	w.seal()
	local state = w.track(record)
	w.admit({ state })
	spec.eq(state.phase, 5, "faulted by lifecycle")
	local ok, err = pcall(w.emit.resolve, "I")
	spec.eq(ok, false, "faulted provider rejected")
	spec.has(err, "Broken", "provider named")
	spec.has(err, "Faulted", "phase named")
end

tests["rule 6: All joins on enable, leaves on disable and fault, and re-enables at the end"] = function()
	local w = world()
	local _, record = w.define("Item", "behaviour", nil, { provides = { "I" } })
	w.seal()
	local a, b = w.track(record), w.track(record)
	local set = w.emit.resolveAll("I")
	spec.eq(set:size(), 0, "not enabled")
	w.admit({ a, b })
	spec.eq(set:size(), 2, "enabled")
	w.life.request(a, 1)
	spec.eq(set:has(a.object), false, "disabled")
	spec.eq(set:first(), b.object, "next enabled")
	w.life.request(a, 0)
	local seen = {}
	set:forEach(function(item)
		table.insert(seen, item == a.object and "A" or "B")
	end)
	spec.list(seen, { "B", "A" }, "enable order")
	w.life.fault(b, 6, "frame failure")
	spec.eq(set:has(b.object), false, "faulted leaves")
end

for _, mode in { "forEach", "observe" } do
	for _, failure in { "throw", "yield" } do
		tests[`C2: {mode} callbacks that {failure} log and continue without blame`] = function()
			local w = world()
			local _, record = w.define("Member", "behaviour", nil, { provides = { "I" } })
			local _, subscriber = w.define("Subscriber", "subsystem")
			w.seal()
			local states = { w.track(record), w.track(record), w.track(record) }
			local owner = w.track(subscriber)
			w.admit({ owner, table.unpack(states) })
			local set = w.emit.resolveAll("I")
			local visits, resumed = {}, false
			local function callback(item)
				table.insert(visits, item)
				if item == states[2].object then
					if failure == "throw" then
						error("callback failed")
					else
						coroutine.yield()
						resumed = true
					end
				end
			end
			w.life.within(owner, nil, 8, function()
				if mode == "forEach" then
					set:forEach(callback)
				else
					set:observe(callback, callback)
				end
			end, nil, nil)
			spec.eq(#visits, 3, "continued")
			spec.eq(set:size(), 3, "all retained")
			spec.eq(owner.fault, nil, "subscriber not blamed")
			for _, state in states do
				spec.eq(state.fault, nil, "member not blamed")
			end
			spec.eq(resumed, false, "yield cancelled")
			spec.eq(#w.engine.logs, 1, "one log")
			spec.has(w.engine.logs[1].message, "Subscriber", "subscriber class")
			spec.has(w.engine.logs[1].message, "Member", "member class")
			spec.eq(w.engine.logs[1].level, 1, "Error level")
			if mode == "observe" then
				w.life.request(states[2], 1)
				spec.eq(#w.engine.logs, 2, "removed callback failure logged")
				w.life.request(states[2], 0)
				spec.eq(#w.engine.logs, 3, "future added failure logged")
			end
		end
	end
end

tests["act-on 5: a failing walk restores its state and the next walk visits every member in order"] = function()
	local w = world()
	local set = w.emit.resolveAll("I")
	local a, b, c = {}, {}, {}
	set:join(a)
	set:join(b)
	set:join(c)
	local original = set.deliver
	set.deliver = function()
		error("delivery failed")
	end
	local ok = pcall(set.forEach, set, function() end)
	set.deliver = original
	spec.eq(ok, false, "walk error escaped")
	local seen = {}
	set:forEach(function(item)
		table.insert(seen, item == a and "A" or item == b and "B" or "C")
	end)
	spec.list(seen, { "A", "B", "C" }, "restored walk")
	spec.eq(set.members.depth, 0, "walk depth restored")
end

tests["nested All walks and mutations preserve the outer visitor and snapshot"] = function()
	local w = world()
	local set = w.emit.resolveAll("I")
	local a, b, c, d = {}, {}, {}, {}
	set:join(a)
	set:join(b)
	set:join(c)
	local outer, inner = {}, {}
	set:forEach(function(item)
		table.insert(outer, item)
		if item == a then
			set:forEach(function(member)
				table.insert(inner, member)
			end)
			set:leave(c)
			set:join(d)
		end
	end)
	spec.eq(#inner, 3, "inner complete")
	spec.eq(#outer, 2, "outer snapshot skips removed and appended")
	spec.eq(outer[2], b, "outer callback restored")
end

tests["observe replays, disconnects idempotently, and ends with its owner"] = function()
	local w = world()
	local _, record = w.define("Owner", "behaviour")
	w.seal()
	local owner = w.track(record)
	w.admit({ owner })
	local set = w.emit.resolveAll("I")
	local a, b = {}, {}
	set:join(a)
	local added, removed, disconnect = 0, 0, nil
	w.life.within(owner, nil, 8, function()
		disconnect = set:observe(function()
			added += 1
		end, function()
			removed += 1
		end)
	end, nil, nil)
	spec.eq(added, 1, "replay")
	set:join(a)
	set:join(b)
	set:leave(a)
	set:leave(a)
	spec.eq(added, 2, "idempotent join")
	spec.eq(removed, 1, "idempotent leave")
	w.life.request(owner, 2)
	set:leave(b)
	set:join(a)
	spec.eq(added, 2, "owner end disconnected")
	spec.eq(removed, 1, "owner end disconnected removed")
	disconnect()
	disconnect()
end

tests["pool rule: All churn of 1000 members for 100 frames stays bounded and in enable order"] = function()
	local w = world()
	local set = w.emit.resolveAll("I")
	local enabled, pool = {}, {}
	for i = 1, 1500 do
		local item = { name = tostring(i) }
		if i <= 1000 then
			set:join(item)
			table.insert(enabled, item)
		else
			table.insert(pool, item)
		end
	end
	local m = meter(set.members)
	local warmTop
	for frame = 1, 100 do
		m.reads = 0
		local visited, added = {}, {}
		set:forEach(function(item)
			table.insert(visited, item.name)
			if #added < 500 then
				set:leave(item)
				table.insert(pool, item)
				local reused = table.remove(pool, 1)
				set:join(reused)
				table.insert(added, reused)
			end
		end)
		local expected = {}
		for _, item in enabled do
			table.insert(expected, item.name)
		end
		spec.list(visited, expected, `enable order frame {frame}`)
		spec.eq(m.reads <= 6000, true, `bounded slot reads frame {frame}`)
		if frame == 5 then
			warmTop = m.top
		end
		if frame > 5 then
			spec.eq(m.top, warmTop, "slots stop growing")
		end
		enabled = table.move(added, 1, #added, 501, { table.unpack(enabled, 501) })
	end
	spec.eq(set:size(), 1000, "member count")
end

tests["emit attribute uses the matching construction claim and keeps false and zero"] = function()
	local w = world()
	local class, record = w.define("Fields", "behaviour", nil, {
		init = function(self)
			self.speed = w.emit.attribute(self, "speed", 30)
			self.open = w.emit.attribute(self, "open", true)
			self.label = w.emit.attribute(self, "label", "fallback")
			self.absent = w.emit.attribute(self, "absent")
		end,
	})
	w.seal()
	local state = w.track(record, { speed = 0, open = false })
	w.admit({ state })
	spec.eq(state.object.speed, 0, "zero admitted")
	spec.eq(state.object.open, false, "false admitted")
	spec.eq(state.object.label, "fallback", "absent fallback")
	spec.eq(state.object.absent, nil, "absent optional")
	spec.eq(w.claim.readClaim(), nil, "clear")
	spec.eq(pcall(class.new), false, "unclaimed construction fails")
	w.claim.setClaim(state)
	spec.eq(pcall(w.emit.attribute, {}, "speed", 3), false, "other object rejected")
	w.claim.clearClaim()
end

tests["emitted t calls use dot properties for literal optional and mixed union"] = function()
	local t = world().emit.t
	spec.eq(t.literal("door")("door"), true, "literal")
	spec.eq(t.literal(3)(4), false, "literal rejects")
	spec.eq(t.optional(t.number)(nil), true, "optional absent")
	spec.eq(t.optional(t.number)("bad"), false, "optional rejects")
	spec.eq(t.union(t.string, t.literal(3))(3), true, "K3 mixed union")
	spec.eq(t.union(t.string, t.literal(3))(false), false, "union rejects")
	spec.eq(t.boolean(false), true, "boolean")
end

tests["start yields then observes with its owner while task.spawn remains ownerless"] = function()
	local task = require("@lune/task")
	local w = world()
	local set = w.emit.resolveAll("I")
	local thread, owned, spawned = nil, 0, 0
	local _, record = w.define("Watcher", "behaviour", {
		awake = function()
			task.spawn(function()
				set:observe(function()
					spawned += 1
				end)
			end)
		end,
		start = function()
			thread = coroutine.running()
			coroutine.yield()
			set:observe(function()
				owned += 1
			end)
		end,
	})
	w.seal()
	local owner = w.track(record)
	set:join({})
	w.admit({ owner })
	spec.eq(owned, 0, "start suspended")
	spec.eq(coroutine.resume(thread), true, "start resumed")
	spec.eq(owned, 1, "replayed after yield")
	set:join({})
	spec.eq(owned, 2, "owned observer live")
	w.life.request(owner, 2)
	set:join({})
	spec.eq(owned, 2, "owned observer disconnected")
	spec.eq(spawned, 3, "spawned observer retained")
end

tests["observe from a start that already destroyed its own object leaves no live subscription"] = function()
	local w = world()
	local set = w.emit.resolveAll("I")
	local calls = 0
	local state, thread, ended
	local _, record = w.define("Watcher", "behaviour", {
		start = function()
			thread = coroutine.running()
			coroutine.yield()
			w.life.request(state, 2)
			ended = state.phase
			set:observe(function()
				calls += 1
			end)
		end,
	})
	w.seal()
	state = w.track(record)
	w.admit({ state })
	spec.eq(coroutine.resume(thread), true, "start resumed outside a drain")
	spec.eq(ended, 6, "destroyed while its start thread runs (C6)")
	set:join({})
	spec.eq(calls, 0, "an ended owner gets no callbacks")
	spec.eq(#set.observers.items, 0, "no observer retained")
end

return tests
