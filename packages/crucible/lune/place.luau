local fs = require("@lune/fs")
local luau = require("@lune/luau")
local roblox = require("@lune/roblox")
local task = require("@lune/task")
local serde = require("@lune/serde")
local process = require("@lune/process")
local engine = require("./engine")
local networkPort = require("./network-port")

roblox.implementMethod("Instance", "WaitForChild", function(instance, name: string)
	return assert(instance:FindFirstChild(name), `{instance:GetFullName()} has no child {name}`)
end)

local function resolve(root, file)
	root = string.gsub(root, "\\", "/")
	file = string.gsub(file, "\\", "/")
	if string.sub(file, 1, 1) ~= "/" and not string.match(file, "^%a:/") then
		file = root .. "/" .. file
	end
	local prefix = string.match(file, "^%a:") or ""
	local parts = {}
	for part in string.gmatch(string.sub(file, #prefix + 1), "[^/]+") do
		if part == ".." then
			table.remove(parts)
		elseif part ~= "." then
			table.insert(parts, part)
		end
	end
	return prefix .. "/" .. table.concat(parts, "/")
end

return function(client: boolean?, path: string?, options)
	options = options or {}
	local place
	if options.sourcemap then
		local source = serde.decode("json", fs.readFile(options.sourcemap))
		place = roblox.Instance.new("DataModel")
		local root = resolve(process.cwd, options.root or ".")
		local sourceRoot = resolve(root, options.sourceRoot or "src")
		local function walk(node, parent)
			local instance = roblox.Instance.new(node.className)
			instance.Name = node.name
			instance.Parent = parent
			for _, file in node.filePaths or {} do
				file = resolve(root, file)
				if string.match(file, "%.lua[u]?$") then
					instance.Source = fs.readFile(file)
				elseif options.output and string.match(file, "%.tsx?$") and not string.match(file, "%.d%.ts$") then
					local prefix = sourceRoot .. "/"
					assert(string.sub(file, 1, #prefix) == prefix, `source {file} is outside {prefix}`)
					local relative = string.gsub(string.sub(file, #prefix + 1), "%.tsx?$", ".luau")
					instance.Source = fs.readFile(options.output .. "/" .. relative)
				end
			end
			for _, child in node.children or {} do
				walk(child, instance)
			end
		end
		for _, child in source.children or {} do
			walk(child, place)
		end
	else
		place = roblox.deserializePlace(fs.readFile(path or "build/sample-game.rbxl"))
	end
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
	local replacements = {}
	local generations = {}
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
		if replacements[module] then
			loaded[module] = replacements[module]
			return replacements[module].value
		end
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
			if module.Name == "lifecycle" and module.Parent.Name == "internal" then
				local lifecycle = loaded[module].value
				local track = lifecycle.track
				lifecycle.track = function(record, instance, ...)
					local state = track(record, instance, ...)
					if instance then
						generations[record] = generations[record] or {}
						generations[record][instance] = state
					end
					return state
				end
			end
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
		provideGame = function(module, value)
			assert(loaded[module] == nil, "replace a module before importing it or its consumers")
			replacements[module] = { value = value }
		end,
		lifecycle = function(class, instance)
			local record = load(crucible.internal.registry).recordOf(class)
			assert(record, "lifecycle needs a registered class from this graph")
			local state = if record.bound then (generations[record] or {})[instance] else record.state
			return if state
				then ({ "unconstructed", "constructed", "awake", "enabled", "disabled", "faulted", "destroyed" })[state.phase + 1]
				else "unconstructed"
		end,
		import = function(module)
			return runtime.import(include.RuntimeLib, module)
		end,
	}
end
