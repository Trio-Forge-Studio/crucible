local golden = require("./lib/random-golden")
local spec = require("./lib/load")

local Rng = spec.module("util/random").Rng

local WORDS = { "a", "b", "c", "d", "e", "f", "g", "h" }

local function many(count: number, draw: () -> any): { any }
	local drawn = table.create(count)
	for index = 1, count do
		drawn[index] = draw()
	end
	return drawn
end

local function average(values: { number }): number
	local total = 0
	for _, value in values do
		total += value
	end
	return total / #values
end

local function counts(values: { any }): { [any]: number }
	local result = {}
	for _, value in values do
		result[value] = (result[value] or 0) + 1
	end
	return result
end

-- Integer outputs, and floats built only from integer arithmetic, match the recorded values bit for bit.
local function exact(actual: { any }, expected: { any }, label: string)
	assert(#actual == #expected, `{label}: expected {#expected} values, got {#actual}`)
	for index, value in expected do
		if actual[index] ~= value then
			error(`{label}[{index}]: expected {tostring(value)}, got {tostring(actual[index])}`, 2)
		end
	end
end

-- `normal` goes through math.log and math.cos, which a platform's libm may round differently in the last bit.
local function close(actual: { any }, expected: { any }, label: string)
	assert(#actual == #expected, `{label}: expected {#expected} values, got {#actual}`)
	for index, value in expected do
		local got = actual[index]
		local matches = if type(value) == "number"
			then type(got) == "number" and math.abs(got - value) <= 1e-12 * math.max(1, math.abs(value))
			else got == value
		if not matches then
			error(`{label}[{index}]: expected {tostring(value)}, got {tostring(got)}`, 2)
		end
	end
end

local function fails(call: () -> any, part: string, label: string)
	local ok, message = pcall(call)
	assert(not ok, `{label} is rejected`)
	spec.has(message, part, label)
end

-- Each recorded list depends on one method of one stream, so the specs below all start from this seed.
local function stream()
	return Rng.new("golden", 1)
end

return {
	["generates the xoshiro128** reference sequence"] = function()
		local rng = Rng:fromState({ 1, 2, 3, 4 })
		exact(
			many(6, function()
				return rng:uint32()
			end),
			{ 11520, 0, 5927040, 70819200, 2031721883, 1637235492 },
			"the reference sequence"
		)
		local again = Rng:fromState({ 1, 2, 3, 4 })
		exact(
			many(8, function()
				return again:uint32()
			end),
			golden.reference,
			"eight reference draws"
		)
	end,
	["derives the same stream from each seed"] = function()
		local expected: { [string]: { number } } = {
			["42"] = { 1768527656, 1179704497, 952457629, 392045271 },
			["-1"] = { 2514316023, 3712122463, 1288616284, 4212124868 },
			[tostring(2 ^ 40 + 7)] = { 940947505, 860629668, 2318667971, 1540612657 },
			houses = { 274776295, 1319200954, 1366996277, 148356336 },
			chalet12345 = { 1528027912, 922584547, 2892789382, 1156408121 },
			empty = { 3839691421, 3150456080, 2871649241, 402779910 },
		}
		local cases: { [string]: { any } } = {
			["42"] = { 42 },
			["-1"] = { -1 },
			[tostring(2 ^ 40 + 7)] = { 2 ^ 40 + 7 },
			houses = { "houses" },
			chalet12345 = { "chalet", 12345 },
			empty = { "" },
		}
		for name, seed in cases do
			local rng = Rng.new(table.unpack(seed))
			exact(
				many(4, function()
					return rng:uint32()
				end),
				expected[name],
				`seed {name}`
			)
		end
	end,
	["derives the same state from the same seed"] = function()
		exact(Rng.new(0):state(), { 2444478070, 1892977553, 204952106, 653490083 }, "the state of seed 0")
	end,
	["draws the recorded stream from every seed shape"] = function()
		assert(#golden.seeds >= 20, "numbers, strings of every length, text beyond ASCII, and several parts")
		for index, record in golden.seeds do
			local rng = Rng.new(table.unpack(record.seed))
			exact(rng:state(), record.state, `state of seed {index}`)
			exact(
				many(6, function()
					return rng:uint32()
				end),
				record.uint32,
				`stream of seed {index}`
			)
		end
	end,
	["tells seed parts apart"] = function()
		local states = {}
		for _, seed in { { "1" }, { 1 }, { "ab", "c" }, { "a", "bc" }, { "abc" } } do
			table.insert(states, table.concat(Rng.new(table.unpack(seed)):state(), ","))
		end
		local distinct = 0
		for _ in counts(states) do
			distinct += 1
		end
		spec.eq(distinct, #states, "every seed gives its own state")
	end,
	["draws a random seed without parts"] = function()
		spec.eq(
			table.concat(Rng.new():state(), ",") ~= table.concat(Rng.new():state(), ","),
			true,
			"two unseeded streams differ"
		)
	end,
	["rejects a seed that is not an integer within 2^53"] = function()
		for _, seed in { 0.5, 0 / 0, math.huge, 2 ^ 54 } do
			fails(function()
				return Rng.new(seed)
			end, "integer", `the seed {seed}`)
		end
	end,
	["restores and clones a state"] = function()
		local rng = Rng.new("save")
		rng:uint32()
		local copy = rng:clone()
		local restored = Rng:fromState(rng:state())
		local expected = many(5, function()
			return rng:uint32()
		end)
		exact(
			many(5, function()
				return copy:uint32()
			end),
			expected,
			"the clone"
		)
		exact(
			many(5, function()
				return restored:uint32()
			end),
			expected,
			"the restored stream"
		)
	end,
	["rejects a bad state"] = function()
		fails(function()
			return Rng:fromState({ 0, 0, 0, 0 })
		end, "all zeroes", "an all-zero state")
		fails(function()
			return Rng:fromState({ 1, 2, 3, 2 ^ 32 })
		end, "uint32", "a word past 32 bits")
	end,
	["forks an independent stream"] = function()
		local rng = Rng.new(7)
		local fork = rng:fork()
		local forked = many(4, function()
			return fork:uint32()
		end)
		local parent = many(4, function()
			return rng:uint32()
		end)
		assert(table.concat(forked, ",") ~= table.concat(parent, ","), "the fork draws its own numbers")
		local first, second = Rng.new(7):fork(), Rng.new(7):fork()
		exact(
			many(4, function()
				return first:uint32()
			end),
			many(4, function()
				return second:uint32()
			end),
			"the same seed forks the same stream"
		)
	end,
	["draws floats in [0, 1) with a fair mean"] = function()
		local rng = Rng.new("float")
		local values = many(100000, function()
			return rng:float()
		end)
		for _, value in values do
			assert(value >= 0 and value < 1, "a float is in [0, 1)")
		end
		assert(math.abs(average(values) - 0.5) < 0.005, "the mean is near one half")
	end,
	["draws floats between bounds"] = function()
		local rng = Rng.new("between")
		for _ = 1, 10000 do
			local value = rng:float(-3, 5)
			assert(value >= -3 and value < 5, "a float is within its bounds")
		end
		spec.eq(rng:float(2, 2), 2, "equal bounds give that bound")
		fails(function()
			return rng:float(1, 0)
		end, "min <= max", "reversed bounds")
	end,
	["draws every integer in range evenly"] = function()
		local rng = Rng.new("dice")
		local faces = counts(many(60000, function()
			return rng:int(1, 6)
		end))
		for face = 1, 6 do
			assert(math.abs((faces[face] or 0) - 10000) < 400, `face {face} comes up about 10000 times`)
		end
		spec.eq(rng:int(-4, -4), -4, "one value in range")
		local wide = rng:int(0, 2 ^ 32 - 1)
		assert(wide >= 0 and wide < 2 ^ 32, "the widest range stays in 32 bits")
	end,
	["rejects bad integer ranges"] = function()
		local rng = Rng.new(1)
		fails(function()
			return rng:int(2, 1)
		end, "min <= max", "reversed bounds")
		fails(function()
			return rng:int(0, 1.5)
		end, "integer bounds", "a fractional bound")
		fails(function()
			return rng:int(0, 2 ^ 32)
		end, "2^32", "a span past 2^32")
	end,
	["passes a chance at its probability"] = function()
		local rng = Rng.new("chance")
		local hits = 0
		for _ = 1, 100000 do
			if rng:chance(0.25) then
				hits += 1
			end
		end
		assert(math.abs(hits / 100000 - 0.25) < 0.005, "a quarter of the draws pass")
		spec.eq(Rng.new(1):chance(0), false, "probability 0 never passes")
		spec.eq(Rng.new(1):chance(1), true, "probability 1 always passes")
	end,
	["draws a normal distribution"] = function()
		local rng = Rng.new("normal")
		local values = many(100000, function()
			return rng:normal(10, 2)
		end)
		local mean = average(values)
		local squares = {}
		for index, value in values do
			squares[index] = value * value
		end
		assert(math.abs(mean - 10) < 0.03, "the mean is 10")
		assert(math.abs(math.sqrt(average(squares) - mean * mean) - 2) < 0.03, "the deviation is 2")
	end,
	["picks every item"] = function()
		local rng = Rng.new("pick")
		local seen = counts(many(50, function()
			return rng:pick({ "a", "b", "c" })
		end))
		local names = {}
		for name in seen do
			table.insert(names, name)
		end
		table.sort(names)
		spec.list(names, { "a", "b", "c" }, "every item is picked")
		fails(function()
			return rng:pick({})
		end, "empty", "an empty array")
	end,
	["picks by weight"] = function()
		local rng = Rng.new("weights")
		local picks = counts(many(100000, function()
			return rng:weightedIndex({ 1, 0, 3, 6 })
		end))
		spec.eq(picks[1], nil, "a zero weight is never chosen")
		assert(math.abs((picks[0] or 0) / 100000 - 0.1) < 0.005, "index 0 is chosen a tenth of the time")
		assert(math.abs((picks[2] or 0) / 100000 - 0.3) < 0.005, "index 2 is chosen 3 in 10 times")
		assert(math.abs((picks[3] or 0) / 100000 - 0.6) < 0.005, "index 3 is chosen 6 in 10 times")
	end,
	["picks items by a weight function"] = function()
		local rng = Rng.new("weighted")
		local items = { { name = "common", weight = 0 }, { name = "rare", weight = 1 } }
		local seen = {}
		local picked = rng:weighted(items, function(item, index)
			table.insert(seen, index)
			return item.weight
		end)
		spec.eq(picked.name, "rare", "the only weighted item is picked")
		spec.eq(picked, items[2], "the item itself comes back")
		spec.list(seen, { 0, 1 }, "the weight function gets each item's zero-based index")
	end,
	["rejects bad weights"] = function()
		local rng = Rng.new(1)
		fails(function()
			return rng:weightedIndex({ 0, 0 })
		end, "more than zero", "weights adding up to zero")
		fails(function()
			return rng:weightedIndex({})
		end, "more than zero", "no weights")
		fails(function()
			return rng:weightedIndex({ 1, -1 })
		end, "non-negative", "a negative weight")
		fails(function()
			return rng:weightedIndex({ 1, math.huge })
		end, "finite", "an infinite weight")
		fails(function()
			return rng:weightedIndex({ 1, 0 / 0 })
		end, "finite", "a NaN weight")
	end,
	["shuffles a copy into every order"] = function()
		local rng = Rng.new("shuffle")
		local orders = counts(many(6000, function()
			return table.concat(rng:shuffle({ 1, 2, 3 }), "")
		end))
		local seen = {}
		for order, count in orders do
			table.insert(seen, order)
			assert(math.abs(count - 1000) < 120, `order {order} comes up about 1000 times`)
		end
		table.sort(seen)
		spec.list(seen, { "123", "132", "213", "231", "312", "321" }, "every order appears")
		exact(Rng.new("same"):shuffle({ 1, 2, 3, 4, 5 }), Rng.new("same"):shuffle({ 1, 2, 3, 4, 5 }), "the same seed")
		spec.eq(#rng:shuffle({}), 0, "an empty array shuffles to an empty array")
		local original = { 1, 2, 3, 4, 5 }
		rng:shuffle(original)
		spec.list(original, { 1, 2, 3, 4, 5 }, "the array passed in stays as it was")
	end,
	["samples distinct items"] = function()
		local rng = Rng.new("sample")
		local items = { "a", "b", "c", "d", "e" }
		for _ = 1, 100 do
			local distinct = 0
			for _ in counts(rng:sample(items, 3)) do
				distinct += 1
			end
			spec.eq(distinct, 3, "three distinct items")
		end
		spec.eq(#rng:sample(items, 0), 0, "none")
		spec.eq(#rng:sample(items, 5), 5, "all")
		fails(function()
			return rng:sample(items, 6)
		end, "count from 0 to 5", "a count past the array")
	end,
	["floats, integers and chances follow the recorded draws"] = function()
		local floats = stream()
		exact(
			many(6, function()
				return floats:float()
			end),
			golden.floats,
			"float()"
		)
		local between = stream()
		exact(
			many(6, function()
				return between:float(-3, 5)
			end),
			golden.floatBetween,
			"float(min, max)"
		)
		spec.eq(stream():float(2, 2), golden.floatDegenerate, "float(2, 2)")
		local ints = stream()
		exact(
			many(12, function()
				return ints:int(1, 6)
			end),
			golden.ints,
			"int(1, 6)"
		)
		local negative = stream()
		exact(
			many(8, function()
				return negative:int(-4, 4)
			end),
			golden.intNegative,
			"int(-4, 4)"
		)
		local single = stream()
		exact(
			many(2, function()
				return single:int(-4, -4)
			end),
			golden.intSingle,
			"int(-4, -4)"
		)
		local wide = stream()
		exact(
			many(4, function()
				return wide:int(0, 2 ^ 32 - 1)
			end),
			golden.intWide,
			"int(0, 2^32 - 1)"
		)
		local half = stream()
		exact(
			many(8, function()
				return half:int(0, 2 ^ 31 + 5)
			end),
			golden.intHalfWord,
			"an int range that rejects draws"
		)
		local chances = stream()
		exact(
			many(10, function()
				return chances:chance(0.25)
			end),
			golden.chances,
			"chance(0.25)"
		)
	end,
	["normals follow the recorded draws"] = function()
		local plain = stream()
		close(
			many(6, function()
				return plain:normal()
			end),
			golden.normals,
			"normal()"
		)
		local shaped = stream()
		close(
			many(6, function()
				return shaped:normal(10, 2)
			end),
			golden.normalsShaped,
			"normal(10, 2)"
		)
	end,
	["picks, weights, shuffles and samples follow the recorded draws"] = function()
		local picks = stream()
		exact(
			many(10, function()
				return picks:pick(WORDS)
			end),
			golden.picks,
			"pick"
		)
		local indexes = stream()
		exact(
			many(12, function()
				return indexes:weightedIndex({ 1, 0, 3, 6 })
			end),
			golden.weightedIndexes,
			"weightedIndex"
		)
		local items = {
			{ name = "common", weight = 6 },
			{ name = "rare", weight = 1 },
			{ name = "none", weight = 0 },
			{ name = "mid", weight = 3 },
		}
		local weighted = stream()
		local names = many(8, function()
			return weighted:weighted(items, function(item)
				return item.weight
			end).name
		end)
		exact(names, golden.weighted, "weighted")
		local shuffles = stream()
		for index = 1, 4 do
			exact(shuffles:shuffle({ 1, 2, 3, 4, 5, 6, 7, 8 }), golden.shuffles[index], `shuffle {index}`)
		end
		spec.eq(#stream():shuffle({}), golden.shuffleEmpty, "shuffle of an empty array")
		local samples = stream()
		for index = 1, 4 do
			exact(samples:sample(WORDS, 3), golden.samples[index], `sample {index}`)
		end
		exact(stream():sample(WORDS, 8), golden.sampleAll, "sample of every item")
		spec.eq(#stream():sample(WORDS, 0), golden.sampleNone, "sample of none")
	end,
	["fork, clone and fromState continue the recorded stream"] = function()
		local rng = stream()
		rng:uint32()
		local fork = rng:fork()
		exact(rng:state(), golden.fork.parentState, "the parent's state after a fork")
		exact(fork:state(), golden.fork.forkState, "the fork's state")
		exact(
			many(4, function()
				return fork:uint32()
			end),
			golden.fork.fork,
			"the fork's draws"
		)
		exact(
			many(4, function()
				return rng:uint32()
			end),
			golden.fork.parent,
			"the parent's draws after the fork"
		)
		local second = stream()
		second:uint32()
		local copy = second:clone()
		local restored = Rng:fromState(second:state())
		exact(second:state(), golden.clone.state, "the state before copying")
		exact(
			many(5, function()
				return copy:uint32()
			end),
			golden.clone.clone,
			"the clone"
		)
		exact(
			many(5, function()
				return restored:uint32()
			end),
			golden.clone.restored,
			"the restored stream"
		)
		exact(
			many(5, function()
				return second:uint32()
			end),
			golden.clone.original,
			"the original"
		)
	end,
	["a long run of mixed calls ends where the recorded run did"] = function()
		local rng = Rng.new("mixed", 99)
		local log = {}
		for step = 1, 40 do
			local kind = step % 8
			local value
			if kind == 0 then
				value = rng:uint32()
			elseif kind == 1 then
				value = rng:float()
			elseif kind == 2 then
				value = rng:int(-50, 50)
			elseif kind == 3 then
				value = rng:normal(0, 3)
			elseif kind == 4 then
				value = rng:weightedIndex({ 5, 1, 0, 2 })
			elseif kind == 5 then
				value = rng:pick(WORDS)
			elseif kind == 6 then
				value = table.concat(rng:shuffle({ 1, 2, 3, 4, 5, 6 }), "")
			else
				value = table.concat(rng:sample(WORDS, 4), "")
			end
			table.insert(log, value)
		end
		close(log, golden.mixed.log, "the mixed draws")
		exact(rng:state(), golden.mixed.state, "the final state")
	end,
	["the documented examples print what they say"] = function()
		local rng = Rng.new("chalet", 12345)
		spec.eq(rng:int(1, 6), 5, "int")
		spec.eq(rng:pick({ "lamp", "vase", "clock" }), "vase", "pick")
		spec.eq(table.concat(rng:shuffle({ 1, 2, 3, 4, 5 }), " "), "5 4 1 2 3", "shuffle")
		local loot = Rng.new("loot", 7)
		spec.eq(({ "common", "rare", "epic" })[loot:weightedIndex({ 80, 18, 2 }) + 1], "common", "weightedIndex")
	end,
}
