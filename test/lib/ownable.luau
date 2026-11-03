-- Fake engine values for `own`. Lune has no RBXScriptConnection, and its typeof ignores a proxy's `__type`, so a
-- runtime loaded with `ownable.globals` sees these tables as Roblox's typeof names the real values.
local CONNECTION = {}
local INSTANCE = {}

local ownable = {}

function ownable.typeof(value: any): string
	local meta = getmetatable(value)
	if meta == CONNECTION then
		return "RBXScriptConnection"
	elseif meta == INSTANCE then
		return "Instance"
	end
	return typeof(value)
end

function ownable.connection(disconnect: () -> ())
	return setmetatable({ Disconnect = disconnect }, CONNECTION)
end

-- The fake engine's `describe` reads `Name`.
function ownable.instance(name: string, destroy: () -> ())
	return setmetatable({ Name = name, Destroy = destroy }, INSTANCE)
end

ownable.globals = { typeof = ownable.typeof }

return ownable
