local harness = require("./lib/ui-harness")
local function setup()
	local h = harness()
	h.Tabs = h.require("tabs").Tabs
	return h
end
local function render(h, rootProps, triggers)
	local Tabs = h.Tabs
	local rendered = { triggers = {}, contents = {}, mounted = {} }
	rootProps.children = function(tabs)
		rendered.tabs = tabs
		local list = {}
		for index, props in triggers do
			rendered.triggers[index] = Tabs.Trigger(props)
			table.insert(list, rendered.triggers[index])
		end
		local contents = {}
		for _, props in triggers do
			table.insert(
				contents,
				Tabs.Content({
					value = props.value,
					children = function()
						rendered.mounted[props.value] = (rendered.mounted[props.value] or 0) + 1
						h.vide.cleanup(function()
							rendered.mounted[props.value] -= 1
						end)
						local frame = h.vide.jsx("frame", { Name = props.value })
						rendered.contents[props.value] = frame
						return frame
					end,
				})
			)
		end
		return { Tabs.List({ children = list }), contents }
	end
	local _, dispose = h.mount("Base", function()
		return Tabs.Root(rootProps)
	end)
	h.flush()
	return rendered, dispose
end
return {
	["tab parts outside Tabs.Root raise a named error"] = function()
		local h = setup()
		for _, part in { "List", "Trigger", "Content" } do
			local ok, message = pcall(function()
				h.mount("Base", function()
					return h.Tabs[part]({ value = "a", children = function() end })
				end)
			end)
			assert(
				not ok and string.find(message, `Tabs.{part} must be used inside Tabs.Root`, 1, true),
				`Tabs.{part}: {message}`
			)
		end
		h.stop()
	end,
	["triggers switch the mounted content and render neutral passthrough hosts"] = function()
		local h = setup()
		local reported = {}
		local rendered = render(h, {
			defaultValue = "general",
			onValueChange = function(value)
				table.insert(reported, value)
			end,
		}, { { value = "general", Name = "General", LayoutOrder = 2 }, { value = "audio" } })
		local general, audio = rendered.triggers[1], rendered.triggers[2]
		assert(general.Name == "General" and general.LayoutOrder == 2, "host props pass through")
		assert(general.Text == "" and general.AutoButtonColor == false, "Roblox defaults are neutralised")
		assert(rendered.mounted.general == 1 and not rendered.mounted.audio, "only the active content mounts")
		assert(general:GetAttribute("Selected") == true and audio:GetAttribute("Selected") == false, "Selected")
		audio.Activated:Fire()
		assert(reported[1] == "audio" and rendered.tabs.value() == "audio", "pressing a trigger selects its value")
		assert(rendered.mounted.general == 0 and rendered.mounted.audio == 1, "content follows the value")
		h.stop()
	end,
	["arrow links move along the list and skip disabled triggers"] = function()
		for _, orientation in { "Horizontal", "Vertical" } do
			local h = setup()
			local rendered = render(h, { defaultValue = "a", orientation = orientation }, {
				{ value = "a" },
				{ value = "b", disabled = true },
				{ value = "c" },
			})
			local a = rendered.triggers[1]
			h.select(a)
			local forward = if orientation == "Horizontal" then "NextSelectionRight" else "NextSelectionDown"
			local backward = if orientation == "Horizontal" then "NextSelectionLeft" else "NextSelectionUp"
			assert(a[forward] == rendered.triggers[3], `{orientation}: movement skips the disabled trigger`)
			assert(a[backward] == rendered.triggers[3], `{orientation}: tabs loop by default`)
			h.stop()
		end
	end,
	["manual tabs activate only on press and automatic tabs activate on selection unless disabled"] = function()
		for _, mode in { "Manual", "Automatic" } do
			local h = setup()
			local reported = {}
			local rendered = render(h, {
				defaultValue = "a",
				activationMode = if mode == "Manual" then nil else mode,
				onValueChange = function(value)
					table.insert(reported, value)
				end,
			}, { { value = "a" }, { value = "b", disabled = true }, { value = "c" } })
			local b, c = rendered.triggers[2], rendered.triggers[3]
			h.select(c)
			if mode == "Manual" then
				assert(rendered.tabs.value() == "a" and #reported == 0, "selection passing through never activates")
				assert(rendered.mounted.a == 1 and not rendered.mounted.c, "the page stays put")
				c.Activated:Fire()
				assert(rendered.tabs.value() == "c" and reported[1] == "c", "a press activates")
			else
				assert(rendered.tabs.value() == "c" and reported[1] == "c", "selecting a trigger activates it")
				h.select(b)
				assert(rendered.tabs.value() == "c", "a disabled trigger never activates")
			end
			h.stop()
		end
	end,
	["a trigger's function children read its binding with or without asChild"] = function()
		local h = setup()
		local labels, bindings = {}, {}
		local child
		h.mount("Base", function()
			return h.Tabs.Root({
				defaultValue = "a",
				children = function()
					return {
						h.Tabs.Trigger({
							value = "a",
							children = function(trigger)
								bindings.a = trigger
								labels.a = h.vide.jsx("textlabel", { Name = "Label" })
								return labels.a
							end,
						}),
						h.Tabs.Trigger({
							value = "b",
							asChild = true,
							children = function(trigger)
								bindings.b = trigger
								child = h.vide.jsx("textbutton", { Name = "B" })
								return child
							end,
						}),
						h.Tabs.Trigger({ value = "c", children = h.vide.jsx("textlabel", { Name = "Plain" }) }),
					}
				end,
			})
		end)
		h.flush()
		local a = labels.a.Parent
		assert(a and a.ClassName == "TextButton", "a function child renders inside the trigger")
		assert(bindings.a.selected() and not bindings.b.selected(), "the binding reports which trigger is active")
		h.select(child)
		assert(bindings.b.highlighted() and not bindings.a.highlighted(), "the asChild binding follows its child")
		child.Activated:Fire()
		assert(bindings.b.selected() and not bindings.a.selected(), "pressing the asChild child activates it")
		h.stop()
	end,
	["an asChild trigger keeps the child's handlers, adopts its modifiers and disconnects on unmount"] = function()
		local h = setup()
		local child = h.instance("TextButton", "Base")
		local corner = h.instance("UICorner", "Base")
		local own, requested = 0, 0
		child.Activated:Connect(function()
			own += 1
		end)
		local present = h.vide.source(true)
		h.mount("Base", function()
			return h.Tabs.Root({
				value = "other",
				onValueChange = function()
					requested += 1
				end,
				children = function()
					return h.vide.show(present, function()
						return h.Tabs.Trigger({ value = "shop", asChild = true, children = { child, corner } })
					end)
				end,
			})
		end)
		assert(corner.Parent == child, "the modifier moves under the child")
		assert(child:GetAttribute("Selected") == false, "attributes land on the child")
		child.Activated:Fire()
		assert(own == 1 and requested == 1, "the child's handler and the part's both run")
		present(false)
		child.Activated:Fire()
		assert(own == 2 and requested == 1, "after unmount only the child's own handler runs")
		h.stop()
	end,
}
