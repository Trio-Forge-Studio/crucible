-- The one fake Instance every variant uses, so Instance cost is equal across Crucible A, Crucible B and Vide. An
-- Instance is a userdata proxy, as on Roblox, because Vide tells children from property tables by `type`. Its
-- properties live in the proxy's own metatable, so an Instance that is only unparented, as Vide leaves a removed
-- row, is collected like any value. Reads and writes go through the metamethods, which stand in for the engine's
-- property call. `Parent` keeps a child set, and `Destroy` unparents and destroys the subtree. Nothing here fires a
-- signal: the places run Deferred signal behaviour, so a property write runs no Luau at the write.
local fake = {}
fake.writes = 0

local methods = {}

local function reparent(instance, parent)
	local own = getmetatable(instance)
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
	local list = {}
	for child in own.__children do
		table.insert(list, child)
	end
	for _, child in list do
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

local function index(instance, key)
	local method = methods[key]
	if method ~= nil then
		return method
	end
	return getmetatable(instance)[key]
end

local function newindex(instance, key, value)
	fake.writes += 1
	if key == "Parent" then
		reparent(instance, value)
	else
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
	meta.ClassName = className
	meta.Name = className
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

-- Roblox's typeof for the values these specs make. Crucible's `own` and Vide's `cleanup` and `apply` ask it.
function fake.typeof(value: any): string
	if isInstance(value) then
		return "Instance"
	end
	return typeof(value)
end

function fake.isDestroyed(instance): boolean
	return getmetatable(instance).__destroyed == true
end

-- The children of `parent` sorted by LayoutOrder, as a UIListLayout would show them.
function fake.ordered(parent): { any }
	local list = methods.GetChildren(parent)
	table.sort(list, function(a, b)
		return a.LayoutOrder < b.LayoutOrder
	end)
	return list
end

return fake
