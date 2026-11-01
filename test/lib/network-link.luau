-- A two-sided fake network for Lune specs. One server realm and any number of client realms, each its own module graph
-- with the fake engine behind the core, joined by a NetworkPort per realm.
--
-- What it models, because the design relies on it:
--   * One remote per client, reliable and in fire order, delivered by `link:flush()`.
--   * The engine's per-remote queue: packets fired before a client connected arrive once it connects.
--   * Buffers copied and sidecars rebuilt on delivery, so no buffer or sidecar table is shared across realms.
--   * Instances are shared Lune Instances. `hide(client, x)` makes one invisible to a client, `clientOnly(client, x)`
--     makes one unknown to the server, and `destroy(x)` removes one on the server while in flight. An invisible
--     Instance leaves its sidecar key absent.
--   * The engine-supplied Player on the server. A client cannot choose it.
--   * A deterministic clock: `advance(seconds)` fires delays, discovery deadlines and handler lifetimes.
--   * `Player:Kick` is recorded with its message, and PlayerRemoving follows on the next flush, after the packets that
--     client already fired, as Studio measured (PlayerRemoving fires 1 to 3 s after Kick returns).
--   * `leave(client)` fires PlayerRemoving at once, so a spec can end a peer from inside a callback.
--   * Raw injection both ways, and fault injection on the port's create, publish, fireServer and discover.
-- What it does not prove: replication, streaming timing, attribute sizes, kick timing or wire size. Studio does.
local roblox = require("@lune/roblox")
local Engine = require("./engine")
local spec = require("./load")

local link = {}
link.__index = link

local GAME = "crucible-path:ServerScriptService/Game"

local function copy(value)
	if type(value) == "buffer" then
		local out = buffer.create(buffer.len(value))
		buffer.copy(out, 0, value)
		return out
	end
	return value
end

function link.new()
	local self = setmetatable({
		now = 0,
		sequence = 0,
		timers = {},
		deferred = {},
		wire = {},
		waiters = {},
		kicks = {},
		log = {},
		errors = {},
		clients = {},
		present = {},
		gone = {},
		hidden = {},
		clientOnly = {},
		destroyed = {},
		failing = {},
		world = roblox.Instance.new("Folder"),
	}, link)
	self.world.Name = "Workspace"
	self.task = {
		spawn = function(callback, ...)
			local thread = if type(callback) == "thread" then callback else coroutine.create(callback)
			self:resume(thread, ...)
			return thread
		end,
		defer = function(callback, ...)
			local args = table.pack(...)
			table.insert(self.deferred, function()
				callback(table.unpack(args, 1, args.n))
			end)
		end,
		delay = function(seconds, callback, ...)
			local args = table.pack(...)
			return self:delay(seconds, function()
				callback(table.unpack(args, 1, args.n))
			end)
		end,
		cancel = function(thread)
			coroutine.close(thread)
		end,
		wait = function()
			error("network specs never call task.wait")
		end,
	}
	self.server = self:realm("server", "Server")
	return self
end

function link:resume(thread, ...)
	local ok, err = coroutine.resume(thread, ...)
	if not ok then
		table.insert(self.errors, debug.traceback(thread, tostring(err)))
	end
end

function link:delay(seconds, callback)
	self.sequence += 1
	local timer = { at = self.now + seconds, order = self.sequence, callback = callback }
	table.insert(self.timers, timer)
	return function()
		timer.cancelled = true
	end
end

-- Live timers, for asserting that a closed session left none.
function link:pending(): number
	local n = 0
	for _, timer in self.timers do
		if not timer.cancelled then
			n += 1
		end
	end
	return n
end

function link:fault(operation: string)
	local skip = self.failing[operation]
	if skip == nil then
		return
	end
	if skip > 0 then
		self.failing[operation] = skip - 1
		return
	end
	self.failing[operation] = nil
	error(`injected {operation} failure`)
end

-- A later call of that port operation throws once: "create", "publish", "fireServer" or "discover". `skip` calls
-- succeed first.
function link:failNext(operation: string, skip: number?)
	self.failing[operation] = skip or 0
end

function link:instance(className: string, name: string?)
	local instance = roblox.Instance.new(className)
	instance.Name = name or className
	instance.Parent = self.world
	return instance
end

function link:hide(client, instance)
	self.hidden[client] = self.hidden[client] or {}
	self.hidden[client][instance] = true
end

function link:only(client, instance)
	self.clientOnly[client] = self.clientOnly[client] or {}
	self.clientOnly[client][instance] = true
