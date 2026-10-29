local loadPlace = require("./lib/place")

return {
	["the root exports its placeholder without loading either side"] = function()
		local h = loadPlace()
		local root = h.import(h.crucible)
		assert(root.sharedPlaceholder == "crucible/shared", "the shared placeholder resolves")
		assert(h.loaded[h.crucible.server] == nil, "the root must not load the server entry")
		assert(h.loaded[h.crucible.client] == nil, "the root must not load the client entry")
		assert(h.import(h.crucible.server).serverPlaceholder == "crucible/server", "the server subpath resolves")
		local client = loadPlace(true)
		assert(
			client.import(client.crucible.client).clientPlaceholder == "crucible/client",
			"the client subpath resolves"
		)
		assert(h.loaded[h.crucible.server] ~= nil, "the loader observes a server import")
		assert(client.loaded[client.crucible.client] ~= nil, "the loader observes a client import")
		client.place:Destroy()
		h.place:Destroy()
	end,
	["the compiled server script imports the root and server subpath"] = function()
		local h = loadPlace()
		local entry = h.place:GetService("ServerScriptService").TS.main
		h.load(entry)
		assert(entry:GetAttribute("CrucibleShared") == "crucible/shared", "the server imports the shared placeholder")
		assert(entry:GetAttribute("CrucibleServer") == "crucible/server", "the server imports its placeholder")
		assert(h.loaded[h.crucible.client] == nil, "the server must not load the client entry")
		h.place:Destroy()
	end,
	["the compiled client script imports the root and client subpath"] = function()
		local h = loadPlace(true)
		local entry = h.place:GetService("StarterPlayer").StarterPlayerScripts.TS.main
		h.load(entry)
		assert(entry:GetAttribute("CrucibleShared") == "crucible/shared", "the client imports the shared placeholder")
		assert(entry:GetAttribute("CrucibleClient") == "crucible/client", "the client imports its placeholder")
		assert(h.loaded[h.crucible.server] == nil, "the client must not load the server entry")
		h.place:Destroy()
	end,
}
