-- The keyed list operations, as functions from a list to a new list. Items are compared by identity.
local lists = {}

function lists.items(ui, n: number, from: number?)
	local items = table.create(n)
	local base = from or 0
	for i = 1, n do
		items[i] = { label = ui.state(base + i) }
	end
	return items
end

function lists.reverse(list)
	local out = table.create(#list)
	for i = #list, 1, -1 do
		table.insert(out, list[i])
	end
	return out
end

function lists.insertMiddle(list, extra)
	local half = #list // 2
	local out = table.move(list, 1, half, 1, table.create(#list + #extra))
	table.move(extra, 1, #extra, half + 1, out)
	table.move(list, half + 1, #list, half + #extra + 1, out)
	return out
end

function lists.removeMiddle(list, count: number)
	local start = (#list - count) // 2 + 1
	local out = table.move(list, 1, start - 1, 1, table.create(#list - count))
	table.move(list, start + count, #list, start, out)
	return out
end

function lists.swap(list, a: number, b: number)
	local out = table.clone(list)
	out[a], out[b] = out[b], out[a]
	return out
end

return lists
