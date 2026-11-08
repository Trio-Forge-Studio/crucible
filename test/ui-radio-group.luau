local harness = require("./lib/ui-harness")
local function setup()
	local h = harness()
	h.RadioGroup = h.require("radio-group").RadioGroup
	return h
end
local function render(h, rootProps, items)
	local RadioGroup = h.RadioGroup
	local rendered = { items = {}, indicators = {} }
	rootProps.children = function(radio)
		rendered.radio = radio
		local list = {}
		for index, props in items do
			props.children = function()
				rendered.indicators[index] = RadioGroup.Indicator({ Name = "Dot" })
				return rendered.indicators[index]
			end
			rendered.items[index] = RadioGroup.Item(props)
			table.insert(list, rendered.items[index])
		end
		return list
	end
	h.mount("Base", function()
		return RadioGroup.Root(rootProps)
	end)
	h.flush()
	return rendered
end
return {
	["radio parts outside their parent raise a named error"] = function()
		local h = setup()
		for _, case in { { "Item", "RadioGroup.Root" }, { "Indicator", "RadioGroup.Item" } } do
			local ok, message = pcall(function()
				h.mount("Base", function()
					return h.RadioGroup[case[1]]({ value = "a" })
				end)
			end)
			assert(
				not ok and string.find(message, `RadioGroup.{case[1]} must be used inside {case[2]}`, 1, true),
				`RadioGroup.{case[1]}: {message}`
			)
		end
		h.stop()
	end,
	["pressing an item checks it and its indicator shows only while checked"] = function()
		local h = setup()
		local reported = {}
		local rendered = render(h, {
			defaultValue = "Low",
			onValueChange = function(value)
				table.insert(reported, value)
			end,
		}, { { value = "Low", Name = "Low", LayoutOrder = 1 }, { value = "High" } })
		local low, high = rendered.items[1], rendered.items[2]
		assert(low.Name == "Low" and low.LayoutOrder == 1 and low.Text == "", "host props pass through")
		assert(low:GetAttribute("Checked") == true and high:GetAttribute("Checked") == false, "items write Checked")
		assert(rendered.indicators[1].Visible and not rendered.indicators[2].Visible, "only the checked dot shows")
		high.Activated:Fire()
		assert(rendered.radio.value() == "High" and reported[1] == "High", "a press checks the item")
		assert(not rendered.indicators[1].Visible and rendered.indicators[2].Visible, "the dot follows the value")
		high.Activated:Fire()
		assert(#reported == 1, "pressing the checked item reports nothing")
		h.stop()
	end,
	["selection passing through never checks an item"] = function()
		local h = setup()
		local rendered = render(h, { defaultValue = "a" }, { { value = "a" }, { value = "b" } })
		h.select(rendered.items[2])
		assert(rendered.radio.value() == "a", "moving onto an item leaves the value alone")
		h.stop()
	end,
	["a controlled group only reports and disabled items never check"] = function()
		local h = setup()
		local value = h.vide.source("a")
		local reported = {}
		local rendered = render(h, {
			value = value,
			onValueChange = function(next)
				table.insert(reported, next)
			end,
		}, { { value = "a" }, { value = "b" }, { value = "c", disabled = true } })
		rendered.items[2].Activated:Fire()
		assert(reported[1] == "b" and rendered.radio.value() == "a", "the owner decides")
		value("b")
		assert(rendered.items[2]:GetAttribute("Checked") == true, "the item follows the owner's value")
		rendered.items[3].Activated:Fire()
		assert(#reported == 1 and rendered.items[3]:GetAttribute("Disabled") == true, "a disabled item is inert")
		h.stop()
	end,
	["arrow links run vertically by default, skip disabled items and stop at the ends"] = function()
		local h = setup()
		local rendered = render(
			h,
			{ defaultValue = "a" },
			{ { value = "a" }, { value = "b", disabled = true }, { value = "c" } }
		)
		local a, c = rendered.items[1], rendered.items[3]
		h.select(a)
		assert(a.NextSelectionDown == c, "movement skips the disabled item")
		assert(a.NextSelectionUp == a, "the first item does not wrap")
		h.stop()
		local horizontal = setup()
		local row = render(horizontal, { defaultValue = "a", orientation = "Horizontal", loop = true }, {
			{ value = "a" },
			{ value = "b" },
		})
		horizontal.select(row.items[1])
		assert(row.items[1].NextSelectionLeft == row.items[2], "a horizontal looping row wraps on Left")
		horizontal.stop()
	end,
}