end

function link:destroy(instance)
	self.destroyed[instance] = true
end

local function visible(self, to, from, instance)
	if to == self.server then
		return not self.destroyed[instance] and not (self.clientOnly[from] and self.clientOnly[from][instance])
	end
	return not (self.hidden[to] and self.hidden[to][instance])
end

-- Rebuilds a sidecar for its receiver: a fresh table, with the Instances it cannot see left out.
local function rebuild(self, to, from, sidecar)
	if type(sidecar) ~= "table" then
		return sidecar
	end
	local out = {}
	for key, value in sidecar do
		if typeof(value) ~= "Instance" or visible(self, to, from, value) then
			out[key] = value
		end
	end
	return out
end

local function record(self, from, to, args, injected)
	local packet = args[1]
	local slots = 0
	if type(args[2]) == "table" then
		for _ in args[2] do
			slots += 1
		end
	end
	table.insert(self.log, {
		from = from.name,
		to = to.name,
		opcode = if type(packet) == "buffer" and buffer.len(packet) > 0 then buffer.readu8(packet, 0) else nil,
		packet = copy(packet),
		slots = slots,
		injected = injected,
	})
end

function link:transmit(from, to, packet, sidecar)
	local args = table.pack(copy(packet), table.clone(sidecar))
	record(self, from, to, args)
	table.insert(self.wire, { from = from, to = to, args = args, visibility = true })
end

-- Raw remote arguments from a client, bypassing its session: any count, any types.
function link:inject(client, ...)
	local args = table.pack(...)
	record(self, client, self.server, args, true)
	table.insert(self.wire, { from = client, to = self.server, args = args })
end

-- Raw remote arguments from the server to one client.
function link:injectToClient(client, ...)
	local args = table.pack(...)
	record(self, self.server, client, args, true)
	table.insert(self.wire, { from = self.server, to = client, args = args })
end

function link:sent(filter)
	local out = {}
	for _, entry in self.log do
		if filter == nil or filter(entry) then
			table.insert(out, entry)
		end
	end
	return out
end

