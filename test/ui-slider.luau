local harness = require("./lib/ui-harness")
local roblox: any = require("@lune/roblox")
local Enum = roblox.Enum
local function slider(h, given)
	local control = h.instance("TextButton", "Base")
	local track = h.instance("Frame", "Base")
	track.Parent = control
	track.AbsolutePosition = roblox.Vector2.new(100, 0)
	track.AbsoluteSize = roblox.Vector2.new(200, 20)
	local binding, dispose = h.mount("Base", function()
		local result = h.require("use-slider").useSlider(given)
		result.control(control)
		result.track(track)
		return result
	end)
	return control, track, binding, dispose
end
local function pointer(kind, x)
	return { UserInputType = Enum.UserInputType[kind], Position = roblox.Vector3.new(x, 0, 0) }
end
return {
	["slider drags with the mouse and commits the final value once on release"] = function()
		local h = harness()
		h.device("Keyboard")
		local changes, commits = {}, {}
		local control, _, binding = slider(h, {
			max = 10,
			onValueChange = function(value)
				table.insert(changes, value)
			end,
			onValueCommit = function(value)
				table.insert(commits, value)
			end,
		})
		local mouse = pointer("MouseButton1", 160)
		control.InputBegan:Fire(mouse)
		assert(binding.value() == 3 and changes[1] == 3, "a press jumps to the snapped pointer value")
		assert(binding.dragging() and control:GetAttribute("Dragging") == true, "a press starts a drag")
		assert(control:GetAttribute("Highlighted") == true, "dragging highlights")
		h.input.InputChanged:Fire(pointer("MouseMovement", 400))
		assert(binding.value() == 10 and binding.fraction() == 1, "moving past the track clamps to max")
		assert(#commits == 0, "nothing commits mid-drag")
		h.input.InputEnded:Fire(mouse)
		assert(#changes == 2 and #commits == 1 and commits[1] == 10, "release commits the final value once")
		assert(not binding.dragging() and control:GetAttribute("Dragging") == false, "release ends the drag")
		h.input.InputEnded:Fire(mouse)
		assert(#commits == 1, "a second release does nothing")
		local still = pointer("MouseButton1", 300)
		control.InputBegan:Fire(still)
		h.input.InputEnded:Fire(still)
		assert(#changes == 2 and #commits == 1, "a drag that changes nothing commits nothing")
		local lost = pointer("MouseButton1", 100)
		control.InputBegan:Fire(lost)
		h.input.WindowFocusReleased:Fire()
		assert(not binding.dragging() and commits[2] == 0, "window focus loss ends and commits a drag")
		h.stop()
	end,
	["slider follows only the touch that started the drag"] = function()
		local h = harness()
		local commits = {}
		local _, track, binding, dispose = slider(h, {
			max = 4,
			onValueCommit = function(value)
				table.insert(commits, value)
			end,
		})
		local touch = pointer("Touch", 100)
		track.InputBegan:Fire(touch)
		assert(binding.value() == 0 and binding.dragging(), "a touch on the track starts a drag")
		h.input.InputChanged:Fire(pointer("Touch", 300))
		assert(binding.value() == 0, "another touch is ignored")
		touch.Position = roblox.Vector3.new(250, 0, 0)
		h.input.InputChanged:Fire(touch)
		assert(binding.value() == 3, "the dragging touch moves the value")
		h.input.InputEnded:Fire(touch)
		assert(commits[1] == 3 and #commits == 1, "lifting the touch commits")
		local again = pointer("Touch", 100)
		track.InputBegan:Fire(again)
		dispose()
		assert(commits[2] == 0, "unmounting mid-drag commits")
		h.input.InputEnded:Fire(again)
		assert(#commits == 2, "the lifted touch after unmount does not commit again")
		h.stop()
	end,
	["selected slider steps and accelerates on D-pad, arrow keys and thumbstick holds"] = function()
		local h = harness()
		local value = h.vide.source(5)
		local commits = {}
		local control = slider(h, {
			value = value,
			max = 10,
			onValueChange = value,
			onValueCommit = function(next)
				table.insert(commits, next)
			end,
		})
		assert(control.NextSelectionLeft == control and control.NextSelectionRight == control, "navigation is pinned")
		h.finish(h.begin(Enum.KeyCode.DPadRight))
		assert(value() == 5, "an unselected slider ignores the D-pad")
		h.select(control)
		local right = h.begin(Enum.KeyCode.DPadRight)
		assert(value() == 6, "the D-pad steps at once")
		h.tick(0.39)
		assert(value() == 6, "the hold waits before repeating")
		h.tick(0.01)
		assert(value() == 7, "the hold repeats after 0.4 s")
		h.tick(0.149)
		assert(value() == 7, "the first repeat interval is 0.15 s")
		h.tick(0.001)
		assert(value() == 8 and #commits == 0, "the hold keeps repeating without committing")
		h.finish(right)
		h.tick(1)
		assert(value() == 8 and #commits == 1 and commits[1] == 8, "release stops the hold and commits once")
		local left = { KeyCode = Enum.KeyCode.Left, UserInputType = Enum.UserInputType.Keyboard }
		h.input.InputBegan:Fire(left)
		h.input.InputEnded:Fire(left)
		assert(value() == 7 and commits[2] == 7, "the Left arrow behaves like the D-pad")
		local function stick(x, y)
			h.input.InputChanged:Fire({
				KeyCode = Enum.KeyCode.Thumbstick1,
				UserInputType = Enum.UserInputType.Gamepad1,
				Position = roblox.Vector3.new(x, y, 0),
			})
		end
		stick(0.8, 0)
		stick(0.9, 0.1)
		assert(value() == 8, "a horizontal push steps once per direction")
		stick(0.3, 0.2)
		assert(commits[3] == 8, "centering the stick ends and commits the hold")
		stick(0.5, 0.9)
		assert(value() == 8, "a mostly vertical push does nothing")
		stick(-0.7, 0)
		stick(0, 0)
		assert(value() == 7 and commits[4] == 7, "pushing left steps down")
		value(10)
		h.finish(h.begin(Enum.KeyCode.DPadRight))
		assert(value() == 10 and #commits == 4, "a hold that cannot move commits nothing")
		h.select(nil)
		h.input.GetGamepadState = function()
			return { { KeyCode = Enum.KeyCode.Thumbstick1, Position = roblox.Vector3.new(-0.9, 0, 0) } }
		end
		h.select(control)
		stick(-0.9, 0)
		assert(value() == 10, "a stick already pushed when selection arrives waits for centering")
		stick(0, 0)
		stick(-0.9, 0)
		assert(value() == 9, "the stick re-arms after centering")
		h.stop()
	end,
	["a hold ends and commits once on focus loss, disconnect, blocked states, disabling and unmount"] = function()
		local h = harness()
		local disabled = h.vide.source(false)
		local commits = {}
		local control, _, binding, dispose = slider(h, {
			defaultValue = 50,
			max = 100,
			disabled = disabled,
			onValueCommit = function(value)
				table.insert(commits, value)
			end,
		})
		h.select(control)
		local function hold(cancel, label)
			local before = #commits
			local input = h.begin(Enum.KeyCode.DPadRight)
			local held = binding.value()
			cancel()
			h.tick(1)
			assert(#commits == before + 1 and commits[before + 1] == held and binding.value() == held, label)
			h.finish(input)
			assert(#commits == before + 1, label .. " exactly once")
		end
		hold(function()
			h.input.WindowFocusReleased:Fire()
		end, "window focus loss ends the hold")
		hold(function()
			h.input.GamepadDisconnected:Fire(Enum.UserInputType.Gamepad1)
		end, "a gamepad disconnect ends the hold")
		hold(function()
			h.editing(true)
			h.editing(false)
		end, "text editing ends the hold")
		hold(function()
			h.gui.MenuIsOpen = true
			h.gui.MenuIsOpen = false
		end, "the Roblox menu ends the hold")
		hold(function()
			disabled(true)
			disabled(false)
		end, "disabling ends the hold")
		hold(function()
			h.runtime.closeScope(h.root(10))
			h.flush()
		end, "a modal over the slider ends the hold")
		hold(dispose, "unmounting ends the hold")
		h.stop()
	end,
	["slider ends a gesture that its first change disables, and leaves no listener behind"] = function()
		local h = harness()
		local listeners = 0
		local function count(signal)
			local connect = signal.Connect
			signal.Connect = function(self, run)
				listeners += 1
				local connection = connect(self, run)
				local disconnect = connection.Disconnect
				local connected = true
				connection.Disconnect = function()
					if connected then
						listeners -= 1
					end
					connected = false
					disconnect()
				end
				return connection
			end
		end
		count(h.input.InputChanged)
		count(h.input.InputEnded)
		local disabled = h.vide.source(false)
		local changes = 0
		local control, _, binding, dispose = slider(h, {
			max = 10,
			disabled = disabled,
			onValueChange = function()
				changes += 1
				disabled(true)
			end,
		})
		local mouse = pointer("MouseButton1", 160)
		control.InputBegan:Fire(mouse)
		assert(binding.value() == 3 and changes == 1, "the press made its first change")
		assert(not binding.dragging(), "the change disabled the slider, which ended the drag")
		h.input.InputChanged:Fire(pointer("MouseMovement", 300))
		assert(binding.value() == 3 and changes == 1, "the ended drag does not follow the pointer")
		h.input.InputEnded:Fire(mouse)
		dispose()
		assert(listeners == 0, "the ended drag left no listener")

		local gamepadDisabled = h.vide.source(false)
		local held = 0
		local pad, _, padBinding, padDispose = slider(h, {
			max = 10,
			disabled = gamepadDisabled,
			onValueChange = function()
				held += 1
				gamepadDisabled(true)
			end,
		})
		h.select(pad)
		local right = h.begin(Enum.KeyCode.DPadRight)
		assert(padBinding.value() == 1 and held == 1, "the hold made its first step")
		h.tick(0.5)
		h.tick(0.5)
		assert(padBinding.value() == 1 and held == 1, "the ended hold does not repeat")
		h.finish(right)
		padDispose()
		h.tick(0.5)
		assert(padBinding.value() == 1 and held == 1, "nor after release and unmount")
		h.stop()
	end,
	["slider snaps values, writes attributes and refuses while disabled"] = function()
		local h = harness()
		local disabled = h.vide.source(false)
		local changes = 0
		local control, _, binding = slider(h, {
			value = 2.6,
			max = 4,
			disabled = disabled,
			onValueChange = function()
				changes += 1
			end,
		})
		assert(binding.value() == 3 and binding.fraction() == 0.75, "controlled values snap to the step")
		for _, name in { "Highlighted", "Dragging", "Disabled" } do
			assert(control:GetAttribute(name) == false, name .. " starts false")
		end
		h.select(control)
		assert(control:GetAttribute("Highlighted") == true and h.feedback[1] == "Focus", "selection highlights")
		disabled(true)
		assert(control:GetAttribute("Disabled") == true, "disabled reaches styling")
		h.finish(h.begin(Enum.KeyCode.DPadRight))
		control.InputBegan:Fire(pointer("MouseButton1", 300))
		assert(changes == 0 and not binding.dragging(), "a disabled slider ignores gamepad and pointer input")
		assert(h.feedback[2] == "Refused" and h.feedback[3] == "Refused", "each refused gesture gives feedback")
		local _, _, fixed = slider(h, { value = 20, min = 4, max = 4, silent = true })
		assert(fixed.value() == 4 and fixed.fraction() == 0, "a fixed range is normalized")
		h.stop()
	end,
}
