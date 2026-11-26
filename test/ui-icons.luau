local harness = require("./lib/ui-harness")
local roblox = require("@lune/roblox")
local World = require("./lib/sample-world")
local ui = require("./lib/ui-instances")

return {
	["the compiler puts the locked image and crop in the sample menu"] = function()
		local world = World.new()
		world:bootServer()
		world:settle()
		local client = world:connect("Ann", 4120)
		world:settle()
		world:press(client, roblox.Enum.KeyCode.M)
		local screen = assert(client.player.PlayerGui:FindFirstChild("SettingsMenu"), "M mounts the menu")
		ui.fire(assert(screen:FindFirstChild("Help", true), "the help button mounts"), "Activated")
		world.link:flush()
		local icon = assert(screen:FindFirstChild("CoinIcon", true), "help mounts the coin icon")
		assert(icon.Image == "rbxassetid://1", "the compiled icon uses the sample's locked asset")
		assert(icon.ImageRectOffset == roblox.Vector2.new(2, 2), "the crop starts inside the gutter")
		assert(icon.ImageRectSize == roblox.Vector2.new(60, 60), "the crop covers the icon cell")
		world.place:Destroy()
	end,
	["the runtime icon stub explains that compilation is required"] = function()
		local h = harness()
		local ok, message = pcall(h.require().icon, "coins")
		assert(
			not ok and string.find(tostring(message), "must be compiled by Crucible", 1, true),
			"the stub names the compiler requirement"
		)
		h.stop()
	end,
}
