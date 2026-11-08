-- Loads compiled declaration modules against the compiler-built runtime in the sample place, where the runtime's
-- untransformed macros would raise "needs the @trioforge/crucible/transformer plugin".
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local engine = require("../../../../test/lib/engine")
local networkPort = require("../../../../test/lib/network-port")

local placePath, runtime, gameOut = process.args[1], process.args[2], process.args[3]
assert(placePath and runtime and gameOut, "usage: declarations.luau <place> <runtime out> <game out>")

roblox.implementMethod("Instance", "WaitForChild", function(instance, name: string)
	return assert(instance:FindFirstChild(name), `{instance:GetFullName()} has no child {name}`)
end)

local place = roblox.deserializePlace(fs.readFile(placePath))

local function source(path: string): string
	return fs.readFile(if fs.isFile(`{path}.luau`) then `{path}.luau` else `{path}/init.luau`)
end

local function swap(instance, path: string)
	if instance:IsA("ModuleScript") then
		instance.Source = source(path)
	end
	for _, child in instance:GetChildren() do
		swap(child, `{path}/{child.Name}`)
	end
end
swap(place:GetService("ReplicatedStorage").rbxts_include.node_modules["@trioforge"].crucible.out, runtime)

local function module(service: string, name: string)
	local parent = place:GetService(service)
	local folder = parent:FindFirstChild("TS")
	if folder == nil then
		folder = roblox.Instance.new("Folder")
		folder.Name = "TS"
		folder.Parent = parent
	end
	local script = roblox.Instance.new("ModuleScript")
	script.Name = name
	script.Source = source(`{gameOut}/{if service == "ServerScriptService" then "server" else "shared"}/{name}`)
	script.Parent = folder
	return script
end
local messages = module("ReplicatedStorage", "messages")
local data = module("ServerScriptService", "data")

local e = engine.new()
local runService = {
	Heartbeat = {},
	IsRunning = function()
		return false
	end,
	IsStudio = function()
		return false
	end,
	IsClient = function()
		return false
	end,
	IsServer = function()
		return true
	end,
}
local game = {
	GetService = function(_self, name: string)
		return if name == "RunService" then runService else place:GetService(name)
	end,
}
local realm = {}
local loaded = {}
local function load(target)
	if target.Name == "roblox-port" and target.Parent.Name == "internal" then
		return {
			robloxPort = function()
				return e.port
			end,
		}
	end
	if loaded[target] == nil then
		local chunk = luau.load(target.Source, {
			debugName = target:GetFullName(),
			environment = { _G = realm, game = game, script = target, require = load, task = task },
		})
		loaded[target] = { value = chunk() }
		if target.Name == "port" and target.Parent.Name == "network" then
			loaded[target].value.installNetworkPort(networkPort())
		end
	end
	return loaded[target].value
end

local shared = load(messages)
assert(type(shared.Ping) == "table", "toServer declares a message token")
assert(shared.Score ~= nil, "replicated declares a state")
local server = load(data)
assert(server.Saves ~= nil, "dataStore declares a store")
assert(server.Limit ~= nil, "config declares a config")
print("declarations load")
