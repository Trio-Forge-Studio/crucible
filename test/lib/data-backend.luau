-- One shared DataStore backend of committed values, UserIds and metadata, served to two or more independently loaded
-- server realms (data rule 108). Time is one scripted timeline: each realm's monotonic clock, which is also its
-- BootPort clock, is the timeline plus a fixed offset, and its wall clock carries an offset the spec can step alone.
-- Calls run as events on the timeline, so gates park them at four points and plans inject failures.
local spec = require("./load")
local Engine = require("./engine")

local FRAME = 1 / 60

local Backend = {}
Backend.__index = Backend

local function copy(value)
	if type(value) ~= "table" then
		return value
	end
	local result = {}
	for key, entry in value do
		result[copy(key)] = copy(entry)
	end
	return result
end

function Backend.new()
	return setmetatable({
		now = 0,
		sequence = 0,
		events = {},
		stores = {},
		gates = {},
		plans = {},
		calls = {},
		realms = {},
		errors = {},
	}, Backend)
end

function Backend:schedule(time: number, realm, fn)
	self.sequence += 1
	local event = { time = time, sequence = self.sequence, realm = realm, fn = fn }
	local index = #self.events + 1
	for i, other in self.events do
		if other.time > time then
			index = i
			break
		end
	end
	table.insert(self.events, index, event)
	return event
end

function Backend:resume(thread, ...)
	if coroutine.status(thread) ~= "suspended" then
		return
	end
	local ok, err = coroutine.resume(thread, ...)
	if not ok then
		table.insert(self.errors, debug.traceback(thread, tostring(err)))
	end
end

-- Runs every event due within `seconds`, in time order. A suspended realm's events wait; a crashed realm's are dropped.
function Backend:advance(seconds: number)
	local target = self.now + seconds
	while true do
		local found
		for index, event in self.events do
			if event.time > target then
				break
			end
			if event.cancelled or (event.realm and event.realm.dead) then
				table.remove(self.events, index)
				found = false
				break
			end
			if not (event.realm and event.realm.suspended) then
				table.remove(self.events, index)
				found = event
				break
			end
		end
		if found == nil then
			break
		end
		if found then
			self.now = math.max(self.now, found.time)
			found.fn()
		end
	end
	self.now = target
	if #self.errors > 0 then
		error(table.concat(self.errors, "\n"), 2)
	end
end

function Backend:entry(store: string, key: string)
	return self.stores[store] and self.stores[store][key]
end

-- The committed envelope at a key, as a copy.
function Backend:envelope(store: string, key: string)
	local entry = self:entry(store, key)
	return entry and copy(entry.value)
end

function Backend:stored(store: string, key: string)
	return copy(self:entry(store, key))
end

-- Seeds a key with a stored value, as a game's fixture spec does with a record an older build saved.
function Backend:seed(store: string, key: string, value, userIds, metadata)
	self.stores[store] = self.stores[store] or {}
	self.stores[store][key] = {
		value = copy(value),
		userIds = copy(userIds or {}),
		metadata = copy(metadata or {}),
		version = 1,
	}
end

local function matches(filter, call): boolean
	for field, expected in filter do
		if field ~= "count" and call[field] ~= expected then
			return false
		end
	end
	return true
end

