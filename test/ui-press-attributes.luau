local harness = require("./lib/ui-harness")
return {
	["attribute names are exported and frozen"] = function()
		local names = require("./lib/ui-load")()("attributes").Attribute
		for _, name in { "Highlighted", "Pressed", "Disabled", "Selected", "Editing", "Invalid", "Notice" } do
			assert(names[name] == name, "export matches instance attribute " .. name)
		end
		assert(table.isfrozen(names), "attribute contract cannot be mutated")
	end,
	["press attributes start false and follow hover press and disabled"] = function()
		local h = harness()
		h.device("Keyboard")
		local disabled = h.vide.source(false)
		local calls = 0
		local button = h.press("Base", "Press", {
			disabled = disabled,
			onPress = function()
				calls += 1
			end,
		})
		for _, name in { "Highlighted", "Pressed", "Disabled" } do
			assert(button:GetAttribute(name) == false, name .. " starts false")
		end
		button.MouseEnter:Fire()
		assert(button:GetAttribute("Highlighted") == true, "hover highlights")
		local pointer = { UserInputType = h.enum.UserInputType.MouseButton1 }
		button.InputBegan:Fire(pointer)
		assert(button:GetAttribute("Pressed") == true, "pointer presses")
		h.input.InputEnded:Fire(pointer)
		assert(button:GetAttribute("Pressed") == false, "release clears pressed")
		button.Activated:Fire()
		assert(calls == 1, "activation reaches the game")
		button.InputBegan:Fire(pointer)
		disabled(true)
		assert(button:GetAttribute("Disabled") == true, "disabled reacts")
		assert(button:GetAttribute("Pressed") == false, "disabled cancels pressed")
		button.Activated:Fire()
		assert(calls == 1, "disabled blocks activation")
		button.MouseLeave:Fire()
		assert(button:GetAttribute("Highlighted") == false, "leave clears hover")
		disabled(false)
		assert(button:GetAttribute("Disabled") == false, "reenabled reacts")
		h.stop()
	end,
}
