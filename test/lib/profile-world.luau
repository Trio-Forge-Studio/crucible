-- Two-realm profile store specs: the sync world's server and clients over networking's fake link, with the server's
-- data port served by one shared data backend (data rule 108). The backend runs on the link's timeline, so data's
-- timers, storage latency, sync frames and network delivery advance together, and data's monotonic clock is the
-- link's boot clock (rule 25).
local Backend = require("./data-backend")
local SyncWorld = require("./sync-world")

local FRAME = 1 / 60

local World = setmetatable({}, { __index = SyncWorld })
World.__index = World

local function present(link)
	local players = {}
	for player in link.present do
		table.insert(players, player)
	end
	table.sort(players, function(a, b)
		return a.UserId < b.UserId
	end)
	return players
end

-- The server realm's data port. Storage calls go to the shared backend, whose events run as link timers.
local function dataPort(self, realm)
	local link = realm.link
	local backend = self.backend
	rawset(backend, "now", nil)
	setmetatable(backend, {
		__index = function(_, key)
			if key == "now" then
				return link.now
			end
			return Backend[key]
		end,
	})
	function backend.schedule(_, time: number, owner, fn)
		local event = { realm = owner }
		link:delay(math.max(time - link.now, 0), function()
			if event.cancelled or (owner and owner.dead) then
				return
			end
			fn()
		end)
		return event
	end
	local state = {
		id = "server1",
		latency = 0.1,
		budget = { update = 1000, read = 1000 },
		dead = false,
		suspended = false,
		seed = 7919,
		guids = 0,
		wallOffset = 1700000000,
		timers = {},
	}
	self.dataRealm = state
	local function resume(thread, ...)
		backend:resume(thread, ...)
	end
	return {
		update = function(store, key, transform)
			return backend:call(state, "update", store, key, transform)
		end,
		read = function(store, key)
			return backend:call(state, "read", store, key)
		end,
		budget = function(kind)
			return state.budget[kind]
		end,
		wall = function()
			return link.now + state.wallOffset
		end,
		serverId = state.id,
		jobId = `job-{state.id}`,
		guid = function()
			state.guids += 1
			return `{state.id}-token{state.guids}`
		end,
		random = function()
			state.seed = (state.seed * 1103515245 + 12345) % 2147483648
			return state.seed / 2147483648
		end,
		spawn = function(fn)
			resume(coroutine.create(fn))
		end,
		delay = function(seconds: number, fn)
			local thread = coroutine.create(fn)
			state.timers[thread] = backend:schedule(link.now + math.max(seconds, 0), state, function()
				state.timers[thread] = nil
				resume(thread)
			end)
			return thread
		end,
		cancel = function(thread)
			local event = state.timers[thread]
			if event then
				event.cancelled = true
				state.timers[thread] = nil
			end
		end,
		resume = function(thread)
			resume(thread)
		end,
		wait = function(seconds: number)
			local thread = coroutine.running()
			backend:schedule(link.now + math.max(seconds, FRAME), state, function()
				resume(thread)
			end)
			coroutine.yield()
		end,
		players = {
			count = function()
				return #present(link)
			end,
			present = function(player)
				return link.present[player] ~= nil
			end,
			byUserId = function(userId: number)
				for player in link.present do
					if player.UserId == userId then
						return player
					end
				end
				return nil
			end,
			kick = function(player, message: string)
				table.insert(self.kicks, { player = player, message = message, at = link.now })
			end,
			all = function()
				return present(link)
			end,
			onAdded = function(listener)
				table.insert(self.added, listener)
				return function()
					table.remove(self.added, table.find(self.added, listener))
				end
			end,
			onRemoving = function(listener)
				table.insert(self.removing, listener)
				return function()
					table.remove(self.removing, table.find(self.removing, listener))
				end
			end,
		},
		marketplace = function()
			return self.marketplace
		end,
	}
end

-- options are the sync world's: declare(realm) returns the declarations both realms evaluate, serve(server, d, world)
-- defines server Subsystems. options.backend shares a backend between worlds.
function World.new(options)
	local self = {
		backend = options.backend or Backend.new(),
		added = {},
		removing = {},
		kicks = {},
		marketplace = {},
	}
	local merged = table.clone(options)
	merged.data = function(realm)
		return dataPort(self, realm)
	end
	local world = SyncWorld.new(merged)
	for key, value in self do
		world[key] = value
	end
	setmetatable(world, World)
	local link = world.link
	local leave = link.leave
	function link.leave(linkSelf, client)
		local wasPresent = linkSelf.present[client.player] ~= nil
		leave(linkSelf, client)
		if wasPresent then
			for _, listener in table.clone(world.removing) do
				listener(client.player)
			end
		end
	end
	world.server.dataApi = world.server.graph.module("")
	return world
end

-- A client realm for player `userId`, joined to the server's data as PlayerAdded.
function World:player(name: string, userId: number, options)
	options = table.clone(options or {})
	options.userId = userId
	local client = self:join(name, options)
	for _, listener in table.clone(self.added) do
		listener(client.player)
	end
	self.link:flush()
	return client
end

-- Advances the shared timeline, then raises any error a backend-resumed thread or the link recorded.
function World:advance(seconds: number)
	self.link:advance(seconds)
	if #self.backend.errors > 0 then
		local errors = table.concat(self.backend.errors, "\n")
		table.clear(self.backend.errors)
		error(errors, 2)
	end
end

-- Lets every load, write and frame settle: storage latency, then a Heartbeat that sends frames.
function World:settle(seconds: number?)
	self:advance(seconds or 0.5)
	self:heartbeat()
end

-- Runs `fn` on a server thread; `box.done` and `box.value` report its return.
function World:run(fn)
	return self.link:run(fn)
end

-- The kick message the server sent `player`, or nil.
function World:kicked(player)
	for _, kick in self.kicks do
		if kick.player == player then
			return kick.message
		end
	end
	return nil
end

-- The committed envelope at a profile key.
function World:envelope(store: string, userId: number)
	return self.backend:envelope(store, tostring(userId))
end

-- The decoded frames to `client` after index `from`, each with its keys.
function World:framesSince(client, from: number)
	local out = {}
	for i, decoded in self:frames(client) do
		if i > from then
			table.insert(out, decoded)
		end
	end
	return out
end

return World
