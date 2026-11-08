local harness = require("./lib/ui-harness")
local function icon(h, given)
	local button = h.instance("TextButton", "Topbar")
	local binding, dispose = h.mount("Base", function()
		local result = h.require("use-topbar-icon").useTopbarIcon(given)
		result.action(button)
		return result
	end)
	return button, binding, dispose
end
return {
	["top bar icon attributes follow hover selected disabled and notice"] = function()
		local h = harness()
		h.device("Keyboard")
		local selected = h.vide.source(false)
		local disabled = h.vide.source(false)
		local notice = h.vide.source(false :: any)
		local calls = 0
		local button = icon(h, {
			selected = selected,
			disabled = disabled,
			notice = notice,
			onSelectedChange = function()
				calls += 1
			end,
		})
		for _, name in { "Highlighted", "Pressed", "Disabled", "Selected" } do
			assert(button:GetAttribute(name) == false, name .. " starts false")
		end
		assert(button:GetAttribute("Notice") == 0, "notice starts at zero")
		button.MouseEnter:Fire()
		assert(button:GetAttribute("Highlighted") == true, "hover highlights")
		selected(true)
		assert(button:GetAttribute("Selected") == true, "controlled selection reaches styling")
		notice(true)
		assert(button:GetAttribute("Notice") == 1, "a boolean notice counts as one")
		notice(120)
		assert(button:GetAttribute("Notice") == 120, "a numeric notice keeps its count")
		disabled(true)
		button.Activated:Fire()
		assert(button:GetAttribute("Disabled") == true and calls == 0, "disabled icon refuses")
		h.stop()
	end,
	["top bar icons play no feedback, like Roblox's own top bar"] = function()
		local h = harness()
		h.device("Keyboard")
		local button = icon(h, {})
		button.MouseEnter:Fire()
		button.Activated:Fire()
		assert(#h.feedback == 0, "hover and press stay silent")
		h.stop()
	end,
	["top bar icon presses while a modal is open and refuses while the Roblox menu is open"] = function()
		local h = harness()
		local calls, hud = 0, 0
		local button = icon(h, {
			selected = false,
			onSelectedChange = function()
				calls += 1
			end,
		})
		local base = h.press("Base", "Hud", {
			onPress = function()
				hud += 1
			end,
		})
		assert(button.Selectable, "play mode lets the gamepad reach the icon")
		local screen = h.root(10)
		local inside = h.control(screen, "Inside", { selectFirst = true })
		h.flush()
		base.Activated:Fire()
		button.Activated:Fire()
		assert(hud == 0 and calls == 1, "the icon presses above a modal that blocks the HUD")
		assert(not button.Selectable and h.gui.SelectedObject == inside, "the modal keeps gamepad selection")
		h.gui.MenuIsOpen = true
		button.Activated:Fire()
		assert(calls == 1, "the Roblox menu blocks the icon")
		h.gui.MenuIsOpen = false
		h.runtime.closeScope(screen)
		button.Activated:Fire()
		assert(calls == 2 and button.Selectable, "the icon presses again in play mode")
		h.stop()
	end,
	["a controlled top bar icon reports the opposite of its selection and leaves it to the game"] = function()
		local h = harness()
		local selected = h.vide.source(false)
		local reported = {}
		local button, binding = icon(h, {
			selected = selected,
			onSelectedChange = function(next)
				table.insert(reported, next)
			end,
		})
		button.Activated:Fire()
		assert(reported[1] == true, "pressing an unselected icon reports true")
		assert(not binding.selected() and button:GetAttribute("Selected") == false, "the press leaves selection alone")
		selected(true)
		button.Activated:Fire()
		assert(reported[2] == false, "pressing a selected icon reports false")
		assert(binding.selected() and button:GetAttribute("Selected") == true, "the game decides selection")
		h.stop()
	end,
	["an uncontrolled top bar icon flips its own selection from defaultSelected"] = function()
		local h = harness()
		local reported = {}
		local button, binding = icon(h, {
			defaultSelected = true,
			onSelectedChange = function(next)
				table.insert(reported, next)
			end,
		})
		assert(binding.selected() and button:GetAttribute("Selected") == true, "starts at defaultSelected")
		button.Activated:Fire()
		assert(not binding.selected() and reported[1] == false, "a press deselects and reports")
		h.stop()
	end,
	["DPadUp selects the remembered or leftmost available icon only from play mode"] = function()
		local h = harness()
		local disabled = h.vide.source(false)
		local function placed(x, given)
			local button = icon(h, given or {})
			button.AbsolutePosition = { X = x, Y = 0 }
			return button
		end
		local daily = placed(100)
		placed(50, { disabled = true })
		local settings = placed(200, { disabled = disabled })
		placed(10).Visible = false
		local function dpadUp()
			h.finish(h.begin(h.enum.KeyCode.DPadUp))
			h.flush()
		end
		dpadUp()
		assert(h.gui.SelectedObject == daily, "skips disabled and hidden icons for the leftmost")
		h.select(settings)
		h.select(nil)
		dpadUp()
		assert(h.gui.SelectedObject == settings, "returns to the last selected icon")
		h.select(nil)
		disabled(true)
		dpadUp()
		assert(h.gui.SelectedObject == daily, "a disabled remembered icon falls back")
		local hud = h.control("Base", "Hud")
		h.select(hud)
		dpadUp()
		assert(h.gui.SelectedObject == hud, "an existing selection stays")
		h.select(nil)
		local screen = h.root(10, "Pointer")
		dpadUp()
		assert(h.gui.SelectedObject == nil, "an open modal blocks top bar focus")
		h.runtime.closeScope(screen)
		h.gui.MenuIsOpen = true
		dpadUp()
		assert(h.gui.SelectedObject == nil, "the Roblox menu blocks top bar focus")
		h.gui.MenuIsOpen = false
		h.device("Keyboard")
		dpadUp()
		assert(h.gui.SelectedObject == nil, "only a gamepad focuses the top bar")
		h.device("Gamepad")
		dpadUp()
		assert(h.gui.SelectedObject == daily, "play mode focuses again")
		h.stop()
	end,
	["top bar caption shows after half a second of mouse hover and hides when selected"] = function()
		local h = harness()
		h.device("Keyboard")
		local selected = h.vide.source(false)
		local button, binding = icon(h, { selected = selected })
		button.MouseEnter:Fire()
		h.tick(0.4)
		assert(not binding.captionShown(), "the caption waits for the hover delay")
		h.tick(0.1)
		assert(binding.captionShown(), "the caption shows after half a second")
		selected(true)
		assert(not binding.captionShown(), "a selected icon hides its caption")
		selected(false)
		button.MouseLeave:Fire()
		assert(not binding.captionShown(), "leaving hides the caption")
		button.MouseEnter:Fire()
		h.tick(0.3)
		assert(not binding.captionShown(), "a new hover restarts the delay")
		button.MouseLeave:Fire()
		h.device("Touch")
		button.MouseEnter:Fire()
		h.tick(1)
		assert(not binding.captionShown(), "touch never shows the caption")
		h.device("Gamepad")
		h.select(button)
		h.flush()
		h.tick(1)
		assert(not binding.captionShown(), "gamepad selection never shows the caption")
		h.stop()
	end,
}
