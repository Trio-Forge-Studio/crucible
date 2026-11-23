local M = require("./lib/ui-load")()("topbar-policy")
local function candidate(object: string, x: number, available: boolean?)
	return { object = object, x = x, available = available ~= false }
end
return {
	["top bar focus picks the leftmost available icon"] = function()
		local candidates = { candidate("settings", 200), candidate("hidden", 0, false), candidate("daily", 100) }
		assert(M.pickTopbarIcon(candidates) == "daily", "leftmost available icon wins")
		assert(M.pickTopbarIcon({ candidate("hidden", 0, false) }) == nil, "no available icon picks nothing")
		assert(M.pickTopbarIcon({}) == nil, "an empty top bar picks nothing")
	end,
	["top bar focus returns to the remembered icon while it stays available"] = function()
		local candidates = { candidate("daily", 100), candidate("settings", 200) }
		assert(M.pickTopbarIcon(candidates, "settings") == "settings", "remembered icon wins over order")
		candidates[2] = candidate("settings", 200, false)
		assert(M.pickTopbarIcon(candidates, "settings") == "daily", "an unavailable remembered icon falls back")
		assert(M.pickTopbarIcon(candidates, "removed") == "daily", "a removed remembered icon falls back")
	end,
	["top bar layout copies TopbarPlus offsets for each inset"] = function()
		local function expect(layout, gap: number, offsetY: number, sizeY: number, label: string)
			assert(layout.gap == gap and layout.offsetY == offsetY and layout.sizeY == sizeY, label)
		end
		expect(M.topbarLayout(58, 58, false), 8, 0, -2, "the current top bar sits 2 px above the inset bottom")
		expect(M.topbarLayout(58, 58, true), 10, -9, -2, "console widens the gap and raises the row")
		expect(M.topbarLayout(36, 36, false), 12, 2, -2, "the legacy top bar uses its own offsets")
		expect(M.topbarLayout(0, 0, false), 12, 15, 50, "a hidden top bar keeps a 50 px row")
		expect(M.topbarLayout(0, 0, true), 10, 4, 50, "a hidden console top bar keeps a 50 px row")
	end,
	["top bar caption shifts inside the icon row and leaves room ones alone"] = function()
		assert(M.captionShift(500, 90, 0, 1000, 12) == 0, "a caption with room stays centred")
		assert(M.captionShift(966, 91, 0, 1000, 12) == -23.5, "a caption past the right edge moves left")
		assert(M.captionShift(30, 80, 0, 1000, 12) == 22, "a caption past the left edge moves right")
		assert(M.captionShift(966, 91, 100, 1100, 12) == 0, "the row's own position counts")
	end,
}
