local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local task = require("@lune/task")

roblox.implementMethod("Instance", "WaitForChild", function(instance, name: string)
	return assert(instance:FindFirstChild(name), `{instance:GetFullName()} has no child {name}`)
end)

return function(client: boolean?)
	local place = roblox.deserializePlace(fs.readFile("build/sample-game.rbxl"))
	local runService = {
		Heartbeat = {},
		IsRunning = function()
			return false
		end,
		IsStudio = function()
			return false
		end,
		IsClient = function()
			return client == true
		end,
	}
	local game = {
		GetService = function(_self, name: string)
			return if name == "RunService" then runService else place:GetService(name)
		end,
	}
	local realm = {}
	local loaded = {}
	local function load(module)
		if loaded[module] == nil then
			local chunk = luau.load(module.Source, {
				debugName = module:GetFullName(),
				environment = {
					_G = realm,
					game = game,
					script = module,
					require = load,
					task = task,
				},
			})
			loaded[module] = { value = chunk() }
		end
		return loaded[module].value
	end
	local include = place:GetService("ReplicatedStorage").rbxts_include
	local runtime = load(include.RuntimeLib)

	return {
		place = place,
		crucible = include.node_modules["@trioforge"].crucible.out,
		loaded = loaded,
		load = load,
		import = function(module)
			return runtime.import(include.RuntimeLib, module)
		end,
	}
end
