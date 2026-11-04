local loadPlace = require("./lib/place")
local spec = require("./lib/load")

-- A realm's place, with the testing entry and the sample's modules loaded into its one module graph.
local function world(client: boolean?)
	local h = loadPlace(client)
	local server = h.place:GetService("ServerScriptService").TS.systems
	local scripts = h.place:GetService("StarterPlayer").StarterPlayerScripts.TS.systems
	local messages = h.import(h.place:GetService("ReplicatedStorage").TS.messages)
	h.testing = h.import(h.crucible.testing)
	h.root = h.import(h.crucible)
	h.emit = h.import(h.crucible.emit)
	h.Collect = messages.Collect
	h.Collected = messages.Collected
	if client then
		h.Hud = h.import(scripts.hud).Hud
	else
		h.Controller = h.import(server.controller).Controller
		h.Store = h.import(server.store).Store
		h.Member = h.import(server.member).Member
		h.Wallet = h.import(server.wallet).Wallet
	end
	return h
end

local function fails(label: string, run: () -> any, expected: string)
	local ok, err = pcall(run)
	assert(not ok, `{label}: expected an error`)
	spec.has(err, expected, label)
end

-- A Luau subsystem the transformer never saw, registered the way its output registers a class.
local function subsystemClass(h, id: string, constructor: (any) -> ())
	local class = setmetatable({}, {
		__tostring = function()
			return id
		end,
		__index = h.root.Subsystem,
	})
	class.__index = class
	class.new = function(...)
		local self = setmetatable({}, class)
		return self:constructor(...) or self
	end
	class.constructor = function(self)
		h.root.Subsystem.constructor(self)
		constructor(self)
	end
	h.emit.defineClass(class, { kind = "subsystem", id = id, deps = {}, provides = {} })
	return class
end

-- A stand-in for a Part, which answers GetAttribute from the table it was given.
local function part(attributes: { [string]: any })
	return {
		GetAttribute = function(_, name: string)
			return attributes[name]
		end,
	}
end

local function named(name: string)
	return setmetatable({}, {
		__tostring = function()
			return name
		end,
	})
end

local OUTSIDE = "constructed outside Crucible"
local SERVER_SYSTEMS = "@trioforge/crucible-sample-game/server/systems"
local MEMBER = `{SERVER_SYSTEMS}/member:Member`
local STORE = `{SERVER_SYSTEMS}/store:Store`

