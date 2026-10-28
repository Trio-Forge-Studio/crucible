local meter = require("./lib/meter")
local spec = require("./lib/load")

local function newList(items: { any }?)
	local list = spec.fresh("internal/dense-list").DenseList.new()
	for _, item in items or {} do
		list:add(item)
	end
	return list
end

local function walk(list, visit: ((any) -> ())?)
	local seen = {}
	list:forEach(function(item)
		table.insert(seen, item)
		if visit then
			visit(item)
		end
	end)
	return seen
end

return {
	["A removes C and appends NEW during a walk over A, B, C: the walk visits A and B only"] = function()
		local list = newList({ "A", "B", "C" })
		local seen = walk(list, function(item)
			if item == "A" then
				list:remove("C")
				list:add("NEW")
			end
		end)
		spec.list(seen, { "A", "B" }, "this walk")
		spec.list(walk(list), { "A", "B", "NEW" }, "the next walk")
		spec.eq(list:size(), 3, "size")
	end,

	["a nested walk defers compaction to the outermost walk"] = function()
		local list = newList({ "A", "B", "C", "D" })
		local outer = walk(list, function(item)
			if item == "A" then
				local inner = walk(list, function(innerItem)
					if innerItem == "B" then
						list:remove("C")
					end
				end)
				spec.list(inner, { "A", "B", "D" }, "inner walk")
			end
		end)
		spec.list(outer, { "A", "B", "D" }, "outer walk skips the hole")
		list:add("E")
		spec.list(walk(list), { "A", "B", "D", "E" }, "compacted, order kept")
	end,

	["add and remove are idempotent and first skips holes"] = function()
		local list = newList()
		spec.eq(list:add("A"), true, "first add")
		spec.eq(list:add("A"), false, "second add")
		list:add("B")
		spec.eq(list:remove("A"), true, "first remove")
		spec.eq(list:remove("A"), false, "second remove")
		spec.eq(list:first(), "B", "first")
		spec.eq(list:has("A"), false, "has")
	end,

	["act-on 5: a walk whose visit throws rethrows, then removal outside a walk compacts and walks keep order"] = function()
		local list = newList({ "A", "B", "C", "D", "E", "F" })
		local ok, err = pcall(list.forEach, list, function(item)
			if item == "B" then
				error("visit threw")
			end
		end)
		spec.eq(ok, false, "the walk rethrew")
		spec.has(err, "visit threw", "the visit's error")
		for _, item in { "A", "C", "D", "E" } do
			list:remove(item)
		end
		local m = meter(list)
		spec.list(walk(list), { "B", "F" }, "order after the failed walk")
		spec.eq(m.reads <= 2 * list:size(), true, `a walk examined {m.reads} slots for {list:size()} members`)
		list:add("G")
		spec.list(walk(list), { "B", "F", "G" }, "an append after compaction")
	end,

	["act-on 9: 1,000 removals outside a walk examine at most 4,000 slots"] = function()
		local list = newList()
		for i = 1, 1000 do
			list:add(i)
		end
		local m = meter(list)
		for i = 1, 1000 do
			list:remove(i)
		end
		spec.eq(m.reads <= 4000, true, `slots examined by 1,000 removals: {m.reads}`)
		m.reads = 0
		spec.list(walk(list), {}, "empty")
		spec.eq(m.reads, 0, "a walk over an emptied list examines no slot")
	end,

	["pool rule: disabling and enabling 500 of 1,000 members in every walk stays bounded and ordered"] = function()
		local list = newList()
		local enabled = {}
		for i = 1, 1000 do
			list:add(i)
			table.insert(enabled, i)
		end
		local pool = {}
		for i = 1001, 1500 do
			table.insert(pool, i)
		end
		local m = meter(list)
		local tops = {}
		for frame = 1, 100 do
			local visited, added = {}, {}
			m.reads = 0
			list:forEach(function(item)
				table.insert(visited, item)
				if #added < 500 then
					list:remove(item)
					table.insert(pool, item)
					local reused = table.remove(pool, 1)
					list:add(reused)
					table.insert(added, reused)
				end
			end)
			spec.list(visited, enabled, `frame {frame} walks in enable order`)
			spec.eq(m.reads <= 6 * 1000, true, `frame {frame} examined {m.reads} slots`)
			tops[frame] = m.top
			enabled = table.move(added, 1, #added, #enabled - 499, { table.unpack(enabled, 501) })
		end
		spec.eq(tops[100], tops[5], "the highest slot index stops growing after the first 5 frames")
		spec.eq(list:size(), 1000, "members")
	end,
}
