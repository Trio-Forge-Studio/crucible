local harness = require("./lib/ui-harness")
local roblox = require("@lune/roblox")

return {
	["Lucide icon rectangles follow the sheet grid"] = function()
		local h = harness()
		local ui = h.require()
		local atlas = h.require("lucide")
		assert(ui.lucideVersion == atlas.lucideVersion, "the UI entry reports the bundled Lucide version")
		for _, case in
			{
				{ index = 1, sheet = 1, y = 2 },
				{ index = 17, sheet = 1, y = 66 },
				{ index = 257, sheet = 2, y = 2 },
			}
		do
			local image = ui.icon(atlas.lucideNames[case.index])
			assert(image.Image == atlas.lucideSheets[case.sheet], "each icon uses its sheet asset ID")
			assert(string.match(image.Image, "^rbxassetid://%d+$"), "the sheet is a Roblox asset URI")
			assert(image.ImageRectOffset == roblox.Vector2.new(2, case.y), "offset skips the transparent gutter")
			assert(image.ImageRectSize == roblox.Vector2.new(60, 60), "all icon rectangles are 60×60")
		end
		h.stop()
	end,
	["an unknown Lucide icon reports its name"] = function()
		local h = harness()
		local ok, message = pcall(h.require().icon, "missing-icon-name")
		assert(not ok and string.find(tostring(message), "missing-icon-name", 1, true), "the error names the icon")
		h.stop()
	end,
}
