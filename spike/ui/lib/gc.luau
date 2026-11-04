-- Lune's Luau accepts only collectgarbage("count"), so a full collect is forced instead: allocate small garbage
-- until a weakly held sentinel is collected, which takes one complete GC cycle, and repeat. The lowest count seen
-- over those cycles is the heap right after a sweep, which is live data plus what the sweep let through. That
-- leftover grows with the heap, so `bench` reports the noise of an idle measurement next to the results.
local gc = {}
local sink

local function cycle(low: number): number
	local probe = setmetatable({}, { __mode = "v" })
	probe[1] = table.create(1)
	while probe[1] ~= nil do
		for _ = 1, 32 do
			sink = table.create(2)
		end
		local c = collectgarbage("count")
		if c < low then
			low = c
		end
	end
	sink = nil
	return low
end

-- One complete cycle, so a timed section starts from a similar GC state.
function gc.cycle()
	cycle(math.huge)
end

-- The settled heap in bytes. A weak table frees a dead entry's value one cycle after the key, so cycles run until
-- the lowest count has not dropped by more than 1 KB for four cycles in a row.
function gc.settled(): number
	local low = cycle(math.huge)
	local steady = 0
	for _ = 1, 60 do
		local next = cycle(low)
		steady = if low - next > 1 then 0 else steady + 1
		low = next
		if steady == 4 then
			break
		end
	end
	return low * 1024
end

return gc