-- Parks calls made after it that reach `point` ("transform", "commit", "committed" or "response") while closed.
function Backend:gate(point: string, filter)
	local gate = { point = point, filter = filter or {}, open = false, parked = {}, after = #self.calls }
	function gate.release()
		gate.open = true
		local parked = gate.parked
		gate.parked = {}
		for _, continue in parked do
			continue()
		end
	end
	table.insert(self.gates, gate)
	return gate
end

function Backend:pass(call, point: string, continue)
	for _, gate in self.gates do
		if not gate.open and gate.point == point and call.index > gate.after and matches(gate.filter, call) then
			table.insert(gate.parked, continue)
			call.parked = point
			return
		end
	end
	call.parked = nil
	continue()
end

-- Injects one failure into each of the next `times` matching calls. Kinds: "throttle" (301, nothing sent),
-- "transient" (503 before the commit), "committed" (commits, then a 502), "conflict" (a rerun of the transform),
-- "late" (the response arrives `seconds` later), "stall" (the call holds its slot for `seconds` before running) and
-- "reject" (a non-retryable `code`). `options.after` skips the first calls: the plan only hits calls numbered above it.
function Backend:fail(kind: string, filter, times: number?, options)
	table.insert(self.plans, {
		kind = kind,
		filter = filter or {},
		remaining = times or 1,
		seconds = options and options.seconds or 0,
		code = options and options.code or 400,
		after = options and options.after or 0,
	})
end

function Backend:plan(call)
	for _, plan in self.plans do
		if plan.remaining > 0 and call.index > plan.after and matches(plan.filter, call) then
			plan.remaining -= 1
			return plan
		end
	end
	return nil
end

local function failure(code: number, retryable: boolean, ambiguous: boolean)
	return { kind = "error", code = code, retryable = retryable, ambiguous = ambiguous }
end

function Backend:call(realm, kind: string, store: string, key: string, transform)
	local thread = coroutine.running()
	local call = {
		realm = realm,
		server = realm.id,
		kind = kind,
		store = store,
		key = key,
		index = #self.calls + 1,
		at = self.now,
	}
	table.insert(self.calls, call)
	local plan = self:plan(call)
	call.plan = plan and plan.kind
	local half = realm.latency / 2
	local function respond(outcome)
		self:pass(call, "response", function()
			local extra = if plan and plan.kind == "late" then plan.seconds else 0
			self:schedule(self.now + half + extra, realm, function()
				call.outcome = outcome
				call.answered = self.now
				self:resume(thread, outcome)
			end)
		end)
	end
	local function run()
		self:pass(call, "transform", function()
			if plan and plan.kind == "throttle" then
				return respond(failure(301, true, false))
			elseif plan and plan.kind == "transient" then
				return respond(failure(503, true, true))
			elseif plan and plan.kind == "reject" then
				return respond(failure(plan.code, false, false))
			end
			local entry = self:entry(store, key)
			if kind == "read" then
				return respond({ kind = "ok", value = copy(entry) })
			end
			local function attempt()
				local current = self:entry(store, key)
				local version = current and current.version or 0
				if plan and plan.kind == "conflict" and not call.rerun then
					call.rerun = true
					transform(copy(current))
				end
				return transform(copy(current)), version
			end
			local written, version = attempt()
			self:pass(call, "commit", function()
				local latest = self:entry(store, key)
				while (latest and latest.version or 0) ~= version do
					call.rerun = true
					written, version = attempt()
					latest = self:entry(store, key)
				end
				if written ~= nil then
					self.stores[store] = self.stores[store] or {}
					self.stores[store][key] = {
						value = copy(written.value),
						userIds = copy(written.userIds),
						metadata = copy(written.metadata),
						version = version + 1,
					}
					call.committed = true
				end
				self:pass(call, "committed", function()
					if plan and plan.kind == "committed" then
						return respond(failure(502, true, true))
					end
					respond({ kind = "ok", value = copy(written or latest) })
				end)
			end)
		end)
	end
	self:schedule(self.now + half + (if plan and plan.kind == "stall" then plan.seconds else 0), nil, run)
	return coroutine.yield()
end

function Backend:callsTo(filter)
	local found = {}
	for _, call in self.calls do
		if matches(filter, call) then
			table.insert(found, call)
		end
	end
	return found
end

-- An independently loaded server realm: its own module graph, BootPort, data port, players, clocks and seed.
function Backend:realm(options)
	options = options or {}
	local backend = self
	local id = options.id or `server{#self.realms + 1}`
	local engine = Engine.new()
	local realm = {
		backend = self,
		id = id,
		engine = engine,
		mono = options.mono or (#self.realms * 1000 + 100),
		wallOffset = options.wall or 1700000000,
		latency = options.latency or 0.1,
		budget = { update = 1000, read = 1000 },
		present = {},
		population = 0,
		kicks = {},
		closers = {},
		timers = {},
		guids = 0,
		seed = options.seed or (#self.realms + 1) * 7919,
		dead = false,
		suspended = false,
	}
	table.insert(self.realms, realm)

	function realm.clock()
		return backend.now + realm.mono
	end
	function realm.wall()
		return backend.now + realm.wallOffset
	end
	function realm.spawn(fn, ...)
		local thread = coroutine.create(fn)
		backend:resume(thread, ...)
		return thread
	end
	function realm.wait(seconds: number)
		local thread = coroutine.running()
		backend:schedule(backend.now + math.max(seconds, FRAME), realm, function()
			backend:resume(thread)
		end)
		coroutine.yield()
	end
	function realm.delay(seconds: number, fn)
		local thread = coroutine.create(fn)
		realm.timers[thread] = backend:schedule(backend.now + math.max(seconds, 0), realm, function()
			realm.timers[thread] = nil
			backend:resume(thread)
		end)
		return thread
	end
	function realm.cancel(thread)
		local event = realm.timers[thread]
		if event then
			event.cancelled = true
			realm.timers[thread] = nil
		end
	end
	function realm.random()
		realm.seed = (realm.seed * 1103515245 + 12345) % 2147483648
		return realm.seed / 2147483648
	end
	-- Runs `fn` on a realm thread; `handle.done` and `handle.value` report its return.
	function realm.run(fn)
		local handle = { done = false }
		realm.spawn(function()
			handle.value = fn()
			handle.done = true
		end)
		return handle
	end
	function realm.join(userId: number)
		local player = { UserId = userId, Name = `Player{userId}` }
		realm.present[player] = true
		realm.population += 1
		return player
	end
	function realm.leave(player)
		if realm.present[player] then
			realm.present[player] = nil
			realm.population -= 1
		end
	end
	function realm.kicked(player)
		for _, kick in realm.kicks do
			if kick.player == player then
				return kick.message
			end
		end
		return nil
	end
	-- A crash: nothing of this realm runs again, and its unanswered calls never return. Their commits still land.
	function realm.crash()
		realm.dead = true
	end

	realm.port = {
		update = function(store, key, transform)
			return backend:call(realm, "update", store, key, transform)
		end,
		read = function(store, key)
			return backend:call(realm, "read", store, key)
		end,
		budget = function(kind)
			return realm.budget[kind]
		end,
		wall = realm.wall,
		serverId = id,
		jobId = `job-{id}`,
		guid = function()
			realm.guids += 1
			return `{id}-token{realm.guids}`
		end,
		random = realm.random,
		spawn = function(fn)
			realm.spawn(fn)
		end,
		delay = realm.delay,
		cancel = realm.cancel,
		resume = function(thread)
			backend:resume(thread)
		end,
		wait = realm.wait,
		players = {
			count = function()
				return realm.population
			end,
			present = function(player)
				return realm.present[player] == true
			end,
			byUserId = function(userId)
				for player in realm.present do
					if player.UserId == userId then
						return player
					end
				end
				return nil
			end,
			kick = function(player, message)
				table.insert(realm.kicks, { player = player, message = message, at = backend.now })
			end,
		},
	}

	local boot = engine.port
	boot.clock = realm.clock
	boot.wait = function()
		realm.wait(FRAME)
	end
	boot.spawn = function(fn)
		realm.spawn(fn)
	end
	boot.bindToClose = function(fn)
		table.insert(realm.closers, fn)
	end
	boot.defer = function(fn)
		realm.spawn(fn)
	end

	realm.graph = spec.runtime(options.env, engine.loggingPort, nil, realm.port)
	realm.boot = realm.graph.module("internal/boot")
	realm.limits = realm.graph.module("internal/data/limits")
	realm.lease = realm.graph.module("internal/data/lease")
	realm.envelope = realm.graph.module("internal/data/envelope")
	realm.scheduler = realm.graph.module("internal/data/scheduler")
	realm.shutdown = realm.graph.module("internal/data/shutdown")
	local profiles = realm.graph.module("internal/data/profiles")
	realm.data = profiles.DataEngine.new(realm.port)

	-- Defines framework-shaped Subsystems, then ignites the realm so bootClock() is this realm's clock (rule 25).
	local emit = realm.graph.module("emit")
	local Subsystem = realm.graph.module("subsystem").Subsystem
	for _, definition in options.subsystems or {} do
		local class = setmetatable(definition.methods or {}, { __index = Subsystem })
		class.__index = class
		function class.new()
			local object = setmetatable({}, class)
			Subsystem.constructor(object)
			return object
		end
		emit.defineClass(class, { kind = "subsystem", id = definition.id, deps = definition.deps or {}, provides = {} })
	end
	realm.boot.ignite(boot, { name = "server", renders = false, booted = function() end }, {})

	-- A profile store's engine with the D1 seams: identity load, no claims, and a recorded slot.
	function realm.profile(name: string, defaults, version: number?)
		local published = {}
		local reasons = {}
		local runtime = realm.data:profile({ name = name, version = version or 0, defaults = defaults or {} }, {
			onLoad = function(_, value, reason)
				table.insert(reasons, reason)
				return value
			end,
			onRelease = function(_, value)
				return value
			end,
			publish = function(player, value)
				published[player] = value
			end,
			delivered = function() end,
		})
		runtime.published = published
		runtime.reasons = reasons
		return runtime
	end

	-- Starts core's shutdown sweep on a realm thread, as BindToClose would.
	function realm.close()
		return realm.run(function()
			realm.closers[1]()
			return true
		end)
	end

	return realm
end

Backend.copy = copy
Backend.FRAME = FRAME

return Backend
