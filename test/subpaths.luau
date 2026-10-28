local loadPlace = require("./lib/place")
local spec = require("./lib/load")

return {
	["real root exports stay shared safe and side subpaths resolve"] = function()
		local h = loadPlace()
		local root = h.import(h.crucible)
		assert(root.Subsystem and root.CrucibleBehaviour and root.attribute and root.inject, "real root exports")
		assert(root.sharedPlaceholder == nil, "placeholder removed")
		assert(h.loaded[h.crucible.server] == nil, "root never loads server")
		assert(h.loaded[h.crucible.client] == nil, "root never loads client")
		assert(type(h.import(h.crucible.server).boot) == "function", "server boot resolves")
		local client = loadPlace(true)
		assert(type(client.import(client.crucible.client).boot) == "function", "client boot resolves")
		assert(client.import(client.crucible.client).BOOTED_ATTRIBUTE == "CrucibleBooted", "boot flag resolves")
		client.place:Destroy()
		h.place:Destroy()
	end,
	["compiled sample server boots dependencies behaviours and shutdown under the fake engine"] = function()
		local h = loadPlace()
		local member = h.engine.instance("sample", "Part", { Identity = "fleet-23" })
		h.engine.addTag(member, "S2Member")
		local entry = h.place:GetService("ServerScriptService").TS.main
		h.load(entry)
		assert(entry:GetAttribute("CrucibleServer") == true, "server boot returned")
		spec.list(h.messages, {
			"S2 store awake",
			"S2 controller awake",
			"S2 member awake fleet-23 17",
			"S2 member enable",
			"S2 controller start",
			"S2 member start",
			"S2 server booted",
		}, "compiled lifecycle")
		h.engine.removeTag(member, "S2Member")
		h.engine.flush()
		assert(table.find(h.messages, "S2 member disable"), "tag removal disables")
		assert(table.find(h.messages, "S2 member destroy"), "tag removal destroys")
		assert(#h.closers == 1, "one shutdown callback")
		h.closers[1]()
		spec.list(
			{ h.messages[#h.messages - 1], h.messages[#h.messages] },
			{ "S2 controller shutdown", "S2 store shutdown" },
			"dependent shuts down first"
		)
		assert(#h.engine.logs == 0, "no runtime faults")
		assert(h.loaded[h.crucible.client] == nil, "server does not load client")
		h.place:Destroy()
	end,
	["compiled sample client boots and runs the render hook under the fake engine"] = function()
		local h = loadPlace(true)
		local entry = h.place:GetService("StarterPlayer").StarterPlayerScripts.TS.main
		h.load(entry)
		assert(entry:GetAttribute("CrucibleClient") == true, "client boot returned")
		assert(h.attributes.CrucibleBooted == true, "boot sets local player flag")
		h.engine.cycle(1 / 60, true)
		assert(table.find(h.messages, "S2 renderer awake"), "renderer awoke")
		assert(table.find(h.messages, "S2 renderer start"), "renderer started")
		assert(table.find(h.messages, "S2 render 1"), "renderer ran")
		assert(#h.closers == 0, "client does not bind shutdown")
		assert(#h.engine.logs == 0, "no runtime faults")
		assert(h.loaded[h.crucible.server] == nil, "client does not load server")
		h.place:Destroy()
	end,
}
