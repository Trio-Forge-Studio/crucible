local spec = require("./lib/load")
local realm = require("./lib/reactive-world")

-- Replaces `name` on an internal module before the flush imports it. The replacement throws while `armed[key]`.
local function inject(graph, path, name, armed, key, once)
	local module = graph.module(path)
	local original = module[name]
	module[name] = function(...)
		if armed[key] then
			if once then
				armed[key] = false
			end
			error(`injected {key}`)
		end
		return original(...)
	end
end

local function threeEffects(r, events)
	local s = r.api.state(0)
	r.root(function()
		for _, name in { "P", "Q", "R" } do
			r.api.effect(function()
				table.insert(events, `{name} {s()}`)
				r.api.effect(function() end)
			end)
		end
	end)
	r.flush()
	table.clear(events)
	return s
end

return {
	["limits the graph and flush ceilings and the T13 render commit priority are the contract's"] = function()
		local r = realm()
		spec.eq(r.limits.MAX_RUNS_PER_EFFECT, 64, "MAX_RUNS_PER_EFFECT")
		spec.eq(r.limits.MAX_EFFECT_RUNS_PER_FLUSH, 65536, "MAX_EFFECT_RUNS_PER_FLUSH")
		spec.eq(r.limits.MAX_FLUSH_ROUNDS, 256, "MAX_FLUSH_ROUNDS")
		spec.eq(r.limits.OWNER_CHILDREN_WARNING, 4096, "OWNER_CHILDREN_WARNING")
		spec.eq(r.limits.RENDER_COMMIT_PRIORITY, 2001, "RENDER_COMMIT_PRIORITY")
	end,
	["rule 34 T9 the per-effect bound stops only the feedback loop"] = function()
		local r = realm()
		local a, b, other = r.api.state(0), r.api.state(0), r.api.state(0)
		local runs = { first = 0, second = 0, healthy = 0 }
		r.root(function()
			r.api.effect(function()
				runs.first += 1
				b:set(a() + 1)
			end)
			r.api.effect(function()
				runs.second += 1
				a:set(b() + 1)
			end)
			r.api.effect(function()
				other()
				runs.healthy += 1
			end)
		end)
		r.flush()
		spec.eq(runs.first, r.limits.MAX_RUNS_PER_EFFECT, "the first loop member ran to the bound")
		spec.eq(r.logged(`ran {r.limits.MAX_RUNS_PER_EFFECT} times in one flush`), 1, "and stopped, once")
		spec.eq(#r.engine.deferred, 0, "the other member then settled")
		local second = runs.second
		b:set(-100)
		other:set(1)
		r.flush()
		spec.eq(runs.first, r.limits.MAX_RUNS_PER_EFFECT, "the stopped effect never runs again")
		spec.eq(runs.second, second + 1, "its partner was not stopped")
		spec.eq(runs.healthy, 2, "an unrelated effect was not stopped")
	end,
	["rule 34 run counts are per flush"] = function()
		local r = realm()
		local s = r.api.state(0)
		local runs = 0
		r.root(function()
			r.api.effect(function()
				s()
				runs += 1
			end)
		end)
		for i = 1, r.limits.MAX_RUNS_PER_EFFECT + 5 do
			s:set(i)
			r.flush()
		end
		spec.eq(runs, r.limits.MAX_RUNS_PER_EFFECT + 5, "one run per flush never reaches the bound")
		spec.eq(r.logged("times in one flush"), 0, "never stopped")
	end,
	["rule 34 T9 the total bound carries the rest, unstopped, to a flush requested at once"] = function()
		local r = realm()
		local s, u = r.api.state(0), r.api.state(0)
		local fanout, unrelated = 0, 0
		r.root(function()
			for _ = 1, r.limits.MAX_EFFECT_RUNS_PER_FLUSH do
				r.api.effect(function()
					fanout += 1
				end)
			end
			r.api.effect(function()
				u()
				unrelated += 1
			end)
		end)
		local first = table.remove(r.engine.deferred, 1)
		first()
		spec.eq(fanout, r.limits.MAX_EFFECT_RUNS_PER_FLUSH, "every run up to the bound")
		spec.eq(unrelated, 0, "the last queued effect waits")
		spec.eq(#r.engine.deferred, 1, "a new flush was requested at once")
		spec.eq(
			r.logged(`reached {r.limits.MAX_EFFECT_RUNS_PER_FLUSH} effect runs`),
			1,
			"one diagnostic names the bound"
		)
		r.flush()
		spec.eq(unrelated, 1, "the next flush ran it")
		u:set(1)
		s:set(1)
		r.flush()
		spec.eq(unrelated, 2, "not stopped")
	end,
	["rule 34 T9 the round bound carries the rest and reports once until a flush stays within bounds"] = function()
		local r = realm()
		local depth, u = r.api.state(0), r.api.state(0)
		local unrelated = 0
		r.root(function()
			r.api.effect(function()
				u()
				unrelated += 1
			end)
		end)
		r.flush()
		local function chain(limit)
			local function grow(k)
				r.api.effect(function()
					depth:set(k)
					if k == r.limits.MAX_FLUSH_ROUNDS then
						u:set(k)
					end
					if k < limit then
						grow(k + 1)
					end
				end)
			end
			r.root(function()
				grow(1)
			end)
		end
		chain(2 * r.limits.MAX_FLUSH_ROUNDS + 50)
		table.remove(r.engine.deferred, 1)()
		spec.eq(depth(), r.limits.MAX_FLUSH_ROUNDS, "the flush ended at the round bound")
		spec.eq(unrelated, 1, "the effect queued in the last round waits")
		spec.eq(#r.engine.deferred, 1, "a new flush was requested at once")
		spec.eq(r.logged(`reached {r.limits.MAX_FLUSH_ROUNDS} rounds`), 1, "one diagnostic names the bound")
		table.remove(r.engine.deferred, 1)()
		spec.eq(unrelated, 2, "carried unstopped and run first in the next flush")
		spec.eq(depth(), 2 * r.limits.MAX_FLUSH_ROUNDS, "that flush hit the bound too")
		spec.eq(
			r.logged(`reached {r.limits.MAX_FLUSH_ROUNDS} rounds`),
			1,
			"no second report before a flush within bounds"
		)
		r.flush()
		spec.eq(depth(), 2 * r.limits.MAX_FLUSH_ROUNDS + 50, "the chain completed")
		chain(r.limits.MAX_FLUSH_ROUNDS + 10)
		r.flush()
		spec.eq(r.logged(`reached {r.limits.MAX_FLUSH_ROUNDS} rounds`), 2, "reported again after a flush within bounds")
	end,
	["rule 35 T15 an internal error stops only the culprit and loses no queued entry"] = function()
		local armed = {}
		local r = realm(function(graph)
			inject(graph, "internal/reactive/ownership", "endLifetime", armed, "end", true)
		end)
		local events = {}
		local s = threeEffects(r, events)
		armed["end"] = true
		s:set(1)
		r.flush()
		spec.list(events, { "Q 1", "R 1" }, "the healthy later entries of the batch ran")
		spec.eq(r.logged("internal error; the reactive flush recovered"), 1, "one report")
		spec.eq(r.logged("reactive-flush:"), 1, "naming the culprit's origin")
		table.clear(events)
		s:set(2)
		r.flush()
		spec.list(events, { "Q 2", "R 2" }, "flushing was reset; the culprit never runs again")
	end,
	["rule 35 T15 recovery survives a failing stop and a failing diagnostic sink"] = function()
		local armed = {}
		local r = realm(function(graph)
			inject(graph, "internal/reactive/ownership", "endLifetime", armed, "end", true)
			inject(graph, "internal/reactive/ownership", "retire", armed, "retire", true)
			inject(graph, "internal/diagnostics", "runtimeDiagnostic", armed, "sink", false)
		end)
		local events = {}
		local s = threeEffects(r, events)
		armed["end"], armed.retire, armed.sink = true, true, true
		s:set(1)
		r.flush()
		spec.list(events, { "Q 1", "R 1" }, "both healthy entries still ran")
		armed.sink = false
		table.clear(events)
		s:set(2)
		r.flush()
		spec.list(events, { "Q 2", "R 2" }, "the culprit was latched stopped by plain writes; flushing is false")
	end,
	["rule 35 T15 an error while sorting the batch stops nothing"] = function()
		local armed = {}
		local r = realm(function(graph)
			inject(graph, "internal/reactive/graph", "createdBefore", armed, "sort", true)
		end)
		local events = {}
		local s = threeEffects(r, events)
		armed.sort = true
		s:set(1)
		r.flush()
		spec.list(events, { "P 1", "Q 1", "R 1" }, "the whole batch went back and the next flush ran it")
		spec.eq(r.logged("internal error; the reactive flush recovered: "), 1, "reported without a culprit")
	end,
	["rule 35 a failed flush request leaves it unrequested for the next enqueue"] = function()
		local r = realm()
		local defer = r.engine.port.defer
		local failing = true
		r.engine.port.defer = function(callback)
			if failing then
				failing = false
				error("defer failed")
			end
			defer(callback)
		end
		local runs = 0
		r.root(function()
			r.api.effect(function()
				runs += 1
			end)
		end)
		spec.eq(#r.engine.deferred, 0, "the request failed")
		r.root(function()
			r.api.effect(function()
				runs += 1
			end)
		end)
		spec.eq(#r.engine.deferred, 1, "the next enqueue requested a flush")
		r.flush()
		spec.eq(runs, 2, "both ran")
	end,
}
