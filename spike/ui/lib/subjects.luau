-- One fresh UI per subject, each on its own fresh runtime. `subjects.roots` names the two built runtime folders.
local runtime = require("./runtime")
local CrucibleUI = require("./crucible-ui")
local VideUI = require("./vide-ui")

local subjects = {}

subjects.names = { "trunk B", "base B", "head B", "Vide" }
subjects.roots = { trunk = "", base = "", head = "" }

function subjects.make(name: string)
	if name == "Vide" then
		return VideUI(runtime.vide())
	end
	local trunk = name == "trunk B"
	local root = if trunk then subjects.roots.trunk elseif name == "base B" then subjects.roots.base else subjects.roots.head
	local r = runtime.crucible({ root = root, patch = trunk })
	local ui = CrucibleUI(r, if trunk then "B" else "H")
	ui.realm = r
	return ui
end

return subjects
