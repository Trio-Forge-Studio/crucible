local harness = require("./lib/ui-harness")
return {
	["modal runtime TopbarPlus handoff before initial readiness pauses later targets"] = function()
		local h = harness()
		local id, root = h.root(10)
		root.AbsoluteSize = { X = 100, Y = -1 }
		h.control(id, "First", { selectFirst = true })
		h.flush()
		local topbar = {}
		h.select(topbar)
		h.control(id, "Late", { selectFirst = true })
		root.AbsoluteSize = { X = 100, Y = 100 }
		h.flush()
		assert(h.gui.SelectedObject == topbar, "readiness and late markers never reclaim an early top-bar handoff")
		h.stop()
	end,
	["modal runtime orders actual display layers and pending attachment blocks all owners"] = function()
		local h = harness()
		local hud = h.control("Base", "Hud")
		assert(hud.Selectable and not h.runtime.modalOpen(), "native HUD selection is play mode")
		local admin = h.root(30, "Pointer")
		local adminButton = h.control(admin, "Admin")
		local screen, root = h.root(10, "Native", nil, true)
		local first = h.control(screen, "First", { selectFirst = true })
		h.flush()
		assert(
			h.runtime.modalOpen() and not hud.Selectable and not first.Selectable,
			"pending root blocks gameplay and selection"
		)
		h.attach(root, 10)
		h.flush()
		assert(
			h.runtime.active(admin) and not first.Selectable and not adminButton.Selectable,
			"Admin stays top despite a later Screen and stays pointer only"
		)
		h.runtime.closeScope(admin)
		h.flush()
		assert(first.Selectable and h.gui.SelectedObject == first, "closing Admin uncovers native Screen")
		h.stop()
	end,
	["modal runtime waits for finite nonnegative ancestor layout and cancels stale requests"] = function()
		local h = harness()
		local id, root = h.root(10)
		root.AbsoluteSize = { X = 100, Y = -1 }
		local first = h.control(id, "First", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == nil, "negative ancestor prevents selection without polling")
		root.AbsoluteSize = { X = 100, Y = 100 }
		h.flush()
		assert(h.gui.SelectedObject == first, "property readiness completes once")
		root.AbsoluteSize = { X = 0 / 0, Y = 100 }
		local late, dispose = h.control(id, "Late", { selectFirst = true })
		h.flush()
		dispose()
		root.AbsoluteSize = { X = 100, Y = 100 }
		h.flush()
		assert(h.gui.SelectedObject ~= late, "a disposed pending target cannot complete")
		h.stop()
	end,
	["modal runtime preserves explicit deselection and TopbarPlus handoff"] = function()
		local h = harness()
		local id = h.root(10)
		local first = h.control(id, "First", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == first, "opening selects the marker")
		local topbar = {}
		h.select(topbar)
		h.control(id, "Late", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == topbar, "new marks do not reclaim TopbarPlus")
		h.select(nil)
		h.device("Keyboard")
		h.device("Gamepad")
		h.flush()
		assert(h.gui.SelectedObject == nil, "device entry preserves intentional nil")
		h.select(first)
		local replacement = h.control(id, "Replacement", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == replacement, "native reentry clears pause for a replacement marker")
		h.stop()
	end,
	["modal runtime restores replaced underlying controls and skips non-top disposal"] = function()
		local h = harness()
		local screen = h.root(10)
		local first, remove = h.control(screen, "Equip", { selectFirst = true })
		h.flush()
		local reveal = h.root(25)
		h.control(reveal, "Continue", { selectFirst = true })
		h.flush()
		local replacement = h.control(screen, "Equip", { selectFirst = true })
		remove()
		local intermediate = h.root(20)
		h.control(intermediate, "Intermediate", { selectFirst = true })
		h.flush()
		h.runtime.closeScope(intermediate)
		h.flush()
		assert(
			h.runtime.active(reveal) and h.gui.SelectedObject ~= first,
			"non-top disposal never moves the foreground"
		)
		h.runtime.closeScope(reveal)
		h.flush()
		assert(h.gui.SelectedObject == replacement, "uncover falls back to the rebuilt action")
		h.stop()
	end,
	["modal runtime restores only a live HUD snapshot on last close"] = function()
		local h = harness()
		local hud = h.control("Base", "Hud")
		h.select(hud)
		local id = h.root(10)
		h.control(id, "First", { selectFirst = true })
		h.flush()
		h.runtime.closeScope(id)
		h.flush()
		assert(h.gui.SelectedObject == hud, "last close restores previous HUD selection")
		h.select(nil)
		local second = h.root(10)
		h.control(second, "First", { selectFirst = true })
		h.flush()
		h.runtime.closeScope(second)
		h.flush()
		assert(h.gui.SelectedObject == nil, "no snapshot means no random HUD selection")
		h.stop()
	end,
	["modal runtime losing Claim falls back to Screen close and late Wear replaces Continue"] = function()
		local h = harness()
		local id = h.root(10)
		local close = h.control(id, "Close", {}, true)
		local claim, remove = h.control(id, "Claim", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == claim, "claim is first")
		remove()
		h.flush()
		assert(h.gui.SelectedObject == close, "claim disposal selects internal close fallback")
		local reveal = h.root(25)
		h.control(reveal, "Continue", { selectFirst = true })
		h.flush()
		local wear = h.control(reveal, "Wear", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == wear, "late Wear takes over the marked Continue")
		h.stop()
	end,
	["modal runtime B requires a fresh release across root changes and closes once"] = function()
		local h = harness()
		local calls = 0
		h.holdB(true)
		local id = h.root(10, "Native", function()
			calls += 1
		end)
		h.flush()
		h.close.Pressed:Fire()
		assert(calls == 0, "held B before open cannot close")
		h.holdB(false)
		h.close.Released:Fire()
		local b = h.begin(h.enum.KeyCode.ButtonB)
		h.close.Pressed:Fire()
		assert(calls == 1, "one physical press dispatches once")
		h.runtime.closeScope(id)
		h.root(10, "Native", function()
			calls += 10
		end)
		h.flush()
		h.close.Pressed:Fire()
		assert(calls == 1, "held B cannot reach a newly uncovered root")
		h.finish(b)
		h.begin(h.enum.KeyCode.ButtonB)
		assert(calls == 11, "fresh B reaches the new root")
		h.stop()
	end,
	["modal runtime any TextBox releases Menu and held B stays barred after text editing"] = function()
		local h = harness()
		local calls = 0
		h.root(10, "Native", function()
			calls += 1
		end)
		h.flush()
		assert(h.contexts.Menu, "native top owns Menu")
		h.editing(true)
		assert(not h.contexts.Menu and h.runtime.textEditing(), "custom TextBox suspends Menu")
		local b = h.begin(h.enum.KeyCode.ButtonB)
		h.editing(false)
		h.close.Pressed:Fire()
		assert(calls == 0, "OSK dismissal B cannot close the screen")
		h.finish(b)
		h.begin(h.enum.KeyCode.ButtonB)
		assert(calls == 1, "fresh B after text editing closes")
		h.stop()
	end,
	["modal runtime captures fresh A for hold and cancels on selection cover menu and disconnect"] = function()
		local h = harness()
		local fired = 0
		local edges = {}
		local heldBeforeOpen = h.begin(h.enum.KeyCode.ButtonA)
		local id = h.root(10)
		local button = h.press(id, "Portrait", {
			selectFirst = true,
			hold = 1,
			onPress = function()
				fired += 1
			end,
			onPressedChange = function(value)
				table.insert(edges, value)
			end,
		})
		h.flush()
		h.tick(2)
		assert(fired == 0 and #edges == 0, "A held before mount starts no session")
		h.finish(heldBeforeOpen)
		local a = h.begin(h.enum.KeyCode.ButtonA)
		h.tick(0.6)
		h.finish(a)
		h.tick(0.5)
		assert(fired == 0 and #edges == 2 and edges[2] == false, "early release cancels once")
		a = h.begin(h.enum.KeyCode.ButtonA)
		h.tick(1)
		h.tick(1)
		h.finish(a)
		button.Activated:Fire()
		assert(fired == 1, "hold fires once and ignores release Activated")
		a = h.begin(h.enum.KeyCode.ButtonA)
		h.select(nil)
		h.tick(2)
		h.finish(a)
		assert(fired == 1 and edges[#edges] == false, "selection loss cancels captured A")
		h.select(button)
		a = h.begin(h.enum.KeyCode.ButtonA)
		h.root(25)
		h.tick(2)
		h.finish(a)
		assert(fired == 1 and edges[#edges] == false, "cover cancels captured A")
		h.stop()
	end,
	["modal runtime covered Activated is dropped for Base and modal controls including pointer"] = function()
		local h = harness()
		local fired = 0
		local hud = h.press("Base", "ToastOpen", {
			onPress = function()
				fired += 1
			end,
		})
		local id = h.root(10)
		local button = h.press(id, "Accept", {
			onPress = function()
				fired += 10
			end,
		})
		h.flush()
		hud.Activated:Fire()
		button.Activated:Fire()
		assert(fired == 10, "Base toast action is covered")
		h.root(25)
		h.flush()
		button.Activated:Fire()
		assert(fired == 10, "stale pointer activation cannot cross a modal")
		h.stop()
	end,
	["modal runtime repeat and hold stop on disable menu editing and controller disconnect"] = function()
		local h = harness()
		local id = h.root(10)
		local disabled = h.vide.source(false)
		local fired = 0
		local ends = 0
		local button = h.press(id, "Increase", {
			repeating = true,
			selectFirst = true,
			disabled = disabled,
			onPress = function()
				fired += 1
			end,
			onPressedChange = function(down)
				if not down then
					ends += 1
				end
			end,
		})
		h.flush()
		local a = h.begin(h.enum.KeyCode.ButtonA)
		h.tick(0.4)
		assert(fired == 2, "repeat has immediate step and delay")
		disabled(true)
		h.tick(1)
		assert(ends == 1 and fired == 2 and button.Selectable, "disabled cancels once but stays selectable")
		h.finish(a)
		disabled(false)
		a = h.begin(h.enum.KeyCode.ButtonA)
		h.gui.MenuIsOpen = true
		h.tick(1)
		assert(ends == 2, "Roblox menu cancels the session")
		h.finish(a)
		h.gui.MenuIsOpen = false
		a = h.begin(h.enum.KeyCode.ButtonA)
		h.editing(true)
		h.tick(1)
		assert(ends == 3, "typing cancels the session")
		h.finish(a)
		h.editing(false)
		h.begin(h.enum.KeyCode.ButtonA)
		h.input.GamepadDisconnected:Fire(h.enum.UserInputType.Gamepad1)
		h.tick(1)
		assert(ends == 4, "disconnect cancels the captured controller")
		h.stop()
	end,
	["modal runtime disposed attachment cannot resurrect and a pending-only close restores HUD"] = function()
		local h = harness()
		local hud = h.control("Base", "Hud")
		h.select(hud)
		local id, root = h.root(10, "Native", nil, true)
		h.select(nil)
		h.runtime.closeScope(id)
		h.attach(root, 10)
		h.flush()
		assert(
			not h.runtime.modalOpen() and h.gui.SelectedObject == hud,
			"late ancestry cannot resurrect a disposed pending root"
		)
		h.stop()
	end,
	["modal runtime resumes initial selection after Roblox menu or text editing without reclaim"] = function()
		local h = harness()
		h.gui.MenuIsOpen = true
		local id = h.root(10)
		local first = h.control(id, "First", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == nil, "Roblox menu suspends initial selection")
		h.gui.MenuIsOpen = false
		h.flush()
		assert(h.gui.SelectedObject == first, "closing Roblox menu completes initial selection")
		h.select(nil)
		h.editing(true)
		h.editing(false)
		h.flush()
		assert(h.gui.SelectedObject == nil, "resumption never reclaims intentional deselection")
		h.stop()
	end,
	["modal runtime a timing change cannot turn hold release into ordinary activation"] = function()
		local h = harness()
		local id = h.root(10)
		local hold = h.vide.source(1)
		local fired = 0
		local button = h.press(id, "Dynamic", {
			selectFirst = true,
			hold = hold,
			onPress = function()
				fired += 1
			end,
		})
		h.flush()
		local a = h.begin(h.enum.KeyCode.ButtonA)
		h.tick(0.2)
		hold(nil)
		h.finish(a)
		button.Activated:Fire()
		assert(fired == 0, "hold release stays suppressed after a reactive timing change")
		a = h.begin(h.enum.KeyCode.ButtonA)
		h.finish(a)
		button.Activated:Fire()
		assert(fired == 1, "a fresh ordinary press can activate")
		h.stop()
	end,
	["modal runtime press feedback names focus, press and refusal"] = function()
		local h = harness()
		local id = h.root(10)
		local refuse = false
		local disabled = h.vide.source(false)
		local buy = h.press(id, "Buy", {
			selectFirst = true,
			disabled = disabled,
			onPress = function()
				return if refuse then false else nil
			end,
		})
		h.flush()
		assert(table.concat(h.feedback, ",") == "Focus", "selection highlight gives focus feedback")
		table.clear(h.feedback)
		buy.Activated:Fire()
		refuse = true
		buy.Activated:Fire()
		disabled(true)
		buy.Activated:Fire()
		assert(
			table.concat(h.feedback, ",") == "Press,Refused,Refused",
			"activation gives press feedback, a false result or disabled gives refused"
		)
		local hold = h.press(id, "Hold", {
			hold = 1,
			disabled = h.vide.source(true),
			onPress = function() end,
		})
		h.select(hold)
		table.clear(h.feedback)
		h.finish(h.begin(h.enum.KeyCode.ButtonA))
		assert(table.concat(h.feedback, ",") == "Refused", "a disabled hold refuses on begin")
		local quiet = h.press(id, "Quiet", { silent = true, onPress = function() end })
		table.clear(h.feedback)
		h.select(quiet)
		quiet.Activated:Fire()
		assert(#h.feedback == 0, "silent presses give no feedback")
		h.stop()
	end,
	["modal runtime a physical B release re-arms close without the action release"] = function()
		local h = harness()
		local calls = 0
		h.holdB(true)
		h.root(10, "Native", function()
			calls += 1
		end)
		h.flush()
		h.holdB(false)
		h.input.InputEnded:Fire({ KeyCode = h.enum.KeyCode.ButtonB, UserInputType = h.enum.UserInputType.Gamepad1 })
		h.close.Pressed:Fire()
		assert(calls == 1, "a raw ButtonB release re-arms close when the action misses it")
		h.stop()
	end,
}
