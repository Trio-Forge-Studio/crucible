local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("./lib/load")

local function declare(r)
	return { PlayerData = fixture.declare(r) }
end

local function newWorld(serve)
	return World.new({ declare = declare, serve = serve })
end

-- A world with one loaded player: the server's Player and the client realm.
local function loaded(serve)
	local w = newWorld(serve)
	local ann = w:player("Ann", 4120)
	w:settle()
	local player = ann.player
	assert(w.server.d.PlayerData:loaded(player), "Ann's profile loaded")
	return w, ann, player
end

return {
	["smoke a player loads, writes a field and the client reads it"] = function()
		local w, ann, player = loaded()
		local data = w.server.d.PlayerData
		spec.eq(data(player).currency(), 0, "currency reads the template's 0 after a fresh load")
		spec.eq(data(player).currency:set(25), true, "set returns true")
		spec.eq(data(player).currency(), 25, "the server reads the write")
		w:settle()
		spec.eq(ann.d.PlayerData().currency(), 25, "the client reads the replicated currency")
		spec.eq(ann.d.PlayerData:loaded(), true, "the client's loaded() is true")
		w:finish()
	end,
}
