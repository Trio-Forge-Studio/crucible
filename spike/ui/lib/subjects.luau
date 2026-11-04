-- One fresh UI per variant, each on its own fresh runtime.
local runtime = require("./runtime")
local CrucibleUI = require("./crucible-ui")
local VideUI = require("./vide-ui")

local subjects = {}

subjects.names = { "Crucible A", "Crucible B", "Vide" }

function subjects.make(name: string)
	if name == "Vide" then
		return VideUI(runtime.vide())
	end
	local r = runtime.crucible()
	local ui = CrucibleUI(r, if name == "Crucible A" then "A" else "B")
	ui.realm = r
	return ui
end

return subjects