return {
	["a Subsystem builds with a fake dependency, runs the hooks a spec calls, and each call builds a new object"] = function()
		local h = world()
		local store = { ready = true }
		local controller = h.testing.subsystem(h.Controller, store)
		spec.eq(controller.store, store, "the constructor got the fake")
		spec.list(h.messages, {}, "building runs no hook")
		controller:awake()
		controller:start()
		controller:onShutdown()
		spec.list(
			h.messages,
			{ "S2 controller awake", "S2 controller start", "S2 controller shutdown" },
			"only the hooks the spec called ran"
		)
		assert(h.testing.subsystem(h.Controller, store) ~= controller, "each call builds a new object")
		fails("a dependency that is not ready", function()
			h.testing.subsystem(h.Controller, { ready = false }):awake()
		end, "S2 dependency must be awake")
		local built = h.testing.subsystem(h.Store)
		built:awake()
		spec.eq(built.ready, true, "a class with no arguments")
		h.place:Destroy()
	end,
	["new without the entry still fails, also after a build whose constructor threw or yielded"] = function()
		local h = world()
		fails("before any build", function()
			h.Store.new()
		end, OUTSIDE)
		h.testing.subsystem(h.Store)
		fails("after a build", function()
			h.Store.new()
		end, OUTSIDE)
		local Throws = subsystemClass(h, "Throws", function()
			error("the constructor broke", 0)
		end)
		local Yields = subsystemClass(h, "Yields", function()
			coroutine.yield()
		end)
		local ok, err = pcall(h.testing.subsystem, Throws)
		spec.eq(ok, false, "a throwing constructor fails the build")
		spec.has(err, "the constructor broke", "its message")
		fails("after a constructor that threw", function()
			h.Store.new()
		end, OUTSIDE)
		pcall(h.testing.subsystem, Yields)
		fails("after a constructor that yielded", function()
			h.Store.new()
		end, OUTSIDE)
		h.place:Destroy()
	end,
	["a constructor that yields is refused"] = function()
		local h = world()
		local Yields = subsystemClass(h, "Yields", function()
			coroutine.yield()
		end)
		local ok, err = pcall(h.testing.subsystem, Yields)
		spec.eq(ok, false, "the build fails")
		spec.eq(err, "Yields's constructor yielded; boot faults a constructor that yields", "the message")
		h.place:Destroy()
	end,
	["a behaviour binds to its instance, reads its attributes, and runs its hooks on demand"] = function()
		local h = world()
		local instance = part({ Identity = "ann", Speed = 30 })
		local member = h.testing.behaviour(h.Member, instance)
		spec.eq(member.instance, instance, "bound to the instance")
		spec.eq(member.identity, "ann", "a required attribute")
		spec.eq(member.speed, 30, "an attribute the instance sets")
		spec.eq(h.testing.behaviour(h.Member, part({ Identity = "bob" })).speed, 17, "the fallback applies")
		spec.list(h.messages, {}, "building runs no hook")
		member:awake()
		member:onEnable()
		member:onDestroy()
		spec.list(
			h.messages,
			{ "S2 member awake ann 30", "S2 member enable", "S2 member destroy" },
			"only the hooks the spec called ran"
		)
		fails("a missing required attribute", function()
			h.testing.behaviour(h.Member, part({}))
		end, MEMBER .. ": attribute Identity is invalid (nil)")
		fails("a wrong-typed attribute", function()
			h.testing.behaviour(h.Member, part({ Identity = 5 }))
		end, MEMBER .. ": attribute Identity is invalid (number)")
		fails("a wrong-typed optional attribute", function()
			h.testing.behaviour(h.Member, part({ Identity = "ann", Speed = "fast" }))
		end, MEMBER .. ": attribute Speed is invalid (string)")
		fails("new after a refused build", function()
			h.Member.new()
		end, OUTSIDE)
		h.place:Destroy()
	end,
	["a server Subsystem's receive field registers and its callback is returned"] = function()
		local h = world()
		spec.eq(h.testing.receiver(h.Collect), nil, "nothing receives before the build")
		h.testing.subsystem(h.Wallet)
		local receive = h.testing.receiver(h.Collect)
		spec.eq(type(receive), "function", "the receive field's callback")
		spec.eq(receive({ Name = "ann" }, { size = 0 }), nil, "a callback runs as the game wrote it")
		fails("a second build", function()
			h.testing.subsystem(h.Wallet)
		end, "one Subsystem publishes each value")
		fails("a value that is not a message", function()
			h.testing.receiver({})
		end, "not a message")
		h.place:Destroy()
	end,
	["a receiving Subsystem cannot be built twice in one module graph"] = function()
		local h = world()
		local Listens = subsystemClass(h, "Listens", function(self)
			self.collect = h.emit.receive(self, "collect", h.Collect, function() end)
		end)
		h.testing.subsystem(Listens)
		fails("a second build", function()
			h.testing.subsystem(Listens)
		end, "Coins.Collect has two receivers")
		h.place:Destroy()
	end,
	["a client Subsystem's receive field registers on the client realm, and a server receiver is refused there"] = function()
		local h = world(true)
		local hud = h.testing.subsystem(h.Hud)
		spec.eq(h.testing.registration(h.Hud).kind, "subsystem", "a client class registers")
		local receive = h.testing.receiver(h.Collected)
		spec.eq(type(receive), "function", "the client receiver")
		receive({ gained = 7 })
		spec.eq(hud.lastGain(), 7, "the callback reached the Subsystem")
		local Wrong = subsystemClass(h, "Wrong", function(self)
			self.collect = h.emit.receive(self, "collect", h.Collect, function() end)
		end)
		fails("a server receiver on the client", function()
			h.testing.subsystem(Wrong)
		end, "Wrong.collect: Coins.Collect is client-to-server; receive it in a server Subsystem.")
		h.place:Destroy()
	end,
	["subsystem refuses a behaviour class and behaviour refuses a Subsystem class"] = function()
		local h = world()
		fails("a behaviour", function()
			h.testing.subsystem(h.Member)
		end, `{MEMBER} is a behaviour; build it with behaviour()`)
		fails("a Subsystem", function()
			h.testing.behaviour(h.Store, part({}))
		end, `{STORE} is a Subsystem; build it with subsystem()`)
		fails("an unregistered class", function()
			h.testing.subsystem(named("Loose"))
		end, "Loose is not a registered Crucible class; compile it with the Crucible transformer")
		h.place:Destroy()
	end,
	["message, transfer, sealMessages, registration and n describe what the place declared"] = function()
		local h = world()
		local t = h.testing
		local collect = t.message(h.Collect)
		spec.eq(collect.name, "Coins.Collect", "the name")
		spec.eq(collect.kind, "to-server", "the kind")
		spec.eq(collect.input.kind, "object", "the input node")
		spec.eq(collect.input.fields[1][1], "size", "the field")
		spec.eq(collect.input.fields[1][2].kind, "u8", "its type")
		spec.eq(collect.output, nil, "a send has no output")
		spec.eq(t.message({}), nil, "not a message")
		spec.eq(t.transfer(h.Collect, { size = 200 }).size, 200, "a value round trips")
		fails("an out-of-range u8", function()
			t.transfer(h.Collect, { size = 300 })
		end, "Coins.Collect refused to encode")
		fails("an input that is not a table", function()
			t.transfer(h.Collect, 5)
		end, "Coins.Collect refused to encode payload: expected table")
		fails("an output a send does not have", function()
			t.transfer(h.Collect, { size = 1 }, "output")
		end, "Coins.Collect has no output codec")
		local names = t.sealMessages()
		spec.eq(table.find(names, "Coins.Collect") ~= nil, true, "Collect is listed")
		spec.eq(table.find(names, "Coins.Collected") ~= nil, true, "Collected is listed")
		local sorted = table.clone(names)
		table.sort(sorted)
		spec.list(names, sorted, "in wire order")
		fails("a second seal", function()
			t.sealMessages()
		end, "catalog already sealed")
		local member = t.registration(h.Member)
		spec.eq(member.kind, "behaviour", "the kind")
		spec.eq(member.id, MEMBER, "the id")
		spec.eq(member.tag, "S2Member", "the tag")
		spec.list(member.instanceClasses, { "Part" }, "the instance classes")
		spec.eq(t.registration(h.Store).kind, "subsystem", "a Subsystem")
		spec.eq(t.n.u8.kind, "u8", "the schema builders")
		h.place:Destroy()
	end,
}
