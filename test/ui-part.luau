local harness = require("./lib/ui-harness")
local function setup()
	local h = harness()
	h.part = h.require("part")
	return h
end
local function expectError(run: () -> (), pattern: string, label: string)
	local ok, message = pcall(run)
	assert(not ok, `{label}: expected an error`)
	assert(string.find(tostring(message), pattern, 1, true), `{label}: unexpected error {message}`)
end
return {
	["a part outside its root names both in the error and inside it reads the root value"] = function()
		local h = setup()
		local context = h.part.partContext("Tabs.Root")
		h.mount("Base", function()
			expectError(function()
				context.use("Tabs.Trigger")
			end, "Tabs.Trigger must be used inside Tabs.Root", "outside the root")
			local found = context.provide({ name = "tabs" }, function()
				return context.use("Tabs.Trigger")
			end)
			assert(found.name == "tabs", "a part inside the root reads its value")
		end)
		h.stop()
	end,
	["asChild resolves one GuiObject and moves UI modifiers under it"] = function()
		local h = setup()
		h.mount("Base", function()
			local child = h.instance("TextButton", "Base")
			local corner = h.instance("UICorner", "Base")
			local padding = h.instance("UIPadding", "Base")
			local applied
			local host = h.part.slot(
				{ corner, { child, padding }, nil },
				"Dialog.Trigger",
				"GuiButton",
				function(object)
					applied = object
				end
			)
			assert(host == child and applied == child, "the single GuiObject takes the behaviour")
			assert(corner.Parent == child and padding.Parent == child, "modifiers move under the child")
		end)
		h.stop()
	end,
	["asChild refuses zero or two GuiObjects, other instances, the wrong class and functions"] = function()
		local h = setup()
		local function slot(children, className)
			h.part.slot(children, "Tabs.Trigger", className or "GuiObject", function() end)
		end
		h.mount("Base", function()
			expectError(function()
				slot({ h.instance("UICorner", "Base") })
			end, "exactly one GuiObject child, got 0", "no GuiObject")
			expectError(function()
				slot({ h.instance("Frame", "Base"), h.instance("TextButton", "Base") })
			end, "exactly one GuiObject child, got 2", "two GuiObjects")
			expectError(function()
				slot({ h.instance("Frame", "Base"), h.instance("Sound", "Base") })
			end, "only moves UI modifiers", "a non-modifier sibling")
			expectError(function()
				slot(h.instance("Frame", "Base"), "GuiButton")
			end, "needs a GuiButton, got a Frame", "the wrong class")
			expectError(function()
				slot(function() end)
			end, "needs instances, not a function", "a reactive child")
		end)
		h.stop()
	end,
	["asChild warns in Studio when a control child already sets a property the part owns"] = function()
		local h = setup()
		h.mount("Base", function()
			local child = h.instance("TextButton", "Base")
			child.NextSelectionLeft = child
			h.part.slot(child, "Tabs.Trigger", "GuiButton", function() end, true)
			h.part.slot(h.instance("TextButton", "Base"), "Tabs.Trigger", "GuiButton", function() end, true)
		end)
		assert(#h.warnings == 1 and string.find(h.warnings[1], "NextSelectionLeft", 1, true), "one warning names it")
		h.stop()
	end,
}
