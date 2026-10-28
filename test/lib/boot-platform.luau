local spec = require("./load")

return function(client)
	local p = {
		services = {},
		events = {},
		required = {},
		errors = {},
		warnings = {},
		tasks = {},
		attributes = {},
		bound = {},
		closed = {},
	}
	local function signal(name)
		return {
			Connect = function(_, callback)
				p.events[name] = callback
				return {
					Disconnect = function()
						p.events[name] = nil
					end,
				}
			end,
		}
	end
	function p.node(name, kind, children)
		local n = { Name = name, ClassName = kind or "Folder", children = children or {} }
		function n:GetChildren()
			return self.children
		end
		function n:IsA(class)
			return self.ClassName == class
		end
		function n:GetFullName()
			return (if self.Parent then self.Parent:GetFullName() .. "." else "") .. self.Name
		end
		for _, child in n.children do
			child.Parent = n
		end
		return n
	end
	for _, name in
		{ "ServerStorage", "ReplicatedStorage", "ServerScriptService", "StarterGui", "StarterPack", "StarterPlayer" }
	do
		p.services[name] = p.node(name)
	end
	p.services.CollectionService = {
		GetTagged = function()
			return {}
		end,
		GetInstanceAddedSignal = function(_, tag)
			return signal("added " .. tag)
		end,
		GetInstanceRemovedSignal = function(_, tag)
			return signal("removed " .. tag)
		end,
	}
	local run = {
		IsClient = function()
			return client == true
		end,
		IsServer = function()
			return client ~= true
		end,
	}
	for _, name in { "PreRender", "PreSimulation", "PostSimulation", "RenderStepped", "PreAnimation", "Heartbeat" } do
		run[name] = signal(name)
	end
	function run:BindToRenderStep(name, priority, callback)
		p.bound[name] = { priority = priority, callback = callback }
	end
	function run:UnbindFromRenderStep(name)
		p.bound[name] = nil
	end
	p.services.RunService = run
	p.services.Players =
		{ LocalPlayer = {
			SetAttribute = function(_, name, value)
				p.attributes[name] = value
			end,
		} }
	p.game = {
		GetService = function(_, name)
			return assert(p.services[name], name)
		end,
		BindToClose = function(_, callback)
			table.insert(p.closed, callback)
		end,
	}
	p.task = {
		spawn = function(callback)
			local ok, err = pcall(callback)
			if not ok then
				table.insert(p.errors, err)
			end
		end,
		defer = function(callback)
			table.insert(p.tasks, callback)
		end,
		cancel = function(thread)
			p.cancelled = thread
		end,
		wait = function()
			p.waited = true
		end,
	}
	p.graph = spec.runtime({
		game = p.game,
		task = p.task,
		warn = function(message)
			table.insert(p.warnings, message)
		end,
		require = function(module)
			table.insert(p.required, module:GetFullName())
			if p.onRequire then
				p.onRequire(module)
			end
			return {}
		end,
	})
	return p
end
