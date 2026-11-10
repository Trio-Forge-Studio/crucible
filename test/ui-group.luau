local harness = require("./lib/ui-harness")
local function setup(options)
	local h = harness()
	local group = h.require("group").createGroup(options)
	local items, disabled = {}, {}
	h.mount("Base", function()
		for index = 1, 4 do
			local object = h.instance("TextButton", "Base")
			object.Name = `Item{index}`
			disabled[index] = h.vide.source(false)
			h.runtime.useSelection({}).action(object)
			group.item(object, disabled[index])
			items[index] = object
		end
	end)
	return h, items, disabled
end
return {
	["arrow links follow declaration order and skip disabled hidden and unselectable items"] = function()
		local h, items, disabled = setup()
		h.select(items[1])
		assert(items[1].NextSelectionLeft == items[1], "the first item clamps without loop")
		assert(items[1].NextSelectionRight == items[2], "the next item follows declaration order")
		disabled[2](true)
		assert(items[1].NextSelectionRight == items[3], "a disabled item is skipped as soon as it disables")
		items[3].Visible = false
		items[4].Selectable = false
		h.select(items[2])
		h.select(items[1])
		assert(items[1].NextSelectionRight == items[1], "hidden and unselectable items are skipped")
		items[3].Visible = true
		h.select(items[3])
		assert(
			items[3].NextSelectionLeft == items[1] and items[3].NextSelectionRight == items[3],
			"links skip backwards too"
		)
		assert(items[3].NextSelectionUp == nil, "the cross axis stays with Roblox navigation")
		h.stop()
	end,
	["changing orientation moves an item's links to the new axis and clears the ones it wrote"] = function()
		local h = harness()
		local orientation = h.vide.source("Horizontal")
		local group = h.require("group").createGroup({ orientation = orientation })
		local items = {}
		h.mount("Base", function()
			for index = 1, 3 do
				local object = h.instance("TextButton", "Base")
				h.runtime.useSelection({}).action(object)
				group.item(object)
				items[index] = object
			end
		end)
		h.select(items[2])
		h.select(items[1])
		orientation("Vertical")
		assert(
			items[1].NextSelectionUp == items[1] and items[1].NextSelectionDown == items[2],
			"the selected item links along the new axis"
		)
		assert(
			items[1].NextSelectionLeft == nil and items[1].NextSelectionRight == nil,
			"and no longer along the old one"
		)
		h.select(items[2])
		assert(
			items[2].NextSelectionUp == items[1] and items[2].NextSelectionDown == items[3],
			"an item selected before the change links vertically when it is selected again"
		)
		assert(
			items[2].NextSelectionLeft == nil and items[2].NextSelectionRight == nil,
			"and drops its old horizontal links"
		)
		orientation("Horizontal")
		assert(
			items[2].NextSelectionLeft == items[1] and items[2].NextSelectionRight == items[3],
			"back to horizontal links left and right"
		)
		assert(items[2].NextSelectionUp == nil and items[2].NextSelectionDown == nil, "and clears up and down")
		local elsewhere = h.instance("TextButton", "Base")
		items[2].NextSelectionRight = elsewhere
		orientation("Vertical")
		assert(items[2].NextSelectionRight == elsewhere, "a link the game set instead is not the group's to clear")
		assert(items[2].NextSelectionLeft == nil, "the group's own link is")
		h.stop()
	end,
	["loop wraps at both ends"] = function()
		local h, items = setup({ loop = true })
		h.select(items[4])
		assert(items[4].NextSelectionRight == items[1], "the last item wraps to the first")
		h.select(items[1])
		assert(items[1].NextSelectionLeft == items[4], "the first item wraps to the last")
		h.stop()
	end,
	["vertical groups link up and down and an unmounted item leaves the order"] = function()
		local h = harness()
		local group = h.require("group").createGroup({ orientation = "Vertical" })
		local objects, removers = {}, {}
		for index = 1, 3 do
			local object = h.instance("TextButton", "Base")
			local _, remove = h.mount("Base", function()
				h.runtime.useSelection({}).action(object)
				group.item(object)
			end)
			objects[index], removers[index] = object, remove
		end
		h.select(objects[1])
		assert(objects[1].NextSelectionDown == objects[2] and objects[1].NextSelectionRight == nil, "vertical axis")
		removers[2]()
		assert(objects[1].NextSelectionDown == objects[3], "removal relinks the selected item")
		h.stop()
	end,
}
