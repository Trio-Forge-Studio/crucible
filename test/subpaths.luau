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
	["no side entry loads Vide, and the vide subpath resolves it"] = function()
		local function videLoaded(h): boolean
			local vide = h.place:GetService("ReplicatedStorage").rbxts_include.node_modules["@rbxts"].vide
			for module in h.loaded do
				if module == vide or module:IsDescendantOf(vide) then
					return true
				end
			end
			return false
		end
		local server = loadPlace()
		server.import(server.crucible)
		server.import(server.crucible.server)
		server.import(server.crucible.emit)
		assert(not videLoaded(server), "the root, server and emit entries never load Vide")
		local client = loadPlace(true)
		client.import(client.crucible)
		client.import(client.crucible.client)
		assert(not videLoaded(client), "the client entry never loads Vide")
		assert(type(client.import(client.crucible.vide).useReadable) == "function", "the vide subpath resolves")
		assert(videLoaded(client), "the vide subpath loads Vide")
		client.place:Destroy()
		server.place:Destroy()
	end,
	["no side entry loads the random subpath, and it resolves alone"] = function()
		local function loadedUnder(h, folder): { string }
			local names = {}
			for module in h.loaded do
				if module:IsDescendantOf(folder) then
					table.insert(names, module.Name)
				end
			end
			table.sort(names)
			return names
		end
		local function subpathLoaded(h): boolean
			return #loadedUnder(h, h.crucible.util) > 0
		end
		local server = loadPlace()
		server.import(server.crucible)
		server.import(server.crucible.server)
		server.import(server.crucible.emit)
		assert(not subpathLoaded(server), "the root, server and emit entries never load it")
		local client = loadPlace(true)
		client.import(client.crucible)
		client.import(client.crucible.client)
		client.import(client.crucible.vide)
		assert(not subpathLoaded(client), "the client and vide entries never load it")
		client.place:Destroy()
		server.place:Destroy()

		local random = loadPlace()
		local Rng = random.import(random.crucible.util.random).Rng
		assert(Rng.new(42):uint32() == 1768527656, "the random subpath resolves and draws its recorded stream")
		assert(random.loaded[random.crucible.server] == nil, "random does not load the server")
		random.place:Destroy()
	end,
}
