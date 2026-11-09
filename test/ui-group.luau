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