-- "deliver", "wait" for a client that has not connected yet (the engine's per-remote queue), or "drop".
local function route(self, entry): string
	if entry.leave then
		return "deliver"
	end
	if entry.to == self.server then
		return if self.server.remote ~= nil and self.present[entry.from.player] ~= nil then "deliver" else "drop"
	end
	if self.present[entry.to.player] == nil then
		return "drop"
	end
	return if entry.to.receive ~= nil then "deliver" else "wait"
end

local function deliver(self, entry)
	if entry.leave then
		self:leave(entry.leave)
		return
	end
	local args = entry.args
	if entry.visibility then
		args = table.pack(args[1], rebuild(self, entry.to, entry.from, args[2]))
	end
	if entry.to == self.server then
		self.server.remote.receive(entry.from.player, table.unpack(args, 1, args.n))
	else
		entry.to.receive(table.unpack(args, 1, args.n))
	end
end

-- One step of the engine: a deferred task, else the oldest deliverable packet, else a discovery that can finish.
function link:step(): boolean
	for _, realm in self:realms() do
		if #realm.engine.deferred > 0 then
			table.remove(realm.engine.deferred, 1)()
			return true
		end
	end
	if #self.deferred > 0 then
		table.remove(self.deferred, 1)()
		return true
	end
	for index, entry in self.wire do
		local action = route(self, entry)
		if action ~= "wait" then
			table.remove(self.wire, index)
			if action == "deliver" then
				deliver(self, entry)
			end
			return true
		end
	end
	for index, waiter in self.waiters do
		if waiter.ready() then
			table.remove(self.waiters, index)
			self:resume(waiter.thread)
			return true
		end
	end
	return false
end

function link:flush()
	repeat
		local stepped = self:step()
	until not stepped
end

-- Moves the clock forward, firing due timers in time order, each followed by a flush.
function link:advance(seconds: number)
	local target = self.now + seconds
	self:flush()
	while true do
		local nextTimer
		for _, timer in self.timers do
			if not timer.cancelled and timer.at <= target then
				if
					nextTimer == nil
					or timer.at < nextTimer.at
					or (timer.at == nextTimer.at and timer.order < nextTimer.order)
				then
					nextTimer = timer
				end
			end
		end
		if nextTimer == nil then
			break
		end
		nextTimer.cancelled = true
		self.now = math.max(self.now, nextTimer.at)
		nextTimer.callback()
		self:flush()
	end
	self.now = target
	self:flush()
end

function link:realms()
	local all = { self.server }
	for _, client in self.clients do
		table.insert(all, client)
	end
	return all
end

local function serverPort(self, realm)
	return {
		create = function(receive)
			self:fault("create")
			realm.remote = { receive = receive, published = false }
		end,
		publish = function(protocol, catalog)
			self:fault("publish")
			realm.remote.attributes = { Protocol = protocol, Catalog = catalog }
			realm.remote.published = true
		end,
		fireClient = function(player, packet, sidecar)
			self:transmit(realm, self.present[player] or self.gone[player], packet, sidecar)
		end,
		players = function()
			local players = {}
			for _, client in self.clients do
				if self.present[client.player] then
					table.insert(players, client.player)
				end
			end
			return players
		end,
		onPlayerAdded = function(listener)
			realm.added = listener
			return function()
				realm.added = nil
			end
		end,
		onPlayerRemoving = function(listener)
			realm.removing = listener
			return function()
				realm.removing = nil
			end
		end,
		kick = function(player, message)
			table.insert(self.kicks, { player = player.Name, message = message })
			local client = self.present[player]
			if client and not client.kicked then
				client.kicked = true
				table.insert(self.wire, { leave = client })
			end
		end,
		destroy = function()
			realm.remote = nil
		end,
		delay = function(seconds, callback)
			return self:delay(seconds, callback)
		end,
		resume = function(thread, ...)
			self:resume(thread, ...)
		end,
	}
end

local function clientPort(self, realm)
	return {
		discover = function(timeout, receive)
			self:fault("discover")
			local deadline = self.now + timeout
			local function found()
				local remote = self.server.remote
				return remote ~= nil and remote.published
			end
			if not found() then
				table.insert(self.waiters, {
					thread = coroutine.running(),
					ready = function()
						return found() or self.now >= deadline
					end,
				})
				coroutine.yield()
			end
			if not found() then
				return nil
			end
			local discovery = {
				className = "RemoteEvent",
				protocol = self.server.remote.attributes.Protocol,
				catalog = self.server.remote.attributes.Catalog,
			}
			if realm.tamper then
				realm.tamper(discovery)
			end
			if discovery.className == "RemoteEvent" then
				realm.receive = receive
			end
			return discovery
		end,
		disconnect = function()
			realm.receive = nil
		end,
		fireServer = function(packet, sidecar)
			self:fault("fireServer")
			self:transmit(realm, self.server, packet, sidecar)
		end,
		delay = function(seconds, callback)
			return self:delay(seconds, callback)
		end,
		resume = function(thread, ...)
			self:resume(thread, ...)
		end,
	}
end

-- A realm: its own module graph and fake engine, a fake `game` with the services boot reads, and a network port.
-- `realm.setup(realm)` runs as its one game module, `ServerScriptService.Game`, during boot's load.
function link:realm(side: string, name: string)
	local engine = Engine.new()
	local realm = { link = self, side = side, name = name, engine = engine, attributes = {}, closers = {} }
	engine.port.defer = self.task.defer
	engine.port.bindToClose = function(callback)
		if realm.bindToCloseError then
			error(realm.bindToCloseError)
		end
		table.insert(realm.closers, callback)
	end
	engine.port.spawn = function(callback)
		self.task.spawn(callback)
	end
	engine.port.clock = function()
		return self.now
	end
	engine.port.wait = function()
		error("network specs never yield in onShutdown")
	end
	local module = {
		Name = "Game",
		ClassName = "ModuleScript",
		IsA = function(node, class)
			return node.ClassName == class
		end,
		GetChildren = function()
			return {}
		end,
		GetFullName = function()
			return "ServerScriptService.Game"
		end,
	}
	local services = {
		ServerScriptService = {
			GetChildren = function()
				return { module }
			end,
		},
		RunService = {
			IsServer = function()
				return side == "server"
			end,
			IsClient = function()
				return side == "client"
			end,
		},
		Players = {
			LocalPlayer = {
				SetAttribute = function(_, key, value)
					realm.attributes[key] = value
				end,
			},
		},
	}
	local game = {
		GetService = function(_, service)
			return assert(services[service], service)
		end,
	}
	realm.port = if side == "server" then serverPort(self, realm) else clientPort(self, realm)
	realm.graph = spec.runtime({
		game = game,
		task = self.task,
		typeof = roblox.typeof,
		Vector3 = roblox.Vector3,
		Color3 = roblox.Color3,
		CFrame = roblox.CFrame,
		require = function(required)
			assert(required == module, "only the Game module loads")
			if realm.required == nil then
				realm.required = {}
				if realm.setup then
					realm.setup(realm)
				end
			end
			return realm.required
		end,
	}, engine.loggingPort, realm.port)
	-- Boot reads the engine port through roblox-port; patching the module table before boot loads swaps in the fake.
	realm.graph.module("internal/roblox-port").robloxPort = function()
		return engine.port
	end
	realm.emit = realm.graph.module("emit")
	realm.n = realm.emit.n
	realm.ClientReady = realm.graph.module("network").ClientReady
	realm.life = realm.graph.module("internal/lifecycle")
	realm.registry = realm.graph.module("internal/registry")
	realm.catalog = realm.graph.module("internal/network/catalog")
	realm.claims = realm.graph.module("internal/network/claims")
	realm.construction = realm.graph.module("internal/construction-claim")
	realm.limits = realm.graph.module("internal/network/limits")
	realm.Subsystem = realm.graph.module("subsystem").Subsystem
	realm.Behaviour = realm.graph.module("crucible-behaviour").CrucibleBehaviour
	function realm.service(id, init, methods, options)
		options = options or {}
		local class = setmetatable(methods or {}, { __index = realm.Subsystem })
		class.__index = class
		function class.new(...)
			local object = setmetatable({}, class)
			realm.Subsystem.constructor(object)
			if init then
				init(object, ...)
			end
			return object
		end
		realm.emit.defineClass(class, { kind = "subsystem", id = id, deps = options.deps or {}, provides = {} })
		return class
	end
	function realm.behaviour(id, init, methods)
		local class = setmetatable(methods or {}, { __index = realm.Behaviour })
		class.__index = class
		function class.new(...)
			local object = setmetatable({}, class)
			realm.Behaviour.constructor(object)
			if init then
				init(object, ...)
			end
			return object
		end
		realm.emit.defineClass(class, {
			kind = "behaviour",
			id = id,
			deps = {},
			provides = {},
			tag = id,
			instance = { "Model" },
		})
		return class
	end
	-- The state of the generation an object is, for internal lifecycle requests a spec makes.
	function realm.state(object)
		return realm.life.stateOf(object)
	end
	function realm.logged(part: string): number
		return engine.logged(part)
	end
	function realm.diagnostics(): { string }
		local out = {}
		for _, entry in engine.logs do
			table.insert(out, entry.message)
		end
		return out
	end
	return realm
end

-- Boots the server realm. `setup(server)` declares its messages and Subsystems during load.
function link:boot(setup)
	local server = self.server
	server.setup = setup
	server.entry = server.graph.module("server")
	server.Network = server.entry.Network
	local ok, err = pcall(server.entry.boot, { load = { GAME } })
	server.booted = ok
	server.bootError = if ok then nil else err
	self:flush()
	return ok, err
end

-- Boots a client realm on its own thread, since discovery may wait for publication.
function link:start(client)
	client.entry = client.graph.module("client")
	client.Network = client.entry.Network
	client.thread = coroutine.create(function()
		local ok, err = pcall(client.entry.boot, { load = { GAME } })
		client.booted = ok
		client.bootError = if ok then nil else err
	end)
	self:resume(client.thread)
	self:flush()
end

-- A player joins: PlayerAdded on the server, then, unless `options.boot` is false, that client's boot.
function link:join(name: string, setup, options)
	options = options or {}
	local client = self:realm("client", name)
	client.setup = setup
	client.tamper = options.tamper
	client.player = roblox.Instance.new("Player")
	client.player.Name = name
	table.insert(self.clients, client)
	self.present[client.player] = client
	if self.server.added then
		self.server.added(client.player)
	end
	if options.boot ~= false then
		self:start(client)
	end
	return client
end

-- The player leaves now: PlayerRemoving fires on the server at once and the client stops receiving.
function link:leave(client)
	if self.present[client.player] == nil then
		return
	end
	self.present[client.player] = nil
	self.gone[client.player] = client
	client.receive = nil
	if self.server.removing then
		self.server.removing(client.player)
	end
end

-- Runs `body` on a thread the link resumes, so it may yield in `Network.ask`. Returns a box its result lands in.
function link:run(body)
	local box = {}
	box.thread = coroutine.create(function()
		box.value = body()
		box.done = true
	end)
	self:resume(box.thread)
	return box
end

function link:assertClean()
	if #self.errors > 0 then
		error(`engine-reported errors:\n{table.concat(self.errors, "\n")}`, 2)
	end
end

return link
