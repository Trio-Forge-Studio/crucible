local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local engine = require("./engine")

roblox.implementMethod("Instance", "WaitForChild", function(instance, name: string)
	return assert(instance:FindFirstChild(name), `{instance:GetFullName()} has no child {name}`)
end)

return function(client: boolean?)
	local place = roblox.deserializePlace(fs.readFile("build/sample-game.rbxl"))
	local e = engine.new()
	local messages = {}
	local closers = {}
	local attributes = {}
	e.port.bindToClose = function(callback)
		table.insert(closers, callback)
	end
	e.port.spawn = function(callback)
		callback()
	end
	e.port.clock = os.clock
	e.port.wait = task.wait
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
		IsServer = function()
			return client ~= true
		end,
	}
	local game = {
		GetService = function(_self, name: string)
			if name == "Players" then
				return {
					LocalPlayer = {
						SetAttribute = function(_, key, value)
							attributes[key] = value
						end,
						GetAttribute = function(_, key)
							return attributes[key]
						end,
					},
				}
			end
			return if name == "RunService" then runService else place:GetService(name)
		end,
	}
	local realm = {}
	local loaded = {}
	local function load(module)
		if module.Name == "roblox-port" and module.Parent.Name == "internal" then
			return {
				robloxPort = function()
					return e.port
				end,
			}
		end
		if loaded[module] == nil then
			local chunk = luau.load(module.Source, {
				debugName = module:GetFullName(),
				environment = {
					_G = realm,
					game = game,
					script = module,
					require = load,
					task = task,
					print = function(...)
						local values = {}
						for _, value in { ... } do
							table.insert(values, tostring(value))
						end
						table.insert(messages, table.concat(values, " "))
					end,
				},
			})
			loaded[module] = { value = chunk() }
			if module.Name == "log-output" and module.Parent.Name == "internal" then
				loaded[module].value.installLoggingPort(e.loggingPort)
			end
		end
		return loaded[module].value
	end
	local include = place:GetService("ReplicatedStorage").rbxts_include
	local runtime = load(include.RuntimeLib)

	return {
		place = place,
		crucible = include.node_modules["@trioforge"].crucible.out,
		loaded = loaded,
		engine = e,
		messages = messages,
		closers = closers,
		attributes = attributes,
		load = load,
		import = function(module)
			return runtime.import(include.RuntimeLib, module)
		end,
	}
end
