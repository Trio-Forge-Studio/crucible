-- S6 trial adapter, extracted from the compiled patches.ts implementation.
-- Kept with the benchmark so the trial remains runnable without adopting a runtime dependency.
return function(patch, values)
	local costOf = values.costOf
	local spend = values.spend
	local applyPatch = patch.apply
	local encoded = setmetatable({}, {
		__mode = "k",
	})
	local decoded = setmetatable({}, {
		__mode = "kv",
	})
	local SAME = {}
	local NIL = {}
	local ARRAY_MARKER = "__array"
	local differences = {}
	local function keys(value, cache, encode)
		if type(value) ~= "table" then
			return value
		end
		local known = cache[value]
		if known ~= nil then
			return if known == SAME then value else known
		end
		local changes = {}
		local source = value
		local array = if encode then rawget(value, 1) ~= nil else source[ARRAY_MARKER] == true
		for key, child in pairs(source) do
			if not encode and array and key == ARRAY_MARKER then
				continue
			end
			local name = key
			if array then
				name = if encode then tostring(key) else tonumber(key)
			elseif type(key) == "string" then
				if encode and (key == "__none" or key == ARRAY_MARKER or string.sub(key, 1, 1) == "\000") then
					name = `\000{key}`
				elseif not encode and string.sub(key, 1, 1) == "\000" then
					name = string.sub(key, 2)
				end
			end
			local updated = keys(child, cache, encode)
			if name ~= key or not rawequal(updated, child) then
				table.insert(changes, { key, name, updated })
			end
		end
		local result = if #changes == 0 and not array then source else table.clone(source)
		for _1, _2 in changes do
			local key = _2[1]
			local name = _2[2]
			if key ~= name then
				result[key] = nil
			end
		end
		for _3, _4 in changes do
			local name = _4[2]
			local updated = _4[3]
			result[name] = updated
		end
		if array then
			result[ARRAY_MARKER] = if encode then true else nil
		end
		if not table.isfrozen(result) then
			table.freeze(result)
		end
		cache[value] = if result == value then SAME else result
		if result ~= value then
			local reverse = if encode then decoded else encoded
			reverse[result] = value
		end
		return result
	end
	local function beginPatches()
		differences = {}
	end
	local function snapshot(value)
		return keys(value, encoded, true)
	end
	local function difference(current, updated, work)
		local before = if current == nil then NIL else current
		local after = if updated == nil then NIL else updated
		local byNext = differences[before]
		if byNext == nil then
			byNext = {}
			differences[before] = byNext
		end
		local found = byNext[after]
		if found == nil then
			local visits = costOf(current).visits + costOf(updated).visits
			if visits > work.limit - work.visits then
				work.visits = work.limit
				spend(work)
			end
			work.visits += visits
			found = {
				delta = patch.diff(snapshot(current), snapshot(updated)),
			}
			byNext[after] = found
		end
		return found.delta
	end
	local function apply(current, delta, initial)
		local value = if initial then delta else applyPatch(snapshot(current), delta)
		return keys(value, decoded, false)
	end
	return {
		beginPatches = beginPatches,
		snapshot = snapshot,
		difference = difference,
		apply = apply,
	}
end
