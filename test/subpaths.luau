local loadPlace = require("./lib/place")

return {
	["real root exports stay shared safe and side subpaths resolve"] = function()
		local h = loadPlace()
		local root = h.import(h.crucible)
		assert(root.Subsystem and root.CrucibleBehaviour and root.attribute and root.inject, "real root exports")
		assert(root.sharedPlaceholder == nil, "placeholder removed")
		assert(h.loaded[h.crucible.server] == nil, "root never loads server")
		assert(h.loaded[h.crucible.client] == nil, "root never loads client")
		assert(type(h.import(h.crucible.server).boot) == "function", "server boot resolves")
		assert(h.loaded[h.crucible.server] ~= nil, "loader observes a server import")
		local client = loadPlace(true)
		assert(type(client.import(client.crucible.client).boot) == "function", "client boot resolves")
		assert(client.import(client.crucible.client).BOOTED_ATTRIBUTE == "CrucibleBooted", "boot flag resolves")
		client.place:Destroy()
		h.place:Destroy()
	end,
}
