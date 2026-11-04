-- Variant B's binding: a graph node that reads one source and writes one Instance property, for compiled UI code
-- only. It is shaped as an effect node so the flush queues, sorts, bounds and validates it like an effect, and so
-- its owner retires it like an effect when the owner ends. The patched flush sends it to `run` instead of `runOnce`.
--
-- What it skips, and why that is safe for compiler output:
--   * No worker coroutine, no lifecycle scope, no run lifetime. Its only code is a property write, which runs no
--     Luau under Deferred signal behaviour, so it cannot yield, create anything or register a cleanup.
--   * No dependency tracking. The compiler passes the one source: a state for `s()`, or a computed it wraps
--     around any other expression. The edge is fixed at creation, so no rewire on update.
--   * No cleanup return, no `runs` lifetime bookkeeping beyond what the flush already does.
-- What it keeps: it runs on Crucible's flush, in creation order with effects, under the flush's bounds; it ends
-- with its owner, through the same retire an effect gets; a computed source caches throws and yields, and a
-- binding whose source failed or whose write threw stops with a diagnostic, as an effect would.
local NODE_EFFECT = 2
local NODE_COMPUTED = 1
local CACHE_ERROR = 2
local MARK_CLEAN = 0

return function(r)
	local graph, ownership = r.graph, r.ownership
	local nextId, guardPure, rewire, validate = graph.nextId, graph.guardPure, graph.rewire, graph.validate
	local resolveOwner, adopt, retire, NO_OWNER =
		ownership.resolveOwner, ownership.adopt, ownership.retire, ownership.NO_OWNER
	local report = r.diagnostics.runtimeDiagnostic
	local NONE = table.freeze({})

	local function current(source): (boolean, any)
		if source.kind == NODE_COMPUTED then
			validate(source)
			local cache = source.cache
			if cache.kind == CACHE_ERROR then
				return false, cache.message
			end
			return true, cache.value
		end
		return true, source.value
	end

	local function set(instance, property, value)
		instance[property] = value
	end

	local function stop(binding, reason)
		retire(binding)
		pcall(report, "error", `Crucible: binding of {binding.origin} stopped: {reason}`)
	end

	local function run(binding)
		local edge = binding.dependencies[1]
		local source = edge.source
		local ok, value = current(source)
		edge.seen = source.version
		if not ok then
			stop(binding, value)
			return
		end
		local wrote, err = pcall(set, binding.instance, binding.origin, value)
		if not wrote then
			stop(binding, err)
		end
	end

	r.flushModule.useBindingRunner(run)

	-- `source` is a state, which is its own graph node, or a computed node from `computedNode`.
	local function bind(instance, property: string, source)
		guardPure("create bindings")
		local owner = resolveOwner()
		if owner == nil then
			error(NO_OWNER, 0)
		end
		local ok, value = current(source)
		if not ok then
			error(value, 0)
		end
		instance[property] = value
		if owner.ended then
			return
		end
		-- Fourteen fields once `adopt` adds `prev` and `next`, so the node fits a 16-slot hash. `origin`, which the
		-- flush prints in its diagnostics, is the property name; `dependencies[1]` is the one edge.
		local binding = {
			kind = NODE_EFFECT,
			id = nextId(),
			dependencies = { { source = source, seen = source.version } },
			mark = MARK_CLEAN,
			stopped = false,
			queued = false,
			parent = owner,
			linked = false,
			runs = 0,
			origin = property,
			binding = true,
			instance = instance,
		}
		rewire(binding, NONE, binding.dependencies)
		adopt(owner, binding)
	end

	return { bind = bind, computedNode = graph.createComputedNode }
end
