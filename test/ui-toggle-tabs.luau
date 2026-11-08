local harness = require("./lib/ui-harness")
return {
	["controlled toggle reports the next state only on an enabled press"] = function()
		local h = harness()
		local checked = h.vide.source(false)
		local disabled = h.vide.source(false)
		local calls = 0
		local button = h.instance("TextButton", "Base")
		local binding = h.mount("Base", function()
			local toggle = h.require("use-toggle").useToggle({
				checked = checked,
				disabled = disabled,
				silent = true,
				onCheckedChange = function(value)
					calls += 1
					checked(value)
				end,
			})
			toggle.action(button)
			return toggle
		end)
		assert(not binding.checked() and button:GetAttribute("Checked") == false, "unchecked starts explicit")
		button.Activated:Fire()
		assert(binding.checked() and button:GetAttribute("Checked") == true and calls == 1, "press checks")
		checked(false)
		assert(button:GetAttribute("Checked") == false, "external controlled changes update styling")
		disabled(true)
		button.Activated:Fire()
		assert(calls == 1 and button:GetAttribute("Disabled") == true, "disabled toggle refuses")
		assert(#h.feedback == 0, "silent suppresses feedback")
		h.stop()
	end,
	["uncontrolled toggle owns its state from defaultChecked"] = function()
		local h = harness()
		local reported = {}
		local button = h.instance("TextButton", "Base")
		local binding = h.mount("Base", function()
			local toggle = h.require("use-toggle").useToggle({
				defaultChecked = true,
				onCheckedChange = function(value)
					table.insert(reported, value)
				end,
			})
			toggle.action(button)
			return toggle
		end)
		assert(binding.checked() and button:GetAttribute("Checked") == true, "starts at defaultChecked")
		button.Activated:Fire()
		button.Activated:Fire()
		assert(binding.checked() and reported[1] == false and reported[2] == true, "each press flips and reports")
		h.stop()
	end,
	["tab triggers key on values select first and skip reselection"] = function()
		local h = harness()
		local owner = h.root(10)
		local value = h.vide.source("general")
		local disabled = h.vide.source(false)
		local calls = 0
		local tabs = h.require("use-tabs").useTabs({
			value = value,
			onValueChange = function(next)
				calls += 1
				value(next)
			end,
		})
		local first = h.instance("TextButton", owner)
		local second = h.instance("TextButton", owner)
		local firstTab = h.mount(owner, function()
			local tab = tabs.trigger("general", { selectFirst = true })
			tab.action(first)
			return tab
		end)
		local secondTab, remove = h.mount(owner, function()
			local tab = tabs.trigger("audio", { disabled = disabled })
			tab.action(second)
			return tab
		end)
		h.flush()
		assert(h.gui.SelectedObject == first, "trigger scope inherits modal selection")
		assert(first:GetAttribute("Selected") == true and second:GetAttribute("Selected") == false, "initial selection")
		first.Activated:Fire()
		assert(calls == 0, "pressing the selected trigger does not report")
		second.Activated:Fire()
		assert(calls == 1 and value() == "audio" and not firstTab.selected() and secondTab.selected(), "other selects")
		assert(
			first:GetAttribute("Selected") == false and second:GetAttribute("Selected") == true,
			"selection styles react"
		)
		disabled(true)
		value("general")
		second.Activated:Fire()
		assert(calls == 1 and second:GetAttribute("Disabled") == true, "disabled trigger blocks selection")
		remove()
		second.Activated:Fire()
		assert(calls == 1, "unmounted trigger disconnects")
		h.stop()
	end,
	["uncontrolled tabs start empty or at defaultValue and a disabled root blocks every trigger"] = function()
		local h = harness()
		local disabled = h.vide.source(false)
		local reported = {}
		local button = h.instance("TextButton", "Base")
		local tabs = h.mount("Base", function()
			local created = h.require("use-tabs").useTabs({
				disabled = disabled,
				onValueChange = function(next)
					table.insert(reported, next)
				end,
			})
			created.trigger(2).action(button)
			return created
		end)
		assert(tabs.value() == nil and button:GetAttribute("Selected") == false, "no default selects nothing")
		disabled(true)
		button.Activated:Fire()
		assert(#reported == 0 and button:GetAttribute("Disabled") == true, "a disabled root disables triggers")
		disabled(false)
		button.Activated:Fire()
		assert(tabs.value() == 2 and reported[1] == 2 and button:GetAttribute("Selected") == true, "press selects")
		local preset = h.mount("Base", function()
			return h.require("use-tabs").useTabs({ defaultValue = 3 })
		end)
		assert(preset.value() == 3, "defaultValue seeds the value")
		h.stop()
	end,
}
