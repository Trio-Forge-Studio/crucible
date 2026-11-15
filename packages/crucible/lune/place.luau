local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local engine = require("./engine")
local networkPort = require("./network-port")

roblox.implementMethod("Instance", "WaitForChild", function(instance, name: string)
	return assert(instance:FindFirstChild(name), `{instance:GetFullName()} has no child {name}`)
end)

return function(client: boolean?, path: string?, options)
	options = options or {}
	local place = roblox.deserializePlace(fs.readFile(path or "build/sample-game.rbxl"))
	local e = options.engine or engine.new()
	local messages = {}
	local closers = {}
	local attributes = {}
	e.port.bindToClose = e.port.bindToClose or function(callback)
		table.insert(closers, callback)
	end
	e.port.spawn = e.port.spawn or function(callback)
		callback()
	end
	e.port.clock = e.port.clock or os.clock
	e.port.wait = e.port.wait or task.wait
	local runService = {
		Heartbeat = {
			Connect = function()
				return { Disconnect = function() end }
			end,
		},
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
	local reflectionService = {
		GetClasses = function()
			return {}
		end,
	}
	local game = {
		GetService = function(_self, name: string)
			if options.services and options.services[name] ~= nil then
				return options.services[name]
			end
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
			if name == "ReflectionService" then
				return reflectionService
			end
			return if name == "RunService" then runService else place:GetService(name)
		end,
	}
	local realm = {}
	local loaded = {}
	local load
	local function requireFrom(script)
		return function(target)
			if type(target) == "string" then
				local child = string.match(target, "^@self/(.+)$")
				if child then
					target = assert(script:FindFirstChild(child), `{script:GetFullName()} has no child {child}`)
				elseif string.match(target, "^%.%./") then
					local name = string.sub(target, 4)
					target = assert(script.Parent.Parent:FindFirstChild(name), `missing parent module {name}`)
				else
					local name = assert(string.match(target, "^%./(.+)$"), `unsupported require {target}`)
					target = assert(script.Parent:FindFirstChild(name), `{script:GetFullName()} has no sibling {name}`)
				end
			end
			return load(target)
		end
	end
	function load(module)
		if module.Name == "roblox-port" and module.Parent.Name == "internal" then
			return {
				robloxPort = function()
					return e.port
				end,
			}
		end
		if loaded[module] == nil then
			local environment = {
				_G = realm,
				game = game,
				script = module,
				require = requireFrom(module),
				task = options.task or task,
			}
			for key, value in roblox do
				environment[key] = value
			end
			for key, value in options.globals or {} do
				environment[key] = value
			end
			if environment.print == nil then
				environment.print = function(...)
					local values = {}
					for _, value in { ... } do
						table.insert(values, tostring(value))
					end
					table.insert(messages, table.concat(values, " "))
				end
			end
			local chunk = luau.load(module.Source, {
				debugName = module:GetFullName(),
				environment = environment,
			})
			loaded[module] = { value = chunk() }
			if module.Name == "log-output" and module.Parent.Name == "internal" then
				loaded[module].value.installLoggingPort(e.loggingPort)
			end
			if module.Name == "port" and module.Parent.Name == "network" then
				loaded[module].value.installNetworkPort(options.network or networkPort())
			end
			if module.Name == "port" and module.Parent.Name == "data" and options.dataPort then
				loaded[module].value.installDataPort(options.dataPort)
			end
		end
		return loaded[module].value
	end
	local include = place:GetService("ReplicatedStorage").rbxts_include
	local runtime = load(include.RuntimeLib)
	local crucible = include.node_modules["@trioforge"].crucible
	crucible = crucible:FindFirstChild("out") or crucible

	return {
		place = place,
		crucible = crucible,
		loaded = loaded,
		engine = e,
		messages = messages,
		closers = closers,
		attributes = attributes,
		runService = runService,
		load = load,
		import = function(module)
			return runtime.import(include.RuntimeLib, module)
		end,
	}
end
