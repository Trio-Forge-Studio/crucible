local harness = require("./lib/ui-harness")
local roblox: any = require("@lune/roblox")
local function setup()
	local h = harness()
	h.toggle = h.require("toggle")
	h.Slider = h.require("slider").Slider
	h.TextField = h.require("text-field").TextField
	return h
end
local function expectOutside(h, parts, root: string, part: string)
	local ok, message = pcall(function()
		h.mount("Base", function()
			return parts[part]({})
		end)
	end)
	local expected = `{root}.{part} must be used inside {root}.Root`
	assert(not ok and string.find(message, expected, 1, true), `{root}.{part}: {message}`)
end
return {
	["control parts outside their root raise a named error"] = function()
		local h = setup()
		expectOutside(h, h.toggle.Checkbox, "Checkbox", "Indicator")
		expectOutside(h, h.toggle.Switch, "Switch", "Thumb")
		for _, part in { "Track", "Range", "Thumb" } do
			expectOutside(h, h.Slider, "Slider", part)
		end
		expectOutside(h, h.TextField, "TextField", "Input")
		h.stop()
	end,
	["checkbox and switch roots toggle and their indicator and thumb follow checked"] = function()
		local h = setup()
		local reported = {}
		local box, indicator, toggle, thumb
		h.mount("Base", function()
			box = h.toggle.Checkbox.Root({
				Name = "Music",
				onCheckedChange = function(checked)
					table.insert(reported, checked)
				end,
				children = function()
					indicator = h.toggle.Checkbox.Indicator({ Name = "Check" })
					return indicator
				end,
			})
			toggle = h.toggle.Switch.Root({
				defaultChecked = true,
				children = function()
					thumb = h.toggle.Switch.Thumb({})
					return thumb
				end,
			})
		end)
		assert(
			box.Name == "Music" and indicator.Parent == box and thumb.Parent == toggle,
			"parts nest in the root host"
		)
		assert(not indicator.Visible and indicator:GetAttribute("Checked") == false, "an unchecked indicator hides")
		box.Activated:Fire()
		assert(reported[1] == true and box:GetAttribute("Checked") == true, "pressing the root checks it")
		assert(indicator.Visible and indicator:GetAttribute("Checked") == true, "the indicator follows")
		assert(thumb:GetAttribute("Checked") == true, "the thumb starts at defaultChecked")
		toggle.Activated:Fire()
		assert(thumb:GetAttribute("Checked") == false, "the thumb follows its switch")
		h.stop()
	end,
	["an asChild checkbox root takes the child from its children function"] = function()
		local h = setup()
		local child = h.instance("ImageButton", "Base")
		local binding
		h.mount("Base", function()
			return h.toggle.Checkbox.Root({
				asChild = true,
				children = function(toggle)
					binding = toggle
					return child
				end,
			})
		end)
		child.Activated:Fire()
		assert(binding.checked() and child:GetAttribute("Checked") == true, "the child toggles")
		h.stop()
	end,
	["slider range and thumb take their geometry from the value and the track takes drags"] = function()
		local h = setup()
		h.device("Keyboard")
		local track, range, thumb, binding
		h.mount("Base", function()
			return h.Slider.Root({
				defaultValue = 2,
				max = 10,
				children = function(slider)
					binding = slider
					range = h.Slider.Range({})
					thumb = h.Slider.Thumb({})
					track = h.Slider.Track({ children = { range, thumb } })
					return track
				end,
			})
		end)
		assert(range.Size == roblox.UDim2.new(0.2, 0, 1, 0), "the range spans the fraction")
		assert(thumb.Position == roblox.UDim2.fromScale(0.2, 0.5), "the thumb sits at the fraction")
		assert(thumb.AnchorPoint == roblox.Vector2.new(0.5, 0.5), "the thumb centres on the fraction by default")
		track.AbsoluteSize = roblox.Vector2.new(200, 20)
		local mouse = { UserInputType = h.enum.UserInputType.MouseButton1, Position = roblox.Vector3.new(150, 0, 0) }
		track.InputBegan:Fire(mouse)
		h.input.InputEnded:Fire(mouse)
		assert(binding.value() == 8 and range.Size == roblox.UDim2.new(0.8, 0, 1, 0), "a track press moves the value")
		assert(thumb.Position == roblox.UDim2.fromScale(0.8, 0.5), "the thumb follows")
		h.stop()
	end,
	["text field input shows the value, reports typing and the root captures focus"] = function()
		local h = setup()
		local changes = {}
		local root, input
		h.mount("Base", function()
			root = h.TextField.Root({
				defaultValue = "Ana",
				onValueChange = function(value)
					table.insert(changes, value)
				end,
				children = function()
					input = h.TextField.Input({ PlaceholderText = "Name" })
					return input
				end,
			})
		end)
		assert(input.Text == "Ana" and input.PlaceholderText == "Name", "the input shows the value")
		root.Activated:Fire()
		assert(input:IsFocused() and root:GetAttribute("Editing") == true, "pressing the root edits the input")
		input.Text = "Anabel"
		assert(changes[1] == "Anabel", "typing reports the value")
		h.stop()
	end,
}
