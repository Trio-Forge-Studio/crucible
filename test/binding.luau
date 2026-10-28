local spec = require("./lib/load")
local World = require("./lib/binding-world")
local task = require("@lune/task")
local tests = {}
local function test(name, fn)
	tests[name] = fn
end
local function numeric(required)
	return {
		name = "Speed",
		required = required,
		check = function(v)
			return type(v) == "number"
		end,
	}
end

test("rule 3 and 4: tagged accepted instances bind once per class with the emitted tag and typed instance", function()
	local w = World()
	spec.eq(w.graph.module("").CrucibleBehaviour, w.behaviour, "root export")
	local A = w.define("A", {}, { tag = "Shared" })
	local B = w.define("B", {}, { tag = "Shared" })
	local i = w.tagged("I", "Shared")
	w.boot()
	local a, b = A:get(i), B:get(i)
	spec.eq(a.instance, i, "instance")
	spec.eq(a.tag, "Shared", "tag")
	spec.eq(a ~= b, true, "one per class")
	w.engine.flush()
	spec.eq(A:get(i), a, "no duplicate")
	a:setEnabled(false)
	spec.eq(a:isEnabled(), false, "disabled")
	spec.eq(A:get(i), a, "disabled still visible")
	a:setEnabled(true)
	spec.eq(a:isEnabled(), true, "enabled")
end)

test("C1: AddTag then get outside a drain returns awake; RemoveTag then get returns undefined", function()
	local w = World()
	local awake = 0
	local A = w.define("A", {
		awake = function()
			awake += 1
		end,
	})
	w.boot()
	local i = w.tagged("I", "A")
	local a = A:get(i)
	spec.eq(a ~= nil, true, "immediate admission")
	spec.eq(awake, 1, "awake at call site")
	w.engine.flush()
	spec.eq(A:get(i), a, "late signal")
	w.engine.removeTag(i, "A")
	spec.eq(A:get(i), nil, "immediate removal")
	w.engine.flush()
	w.engine.addTag(i, "A")
	spec.eq(A:get(i) ~= nil and A:get(i) ~= a, true, "new generation")
end)

