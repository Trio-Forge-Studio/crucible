-- Swaps a dense list's `items` for a proxy that counts slot reads and records the highest slot written.
return function(list)
	local raw = list.items
	local meter = { reads = 0, top = 0 }
	list.items = setmetatable({}, {
		__index = function(_, slot)
			meter.reads += 1
			return raw[slot]
		end,
		__newindex = function(_, slot, value)
			if value ~= nil and slot > meter.top then
				meter.top = slot
			end
			raw[slot] = value
		end,
	})
	return meter
end
