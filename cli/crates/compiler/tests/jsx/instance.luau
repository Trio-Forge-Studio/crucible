-- A fake Instance for the JSX specs. An Instance is a userdata proxy, as on Roblox, because Vide tells children from
-- property tables by `type`. Properties live in the proxy's metatable. `Parent` keeps a child set, and `Destroy`
-- unparents the subtree, locks its Parent and disconnects its connections. The names in EVENTS read as signals, and
-- `fire` runs their handlers at once.
local fake = {}
local roblox = require("@lune/roblox")

local EVENTS = { Activated = true, MouseEnter = true, MouseLeave = true }

local methods = {}
local signals = setmetatable({}, { __mode = "k" })
local connections = setmetatable({}, { __mode = "k" })

local function reparent(instance, parent)
	local own = getmetatable(instance)
	assert(not own.__destroyed, `the Parent property of {own.Name} is locked`)
	local previous = own.Parent
	if previous == parent then
		return
	end
	if previous ~= nil then
		getmetatable(previous).__children[instance] = nil
	end
	own.Parent = parent
	if parent ~= nil then
		getmetatable(parent).__children[instance] = true
	end
end

function methods.Destroy(instance)
	local own = getmetatable(instance)
	if own.__destroyed then
		return
	end
	reparent(instance, nil)
	own.__destroyed = true
	for _, signal in own.__signals do
		for connection in signal.handlers do
			connection.Connected = false
		end
		table.clear(signal.handlers)
	end
	for _, child in methods.GetChildren(instance) do
		methods.Destroy(child)
	end
end

function methods.GetChildren(instance)
	local list = {}
	for child in getmetatable(instance).__children do
		table.insert(list, child)
	end
	return list
end

local function connect(signal, handler)
	assert(type(handler) == "function", "Attempt to connect failed: Passed value is not a function")
	local connection = setmetatable({ Connected = true }, {
		__index = {
			Disconnect = function(self)
				self.Connected = false
				signal.handlers[self] = nil
			end,
		},
	})
	connections[connection] = true
	signal.handlers[connection] = handler
	return connection
end

local function signalOf(own, name: string)
	local signal = own.__signals[name]
	if signal == nil then
		signal = setmetatable({ handlers = {} }, { __index = { Connect = connect } })
		signals[signal] = true
		own.__signals[name] = signal
	end
	return signal
end

local function index(instance, key)
	local method = methods[key]
	if method ~= nil then
		return method
	end
	local own = getmetatable(instance)
	if EVENTS[key] then
		return signalOf(own, key)
	end
	return own[key]
end

local function newindex(instance, key, value)
	if key == "Parent" then
		reparent(instance, value)
	else
		assert(not EVENTS[key], `{key} is an event`)
		assert(value ~= nil, `Unable to assign property {key}. Property expected, got nil`)
		getmetatable(instance)[key] = value
	end
end

local function name(instance): string
	return getmetatable(instance).Name
end

function fake.new(className: string): any
	local instance = newproxy(true)
	local meta = getmetatable(instance)
	meta.__index = index
	meta.__newindex = newindex
	meta.__tostring = name
	meta.__children = {}
	meta.__signals = {}
	meta.ClassName = className
	meta.Name = className
	meta.Size = roblox.UDim2.new(0, 100, 0, 100)
	return instance
end

fake.Instance = { new = fake.new }

local function isInstance(value: any): boolean
	if type(value) ~= "userdata" then
		return false
	end
	local meta = getmetatable(value)
	return type(meta) == "table" and meta.__index == index
end

function fake.typeof(value: any): string
	if isInstance(value) then
		return "Instance"
	elseif signals[value] then
		return "RBXScriptSignal"
	elseif connections[value] then
		return "RBXScriptConnection"
	end
	return typeof(value)
end

function fake.fire(instance, event: string, ...)
	local handlers = {}
	for _, handler in signalOf(getmetatable(instance), event).handlers do
		table.insert(handlers, handler)
	end
	for _, handler in handlers do
		handler(...)
	end
end

function fake.isDestroyed(instance): boolean
	return getmetatable(instance).__destroyed == true
end

-- The children of `parent` named `name`, sorted by LayoutOrder, as a UIListLayout would show them.
function fake.named(parent, childName: string): { any }
	local list = {}
	for _, child in methods.GetChildren(parent) do
		if child.Name == childName then
			table.insert(list, child)
		end
	end
	table.sort(list, function(a, b)
		return (a.LayoutOrder or 0) < (b.LayoutOrder or 0)
	end)
	return list
end

-- Each child's `field`, in LayoutOrder.
function fake.texts(parent, childName: string, field: string?): { any }
	local out = {}
	for _, child in fake.named(parent, childName) do
		table.insert(out, child[field or "Text"])
	end
	return out
end

return fake