test("C1: outside get reconciles only its pair while unrelated signals remain pending", function()
	local w = World()
	local calls = {}
	local _A, ar = w.define("A", {
		awake = function(self)
			table.insert(calls, self.instance.Name)
		end,
	})
	local B, br = w.define("B")
	w.boot()
	local a = w.tagged("A1", "A")
	local b = w.tagged("B1", "B")
	table.remove(w.engine.deferred, 1)()
	table.remove(w.engine.deferred, 1)()
	spec.eq(B:get(b) ~= nil, true, "selected B")
	spec.eq(ar.bound[a], nil, "unrelated A not reconciled")
	spec.eq(#calls, 0, "queued A did not wake")
	spec.eq(br.bound[b] ~= nil, true, "B admitted")
	w.engine.flush()
	spec.eq(ar.bound[a] ~= nil, true, "parked pair restored")
end)

test("C1: get inside a hook on an untagged instance does not warn pending", function()
	local w = World()
	local B = w.define("B")
	local plain = w.engine.instance("Plain")
	w.define("A", {
		awake = function()
			spec.eq(B:get(plain), nil, "untagged has no binding")
		end,
	})
	w.tagged("I", "A")
	w.boot()
	spec.eq(w.engine.logged("get pending"), 0, "untagged is not pending")
end)

test("C1: outside get runs a parked pair refreshed by its hook in the same drain", function()
	local w = World()
	local _, br = w.define("B")
	local target = w.engine.instance("T")
	local A = w.define("A", {
		awake = function()
			w.behaviour:refresh(target)
		end,
	})
	w.boot()
	w.engine.addTag(target, "B")
	table.remove(w.engine.deferred, 1)()
	local i = w.tagged("I", "A")
	spec.eq(A:get(i) ~= nil, true, "selected binding awake")
	spec.eq(br.bound[target] ~= nil, true, "refreshed parked pair admitted")
	spec.eq(br.bound[target].phase, 3, "refreshed binding enabled before get returns")
end)

test("rule 5: invalid valid same invalid resets the warning condition", function()
	local w = World()
	local A = w.define("A", {}, {
		attributes = { speed = numeric(true) },
		init = function(self)
			self.speed = w.emit.attribute(self, "speed")
		end,
	})
	local i = w.tagged("I", "A", { Speed = 7 })
	w.boot()
	for _, value in { "bad", 5, "bad" } do
		w.engine.setAttribute(i, "Speed", value)
		w.engine.flush()
	end
	spec.eq(A:get(i).speed, 5, "last valid")
	spec.eq(w.engine.logged("keeping the last valid value"), 2, "same invalid warns after valid")
end)

test(
	"S1 Deferred: RemoveTag then AddTag and Parent nil then back in one resumption create new bindings without get",
	function()
		local w = World()
		local _, ar = w.define("A")
		local i = w.tagged("I", "A")
		w.boot()
		w.engine.flush()
		local old = ar.bound[i]
		w.engine.removeTag(i, "A")
		w.engine.addTag(i, "A")
		w.engine.flush()
		spec.eq(ar.bound[i] ~= nil and ar.bound[i] ~= old, true, "tag return creates new binding")
		old = ar.bound[i]
		w.engine.move(i, nil)
		w.engine.move(i, "Workspace")
		w.engine.flush()
		spec.eq(ar.bound[i] ~= nil and ar.bound[i] ~= old, true, "world return creates new binding")
	end
)

test("C1: get inside a hook never runs lifecycle and warns once per call site when pending", function()
	local w = World()
	local B, br = w.define("B")
	local target = w.tagged("B1", "B")
	local seen = {}
	local function lookup()
		return B:get(target)
	end
	w.define("A", {
		awake = function()
			for _ = 1, 3 do
				table.insert(seen, if lookup() == nil then "none" else "bound")
			end
			spec.eq(br.bound[target].phase, 1, "B still constructed")
		end,
	})
	w.tagged("A1", "A")
	w.boot()
	spec.list(seen, { "none", "none", "none" }, "no nested lifecycle")
	spec.eq(w.engine.logged("get pending"), 1, "one warning per site")
	spec.eq(B:get(target) ~= nil, true, "awake after drain")
end)

test("act-on 1: accepts refreshing and getting its running pair never requeues it", function()
	local w = World()
	local calls = 0
	local A
	A = w.define("A", {
		accepts = function(_, i)
			calls += 1
			if calls < 100 then
				w.behaviour:refresh(i)
				A:get(i)
			end
			return true
		end,
	})
	local i = w.tagged("I", "A")
	w.boot()
	spec.eq(calls, 1, "one reconcile")
	spec.eq(A:get(i) ~= nil, true, "bound")
	spec.eq(calls, 2, "bound get rechecks")
end)

test("C3: default accepts excludes all six storage containers and ancestry alone unbinds and rebinds", function()
	local w = World()
	local A = w.define("A")
	local i = w.tagged("I", "A")
	w.boot()
	local signals = 0
	w.engine.port.onTagAdded("A", function()
		signals += 1
	end)
	w.engine.port.onTagRemoved("A", function()
		signals += 1
	end)
	for _, container in
		{ "ServerStorage", "ReplicatedStorage", "ServerScriptService", "StarterGui", "StarterPack", "StarterPlayer" }
	do
		local old = A:get(i)
		w.engine.move(i, container)
		w.engine.flush()
		spec.eq(w.registry.recordOf(A).bound[i], nil, "ancestry unbound before get")
		spec.eq(A:get(i), nil, container)
		w.engine.move(i, "Workspace")
		w.engine.flush()
		spec.eq(w.registry.recordOf(A).bound[i] ~= nil, true, "ancestry rebound before get")
		spec.eq(A:get(i) ~= nil and A:get(i) ~= old, true, "new binding")
	end
	spec.eq(signals, 0, "S1 row 1b no tag signal")
end)

test("C3: override replaces accepts and may call the base; bound get rechecks accepts", function()
	local w = World()
	local allow = true
	local A = w.define("A", {
		accepts = function()
			return allow
		end,
	})
	local B = w.define("B", {
		accepts = function(_, i)
			return w.behaviour:accepts(i)
		end,
	})
	local i = w.tagged("I", "A")
	w.engine.addTag(i, "B")
	w.engine.move(i, "ServerStorage")
	w.boot()
	spec.eq(A:get(i) ~= nil, true, "override replaces")
	spec.eq(B:get(i), nil, "super excludes")
	allow = false
	spec.eq(A:get(i), nil, "get rechecks")
end)

test("C4: remove and fault suppression last until tag removal or leaving DataModel", function()
	for _, faulted in { false, true } do
		local w = World()
		local fail = faulted
		local A = w.define("A", {
			awake = function()
				if fail then
					error("broken")
				end
			end,
		})
		local i = w.tagged("I", "A")
		w.boot()
		if not faulted then
			A:get(i):remove()
		end
		fail = false
		w.behaviour:refresh(i)
		spec.eq(A:get(i), nil, "suppressed")
		w.engine.move(i, nil)
		w.engine.flush()
		spec.eq(w.engine.port.hasTag(i, "A"), true, "S1 tag retained")
		w.engine.move(i, "Workspace")
		w.engine.flush()
		spec.eq(A:get(i) ~= nil, true, "leave lifts suppression")
		A:get(i):remove()
		w.engine.removeTag(i, "A")
		w.engine.flush()
		w.engine.addTag(i, "A")
		w.engine.flush()
		spec.eq(A:get(i) ~= nil, true, "tag removal lifts suppression")
	end
end)

test("rule 5: required absent and any present invalid attribute block admission", function()
	local w = World()
	local A = w.define("A", {}, {
		attributes = {
			speed = numeric(true),
			optional = {
				name = "Optional",
				required = false,
				check = function(v)
					return type(v) == "boolean"
				end,
			},
		},
	})
	local i = w.tagged("I", "A", { Optional = false })
	w.boot()
	spec.eq(A:get(i), nil, "required absent")
	w.engine.setAttribute(i, "Optional", "bad")
	w.engine.setAttribute(i, "Speed", 2)
	w.engine.flush()
	spec.eq(A:get(i), nil, "optional present invalid")
	w.engine.setAttribute(i, "Optional", false)
	w.engine.flush()
	spec.eq(A:get(i) ~= nil, true, "all valid")
end)

test(
	"rule 5: valid attribute writes update fields; invalid writes keep the last value and log once per condition",
	function()
		local w = World()
		local A = w.define("A", {}, {
			attributes = { speed = numeric(true) },
			init = function(self)
				self.speed = w.emit.attribute(self, "speed")
			end,
		})
		local i = w.tagged("I", "A", { Speed = 90 })
		w.boot()
		local a = A:get(i)
		spec.eq(a.speed, 90, "initializer claim")
		for _, value in { "bad", "worse", true } do
			w.engine.setAttribute(i, "Speed", value)
			w.engine.flush()
		end
		spec.eq(a.speed, 90, "last valid")
		spec.eq(w.engine.logged("keeping the last valid value"), 2, "changed type warns")
		w.engine.setAttribute(i, "Speed", 45)
		w.engine.flush()
		spec.eq(a.speed, 45, "field updated")
		spec.eq(A:get(i), a, "never unbound")
	end
)

test("act-on 3 A3 B3: admission installs watchers before awake writes ahead of deferred tag delivery", function()
	local w = World()
	local A = w.define("A", {
		awake = function(self)
			w.engine.setAttribute(self.instance, "Speed", 2)
		end,
	}, {
		attributes = { speed = numeric(true) },
		init = function(self)
			self.speed = w.emit.attribute(self, "speed")
		end,
	})
	w.boot()
	local i = w.tagged("I", "A", { Speed = 1 })
	local a = A:get(i)
	spec.eq(#i.attributeChanged.Speed.list, 1, "admission owns watch before tag delivery")
	w.engine.flush()
	spec.eq(a.speed, 2, "awake write delivered")
	spec.eq(A:get(i), a, "same generation")
end)

test("act-on 3 B3: reconnect rereads attributes after disconnected pending callbacks are dropped", function()
	local w = World()
	local A = w.define("A", {}, {
		attributes = { speed = numeric(true) },
		init = function(self)
			self.speed = w.emit.attribute(self, "speed")
		end,
	})
	local i = w.tagged("I", "A", { Speed = 1 })
	w.boot()
	w.engine.flush()
	w.engine.removeTag(i, "A")
	w.engine.addTag(i, "A")
	w.engine.setAttribute(i, "Speed", 2)
	w.engine.flush()
	spec.eq(A:get(i).speed, 2, "reconnected snapshot")
end)

test("act-on 6 B6: yielding constructor clears its claim and other threads cannot steal it", function()
	local w = World()
	local A
	local crossThread
	local first = true
	A = w.define("A", {}, {
		init = function()
			if not first then
				return
			end
			first = false
			task.spawn(function()
				crossThread = pcall(A.new)
			end)
			coroutine.yield()
		end,
	})
	local i = w.tagged("I", "A")
	w.boot()
	spec.eq(A:get(i), nil, "faulted")
	spec.eq(crossThread, false, "thread scoped")
	spec.eq(pcall(A.new), false, "no stale claim")
	spec.eq(w.claim.readClaim(), nil, "caller has no claim")
	spec.eq(w.engine.logged("only start may yield"), 1, "yield diagnosed")
end)

test("rule 4: wrong class warns once per instance and class with full name and requirements", function()
	local w = World()
	local A = w.define("game/vehicles/server/Van:Van", {}, { tag = "Van" })
	w.boot()
	for n = 1, 2 do
		local name = "Workspace.Van" .. n
		local i = w.engine.instance(name, "Part")
		w.engine.addTag(i, "Van")
		for _ = 1, 3 do
			spec.eq(A:get(i), nil, "wrong class")
		end
		spec.eq(w.engine.logged(name .. " is a Part; Van needs a Model"), 1, "exact diagnostic once per instance")
	end
end)

test("S1 Deferred: Parent nil fires removed with HasTag true and reparent fires added with a new binding", function()
	local w = World()
	local A = w.define("A")
	local i = w.tagged("I", "A")
	w.boot()
	w.engine.flush()
	local old = A:get(i)
	local events = {}
	w.engine.port.onTagRemoved("A", function(v)
		table.insert(events, if w.engine.port.hasTag(v, "A") then "removed tagged" else "removed")
	end)
	w.engine.port.onTagAdded("A", function()
		table.insert(events, "added")
	end)
	w.engine.move(i, nil)
	spec.eq(#events, 0, "deferred")
	w.engine.flush()
	spec.eq(A:get(i), nil, "left")
	w.engine.move(i, "Workspace")
	w.engine.flush()
	spec.list(events, { "removed tagged", "added" }, "signal order")
	spec.eq(A:get(i) ~= nil and A:get(i) ~= old, true, "new binding")
end)

test("S1 Deferred: Destroy fires one removed signal and retains HasTag", function()
	local w = World()
	local A = w.define("A")
	local i = w.tagged("I", "A")
	w.boot()
	w.engine.flush()
	local count = 0
	w.engine.port.onTagRemoved("A", function()
		count += 1
	end)
	w.engine.destroy(i)
	spec.eq(w.engine.port.hasTag(i, "A"), true, "tag retained after destroy")
	w.engine.flush()
	spec.eq(count, 1, "one removed")
	spec.eq(A:get(i), nil, "unbound")
end)

test("S1 Deferred: AddTag then RemoveTag in one resumption fires added then removed", function()
	local w = World()
	w.define("A")
	w.boot()
	local i = w.engine.instance("I")
	local events = {}
	w.engine.port.onTagAdded("A", function()
		table.insert(events, "added")
	end)
	w.engine.port.onTagRemoved("A", function()
		table.insert(events, "removed")
	end)
	w.engine.addTag(i, "A")
	w.engine.removeTag(i, "A")
	w.engine.flush()
	spec.list(events, { "added", "removed" }, "signal order")
end)

test("S1 Deferred: tagged instance created or cloned at nil emits added only after parenting", function()
	local w = World()
	local A = w.define("A")
	w.boot()
	local i = w.engine.instance("I")
	w.engine.move(i, nil)
	local count = 0
	w.engine.port.onTagAdded("A", function()
		count += 1
	end)
	w.engine.addTag(i, "A")
	w.engine.flush()
	spec.eq(count, 0, "nil emits nothing")
	spec.eq(A:get(i), nil, "not admitted")
	local clone = w.engine.clone(i)
	w.engine.flush()
	spec.eq(count, 0, "clone at nil emits nothing")
	spec.eq(A:get(clone), nil, "clone not admitted")
	w.engine.move(i, "Workspace")
	spec.eq(count, 0, "parent signal deferred")
	w.engine.flush()
	spec.eq(count, 1, "added after parenting")
	spec.eq(A:get(i) ~= nil, true, "bound")
	w.engine.move(clone, "Workspace")
	w.engine.flush()
	spec.eq(count, 2, "clone added after parenting")
	spec.eq(A:get(clone) ~= nil, true, "clone bound")
end)

test("S1 Deferred: attribute changed then tag added in one resumption delivers attribute first", function()
	local w = World()
	w.define("A")
	w.boot()
	local i = w.engine.instance("I")
	local events = {}
	w.engine.port.onAttributeChanged(i, "Speed", function()
		table.insert(events, "attribute")
	end)
	w.engine.port.onTagAdded("A", function()
		table.insert(events, "tag")
	end)
	w.engine.setAttribute(i, "Speed", 3)
	w.engine.addTag(i, "A")
	w.engine.flush()
	spec.list(events, { "attribute", "tag" }, "signal order")
end)

test("v2 usage: concrete behaviour inherits update from its abstract base with the Update mask", function()
	local w = World()
	local base = setmetatable({ update = function() end }, { __index = w.behaviour })
	local A = w.define("A", {}, { base = base })
	spec.eq(rawget(A, "update"), nil, "no forwarding method")
	spec.eq(w.hooks.hooks().overrides(A), 4, "inherited Update only")
end)

test("v2 hooks: CrucibleBehaviour own hooks match the hook-name list", function()
	local w = World()
	local api = { "accepts", "refresh", "get", "isEnabled", "setEnabled", "remove" }
	local names = {}
	for key, value in w.behaviour do
		if type(value) == "function" and key ~= "constructor" and not table.find(api, key) then
			table.insert(names, key)
		end
	end
	table.sort(names)
	local expected = table.clone(w.hooks.BEHAVIOUR_HOOK_NAMES)
	table.sort(expected)
	spec.list(names, expected, "own hooks")
end)
test("act-on 3: reconnect snapshots fields even when no attribute callback survived", function()
	local w = World()
	local A = w.define("A", {}, {
		attributes = { speed = numeric(true) },
		init = function(self)
			self.speed = w.emit.attribute(self, "speed")
		end,
	})
	local i = w.tagged("I", "A", { Speed = 1 })
	w.boot()
	w.engine.flush()
	local a = A:get(i)
	w.stop()
	w.engine.setAttribute(i, "Speed", 2)
	w.binding.watch(w.registry.records())
	w.life.drain()
	spec.eq(a.speed, 2, "reconnect snapshot")
	spec.eq(A:get(i), a, "same binding")
end)

test("v2 binding: frame hook removal finishes its body before teardown of itself and another member", function()
	local w = World()
	local events = {}
	local A
	local target
	A = w.define("A", {
		update = function(self)
			table.insert(events, "update")
			self:remove()
			A:get(target):remove()
			table.insert(events, "continued")
		end,
		onDisable = function(self)
			table.insert(events, "disable " .. self.instance.Name)
		end,
		onDestroy = function(self)
			table.insert(events, "destroy " .. self.instance.Name)
		end,
	})
	local i = w.tagged("I", "A")
	target = w.tagged("J", "A")
	w.boot()
	local a = A:get(i)
	local state = w.life.stateOf(a)
	w.life.within(state, state, 6, function()
		a:update(0)
	end, nil, nil)
	spec.list(events, { "update", "continued", "disable I", "destroy I", "disable J", "destroy J" }, "scope order")
end)

test("public binding declarations compile; all reserved names reject; compiled usage inherits Update", function()
	local result = require("@lune/process").exec("node", { "test/lib/binding-consumer.mjs" })
	spec.eq(result.code, 0, result.stdout .. result.stderr)
	local w = World()
	local exports = require("@lune/luau").load(require("@lune/fs").readFile("build/binding-usage.luau"), {
		environment = {
			script = {},
			_G = setmetatable({}, {
				__index = function()
					return {
						getModule = function()
							return { out = { emit = "emit" } }
						end,
						import = function(_, entry)
							return if entry == "emit" then w.emit else { CrucibleBehaviour = w.behaviour }
						end,
					}
				end,
			}),
		},
	})()
	spec.eq(rawget(exports.LockerZone, "update"), nil, "no forwarding method")
	spec.eq(w.hooks.hooks().overrides(exports.LockerZone), 4, "compiled inherited Update")
	w.emit.defineClass(exports.LockerZone, {
		kind = "behaviour",
		id = "Locker",
		tag = "Locker",
		deps = {},
		provides = {},
		instance = { "Model" },
		attributes = { speed = numeric(true) },
	})
	local i = w.tagged("I", "Locker", { Speed = 12 })
	w.boot()
	local object = exports.LockerZone:get(i)
	spec.eq(object.speed, 12, "compiled field initializer")
	spec.eq(object.seen, 12, "later field initializer sees admitted value")
end)
test("act-on 6: caller clears the constructing worker after success throw and yield", function()
	for _, mode in { "success", "throw", "yield" } do
		local cleared = {}
		local w = World(function(graph)
			local claim = graph.module("internal/construction-claim")
			local clear = claim.clearClaim
			claim.clearClaim = function(worker)
				table.insert(cleared, worker)
				clear(worker)
			end
		end)
		local worker
		w.define("A", {}, {
			init = function()
				worker = coroutine.running()
				if mode == "throw" then
					error("bad")
				end
				if mode == "yield" then
					coroutine.yield()
				end
			end,
		})
		w.tagged("I", "A")
		w.boot()
		spec.eq(#cleared, 1, mode .. " cleared exactly once")
		spec.eq(cleared[1], worker, "correct worker")
	end
end)
test("C1: outside get runs the lifecycle and new pairs its selected binding starts", function()
	local w = World()
	local B = w.define("B")
	local target = w.engine.instance("B1")
	local A = w.define("A", {
		awake = function()
			w.engine.addTag(target, "B")
			w.behaviour:refresh(target)
		end,
	})
	w.boot()
	local i = w.tagged("I", "A")
	spec.eq(A:get(i) ~= nil, true, "selected binding awake")
	spec.eq(w.registry.recordOf(B).bound[target].phase, 3, "caused binding already enabled")
end)

test("v2 binding: subsystems and initially tagged behaviours share one initial batch", function()
	local w = World()
	local events = {}
	local base = w.graph.module("subsystem").Subsystem
	local store = setmetatable({}, { __index = base })
	store.__index = store
	function store.new()
		local self = setmetatable({}, store)
		base.constructor(self)
		return self
	end
	function store:awake()
		table.insert(events, "store awake")
	end
	function store:start()
		table.insert(events, "store start " .. w.emit.resolveAll("A"):size())
	end
	w.emit.defineClass(store, { kind = "subsystem", id = "Store", deps = {}, provides = {} })
	w.define("A", {
		awake = function(self)
			table.insert(events, self.instance.Name .. " awake")
		end,
		onEnable = function(self)
			table.insert(events, self.instance.Name .. " enable")
		end,
		start = function(self)
			table.insert(events, self.instance.Name .. " start")
		end,
	})
	w.tagged("I", "A")
	w.tagged("J", "A")
	w.boot()
	spec.list(
		events,
		{ "store awake", "I awake", "J awake", "I enable", "J enable", "store start 2", "I start", "J start" },
		"one batch"
	)
end)

test("Roblox binding port maps CollectionService ancestry attributes scope and disconnects", function()
	local graph = spec.runtime()
	local services = {}
	for _, name in
		{ "ServerStorage", "ReplicatedStorage", "ServerScriptService", "StarterGui", "StarterPack", "StarterPlayer" }
	do
		services[name] = {}
	end
	local stops = 0
	local callbacks = {}
	local function signal(key)
		return {
			Connect = function(_, fn)
				callbacks[key] = fn
				return {
					Disconnect = function()
						stops += 1
					end,
				}
			end,
		}
	end
	local instance = {
		IsDescendantOf = function(_, ancestor)
			return ancestor == services.ReplicatedStorage or ancestor == services.game
		end,
		IsA = function(_, name)
			return name == "Model"
		end,
		AncestryChanged = signal("ancestry"),
		GetAttribute = function(_, name)
			return if name == "Speed" then 12 else nil
		end,
		GetAttributeChangedSignal = function(_, name)
			return signal(name)
		end,
		GetFullName = function()
			return "Workspace.I"
		end,
	}
	services.CollectionService = {
		HasTag = function(_, item, tag)
			return item == instance and tag == "A"
		end,
		GetTagged = function(_, tag)
			return if tag == "A" then { instance } else {}
		end,
		GetInstanceAddedSignal = function(_, tag)
			return signal("added " .. tag)
		end,
		GetInstanceRemovedSignal = function(_, tag)
			return signal("removed " .. tag)
		end,
	}
	local game = {
		GetService = function(_, name)
			return services[name]
		end,
	}
	services.game = game
	local p = graph.module("internal/roblox-port", { game = game }).robloxBindingPort()
	spec.eq(p.hasTag(instance, "A"), true, "CollectionService HasTag")
	spec.eq(p.tagged("A")[1], instance, "GetTagged")
	spec.eq(p.inWorld(instance), true, "IsDescendantOf game")
	spec.eq(p.isA(instance, "Model"), true, "IsA")
	spec.eq(p.bindingScope(instance), false, "scope")
	for _, name in
		{ "ServerStorage", "ReplicatedStorage", "ServerScriptService", "StarterGui", "StarterPack", "StarterPlayer" }
	do
		local descendant = {
			IsDescendantOf = function(_, ancestor)
				return ancestor == services[name]
			end,
		}
		spec.eq(p.bindingScope(descendant), false, name .. " descendant excluded by Roblox port")
	end
	spec.eq(p.getAttribute(instance, "Speed"), 12, "GetAttribute")
	spec.eq(p.describe(instance), "Workspace.I", "GetFullName")
	local heard = 0
	local function listener()
		heard += 1
	end
	local disconnects = {
		p.onTagAdded("A", listener),
		p.onTagRemoved("A", listener),
		p.onAncestryChanged(instance, listener),
		p.onAttributeChanged(instance, "Speed", listener),
	}
	for _, fn in callbacks do
		fn()
	end
	spec.eq(heard, 4, "all signals mapped")
	for _, stop in disconnects do
		stop()
	end
	spec.eq(stops, 4, "connections disconnected")
end)
test("binding rereads tag changes made by accepts before retaining or creating a generation", function()
	for _, bound in { false, true } do
		local w = World()
		local remove = false
		local A = w.define("A", {
			accepts = function(_, instance)
				if remove then
					w.engine.removeTag(instance, "A")
				end
				return true
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		if bound then
			spec.eq(A:get(i) ~= nil, true, "initial binding")
		end
		remove = true
		spec.eq(A:get(i), nil, "accepts removed the tag")
	end
end)
return tests
