-- A fake ConfigService with players and a scripted clock. Requests park on that clock, so a spec moves time with
-- `advance` and nothing waits in real time.
local serde = require("@lune/serde")
local task = require("@lune/task")

local Backend = {}
Backend.__index = Backend

type Scope = "global" | number

function Backend.new()
	local b = setmetatable({
		now = 0,
		timers = {},
		sequence = 0,
		globals = {},
		targeted = {},
		players = {},
		joined = {},
		leaving = {},
		snapshots = {},
		failures = {},
		delays = {},
		requests = {},
		inFlight = 0,
		maxInFlight = 0,
		jitter = 1,
		outdatedOnPublish = true,
		eventOnPublish = true,
		updateListeners = 0,
	}, Backend)
	b.port = b:makePort()
	return b
end

function Backend:sleep(seconds: number)
	self.sequence += 1
	table.insert(self.timers, { at = self.now + seconds, sequence = self.sequence, thread = coroutine.running() })
	coroutine.yield()
end

-- Runs every timer due by `now + seconds` in time order, then sets the clock there.
function Backend:advance(seconds: number)
	local target = self.now + seconds
	while true do
		local index
		for i, timer in self.timers do
			if
				timer.at <= target
				and (
					index == nil
					or timer.at < self.timers[index].at
					or (timer.at == self.timers[index].at and timer.sequence < self.timers[index].sequence)
				)
			then
				index = i
			end
		end
		if index == nil then
			break
		end
		local timer = table.remove(self.timers, index)
		self.now = timer.at
		if coroutine.status(timer.thread) == "suspended" then
			task.spawn(timer.thread)
		end
	end
	self.now = target
end

local function key(scope: Scope): string
	return if scope == "global" then "global" else tostring(scope)
end

function Backend:values(scope: Scope)
	local values = table.clone(self.globals)
	if scope ~= "global" then
		for name, value in self.targeted[scope] or {} do
			values[name] = value
		end
	end
	return values
end

-- "throw" raises from the request; "load-failed" returns a snapshot whose Error is LoadFailed.
function Backend:failNext(scope: Scope, how: "throw" | "load-failed", times: number?)
	local list = self.failures[key(scope)] or {}
	for _ = 1, times or 1 do
		table.insert(list, how)
	end
	self.failures[key(scope)] = list
end

-- Every later request for the scope takes `seconds` of scripted time until changed.
function Backend:delay(scope: Scope, seconds: number)
	self.delays[key(scope)] = seconds
end

function Backend:request(scope: Scope, kind: string): string?
	table.insert(self.requests, `{kind} {key(scope)}`)
	self.inFlight += 1
	self.maxInFlight = math.max(self.maxInFlight, self.inFlight)
	local delay = self.delays[key(scope)]
	if delay and delay > 0 then
		self:sleep(delay)
	end
	self.inFlight -= 1
	local list = self.failures[key(scope)]
	local failure = list and table.remove(list, 1)
	if failure == "throw" then
		error("ConfigService: Failed to load config")
	end
	return failure
end

function Backend:publish(scope: Scope)
	for _, snapshot in self.snapshots do
		if scope == "global" or snapshot.scope == scope then
			if self.outdatedOnPublish then
				snapshot.outdated = true
			end
			if self.eventOnPublish then
				for _, listener in table.clone(snapshot.listeners) do
					listener()
				end
			end
		end
	end
end

-- Ops change a value: every affected snapshot turns outdated and fires UpdateAvailable, keeping its old values.
function Backend:setGlobal(name: string, value: any)
	self.globals[name] = value
	self:publish("global")
end

function Backend:setForPlayer(userId: number, name: string, value: any)
	self.targeted[userId] = self.targeted[userId] or {}
	self.targeted[userId][name] = value
	self:publish(userId)
end

function Backend:snapshot(scope: Scope, failure: string?)
	local snapshot = {
		scope = scope,
		values = self:values(scope),
		outdated = false,
		failed = failure == "load-failed",
		listeners = {},
	}
	table.insert(self.snapshots, snapshot)
	local backend = self
	return {
		get = function(name)
			return snapshot.values[name]
		end,
		refresh = function()
			local refreshFailure = backend:request(scope, "refresh")
			snapshot.failed = refreshFailure == "load-failed"
			if not snapshot.failed then
				snapshot.values = backend:values(scope)
				snapshot.outdated = false
			end
		end,
		outdated = function()
			return snapshot.outdated
		end,
		failed = function()
			return snapshot.failed
		end,
		onUpdateAvailable = function(listener)
			local entry = function()
				listener()
			end
			table.insert(snapshot.listeners, entry)
			backend.updateListeners += 1
			return function()
				local index = table.find(snapshot.listeners, entry)
				if index then
					table.remove(snapshot.listeners, index)
					backend.updateListeners -= 1
				end
			end
		end,
	}
end

local function signal()
	local listeners = {}
	return {
		listeners = listeners,
		connect = function(listener)
			local entry = function(...)
				listener(...)
			end
			table.insert(listeners, entry)
			return function()
				local index = table.find(listeners, entry)
				if index then
					table.remove(listeners, index)
				end
			end
		end,
		fire = function(...)
			for _, listener in table.clone(listeners) do
				listener(...)
			end
		end,
	}
end

function Backend:makePort()
	local backend = self
	local added, removing = signal(), signal()
	self.joined, self.leaving = added, removing
	return {
		global = function()
			return backend:snapshot("global", backend:request("global", "fetch"))
		end,
		forPlayer = function(player)
			return backend:snapshot(player.UserId, backend:request(player.UserId, "fetch"))
		end,
		players = function()
			return table.clone(backend.players)
		end,
		onPlayerAdded = added.connect,
		onPlayerRemoving = removing.connect,
		wait = function(seconds)
			backend:sleep(seconds)
		end,
		random = function()
			return backend.jitter
		end,
		decodeJson = function(text)
			return serde.decode("json", text)
		end,
		jsonBytes = function(value)
			return #serde.encode("json", value)
		end,
	}
end

function Backend:join(userId: number)
	local player = { UserId = userId, Name = `Player{userId}` }
	table.insert(self.players, player)
	self.joined.fire(player)
	return player
end

function Backend:leave(player)
	table.remove(self.players, table.find(self.players, player))
	self.leaving.fire(player)
end

function Backend:count(prefix: string): number
	local count = 0
	for _, request in self.requests do
		if string.sub(request, 1, #prefix) == prefix then
			count += 1
		end
	end
	return count
end

function Backend:playerListeners(): number
	return #self.joined.listeners + #self.leaving.listeners
end

return Backend
