local harness = require("@self/load")
harness.engine = require("@self/engine")
harness.Backend = require("@self/data-backend")
harness.networkPort = require("@self/network-port")
harness.loadPlace = require("@self/place")

function harness.loadGame(options)
	assert(type(options) == "table" and type(options.place) == "string", "loadGame needs { place = <rbxl path> }")
	return harness.loadPlace(options.client, options.place, options)
end

function harness.profileFixture(realm, store, options)
	options = options or {}
	local meta = if type(store) == "table" then getmetatable(store) else nil
	local declaration = if type(meta) == "table" then meta.__crucibleProfileStore else nil
	assert(declaration, "profileFixture needs a compiled profile store")
	local published = {}
	local reasons = {}
	local profile = realm.data:profile(declaration.schema, {
		onLoad = function(player, value, reason)
			table.insert(reasons, reason)
			return if options.onLoad then options.onLoad(player, value, reason) else value
		end,
		onRelease = options.onRelease or function(_, value)
			return value
		end,
		publish = function(player, value)
			published[player] = value
		end,
		delivered = function() end,
	})
	profile.published = published
	profile.reasons = reasons
	return profile
end

return harness
