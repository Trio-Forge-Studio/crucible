local harness = require("@self/load")
harness.engine = require("@self/engine")
harness.Backend = require("@self/data-backend")
harness.networkPort = require("@self/network-port")
harness.loadPlace = require("@self/place")

function harness.loadGame(options)
	assert(
		type(options) == "table" and (type(options.place) == "string" or type(options.sourcemap) == "string"),
		"loadGame needs a place or sourcemap path"
	)
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
		onLoad = function(session, value, reason)
			table.insert(reasons, reason)
			return if options.onLoad then options.onLoad(session.player, value, reason) else value
		end,
		onRelease = function(session, value, reason)
			return if options.onRelease then options.onRelease(session.player, value, reason) else value
		end,
		publish = function(player, value)
			published[player] = value
		end,
		delivered = function() end,
		observe = function() end,
	})
	local fixture = { published = published, reasons = reasons }
	for _, name in { "load", "write", "save", "writable", "players", "leave", "handoff", "resume", "await" } do
		fixture[name] = function(_, ...)
			return profile[name](profile, ...)
		end
	end
	function fixture:release(player, reason)
		local session = profile.sessions[player]
		if session then
			profile:release(session, reason or "leave")
		end
	end
	function fixture:state(player)
		local session = profile.sessions[player]
		return if session then ({ "loading", "active", "sealed", "ended" })[session.authority.phase + 1] else "absent"
	end
	return fixture
end

return harness
