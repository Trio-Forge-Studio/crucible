-- The same UI as crucible-ui.luau, written the Vide way: `create` with property tables, sources passed as reactive
-- props, `show`, and `values` for the keyed list. `cleanup` destroys each mount's root Instance. Vide updates
-- synchronously at the write, so `flush` does nothing.
return function(vide)
	local create, source, root, cleanup, show, values =
		vide.create, vide.source, vide.root, vide.cleanup, vide.show, vide.values
	local SIZE = table.freeze({ 1, 0, 0, 24 })

	local function Item(text, color)
		return create("TextLabel")({
			Name = "Item",
			Size = SIZE,
			BackgroundTransparency = 1,
			Text = text,
			TextColor3 = color,
		})
	end

	local function Row(item, index)
		return create("TextLabel")({
			Name = "Row",
			Size = SIZE,
			BackgroundTransparency = 1,
			Text = item.label,
			LayoutOrder = index,
		})
	end

	local ui = {}

	function ui.mountItems(texts, colors)
		local frame
		local dispose = root(function()
			local items = table.create(#texts)
			for i = 1, #texts do
				items[i] = Item(texts[i], colors[i])
			end
			frame = create("Frame")({ Name = "Root", items })
			cleanup(frame)
		end)
		return frame, dispose
	end

	function ui.mountList(list)
		local frame
		local dispose = root(function()
			frame = create("Frame")({ Name = "List", values(list, Row) })
			cleanup(frame)
		end)
		return frame, dispose
	end

	function ui.mountShow(open, text, color)
		local frame
		local dispose = root(function()
			frame = create("Frame")({
				Name = "Panel",
				show(open, function()
					return Item(text, color)
				end),
			})
			cleanup(frame)
		end)
		return frame, dispose
	end

	function ui.state(value)
		return source(value)
	end

	function ui.set(s, value)
		s(value)
	end

	function ui.flush() end

	return ui
end
