-- Lune's Instances have no events. Requiring this gives every Instance the signals the ui entry connects, for the
-- whole Lune process, and only a spec fires them.
local roblox = require("@lune/roblox")

type Signal = {
	Connect: (Signal, (...any) -> ()) -> { Disconnect: () -> () },
	Fire: (Signal, ...any) -> (),
}

local function signal(): Signal
	local listeners: { (...any) -> () } = {}
	return {
		Connect = function(_self, listener)
			table.insert(listeners, listener)
			return {
				Disconnect = function()
					local index = table.find(listeners, listener)
					if index then
						table.remove(listeners, index)
					end
				end,
			}
		end,
		Fire = function(_self, ...)
			for _, listener in table.clone(listeners) do
				listener(...)
			end
		end,
	}
end

-- Lune keeps one userdata per Instance, so it can key the Instance's signals.
local signals: { [any]: { [string]: Signal } } = setmetatable({}, { __mode = "k" }) :: any

local function signalOf(instance: any, name: string): Signal
	local own = signals[instance]
	if own == nil then
		own = {}
		signals[instance] = own
	end
	local found = own[name]
	if found == nil then
		found = signal()
		own[name] = found
	end
	return found
end

for class, events in
	{
		GuiButton = { "Activated" },
		GuiObject = { "InputBegan", "MouseEnter", "MouseLeave" },
		Instance = { "AncestryChanged" },
	}
do
	for _, event in events do
		roblox.implementProperty(class, event, function(instance)
			return signalOf(instance, event)
		end)
	end
end
roblox.implementMethod("Instance", "GetPropertyChangedSignal", function(instance, property: string)
	return signalOf(instance, `{property}Changed`)
end)

local instances = {}

function instances.fire(instance: any, event: string, ...: any)
	signalOf(instance, event):Fire(...)
end

-- Roblox fires AncestryChanged on a tree after a mount parents it. Lune never does, so a spec fires it after a mount.
function instances.ancestryChanged(root: any)
	signalOf(root, "AncestryChanged"):Fire()
	for _, descendant in root:GetDescendants() do
		signalOf(descendant, "AncestryChanged"):Fire()
	end
end

return instances
