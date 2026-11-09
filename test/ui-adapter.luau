local load = require("./lib/ui-load")
return {
	["installAnvil keeps one adapter and refuses a missing or different one"] = function()
		local Adapter = load()("adapter")
		assert(not pcall(Adapter.adapter), "reading before install fails")
		local first = {}
		Adapter.installAnvil(first)
		Adapter.installAnvil(first)
		assert(Adapter.adapter() == first, "installing the same adapter again keeps it")
		assert(not pcall(Adapter.installAnvil, {}), "a different adapter is refused")
		assert(Adapter.adapter() == first, "a refused adapter leaves the first in place")
	end,
}
