local spec = require("./lib/load")
local realm = require("./lib/reactive-world")

local function world()
	local r = realm()
	r.ownership = r.graph.module("internal/reactive/ownership")
	return r
end

local function observers(source): number
	local count = 0
	for _ in source.observers do
		count += 1
	end
	return count
end

local function record(calls, value)
	table.insert(calls, value)
end

-- Binds `readable` the way its callers do: the baseline is read untracked just before.
local function bind(r, readable, calls)
	return r.reactive.createBinding(readable, r.api.untracked(readable), record, calls)
end

return {
	["a binding applies nothing at creation and passes the latest value to its sink once per flush"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		r.root(function()
			bind(r, coins, calls)
		end)
		spec.eq(#r.engine.deferred, 0, "a state binding queues nothing at creation")
		r.flush()
		spec.list(calls, {}, "nothing applied at creation or on the first flush")
		coins:set(2)
		coins:set(3)
		spec.list(calls, {}, "nothing at the write")
		r.flush()
		spec.list(calls, { 3 }, "one call with the latest value, passed with its target")
	end,
	["a value that changes and changes back before the flush makes no call"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		r.root(function()
			bind(r, coins, calls)
		end)
		coins:set(2)
		coins:set(1)
		r.flush()
		spec.list(calls, {}, "rawequal to the last value passed")
	end,
	["a getter is read again on the first flush and tracks what each read reads"] = function()
		local r = world()
		local flag, a, b = r.api.state(true), r.api.state("a1"), r.api.state("b1")
		local reads = 0
		local calls = {}
		r.root(function()
			bind(r, function()
				reads += 1
				return if flag() then a() else b()
			end, calls)
		end)
		spec.eq(reads, 1, "the caller's baseline read")
		r.flush()
		spec.eq(reads, 2, "read again on the first flush")
		spec.list(calls, {}, "an equal value made no call")
		b:set("b2")
		r.flush()
		spec.list(calls, {}, "an unread state does not reach it")
		flag:set(false)
		r.flush()
		spec.list(calls, { "b2" }, "the branch switched")
		a:set("a2")
		r.flush()
		spec.list(calls, { "b2" }, "the state it stopped reading no longer reaches it")
		b:set("b3")
		r.flush()
		spec.list(calls, { "b2", "b3" }, "the state it now reads does")
	end,
	["a getter that returns a new table calls the sink on the first flush"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		r.root(function()
			bind(r, function()
				return { coins() }
			end, calls)
		end)
		r.flush()
		spec.eq(#calls, 1, "a new table is a new value")
		r.flush()
		spec.eq(#calls, 1, "no call while nothing it reads changes")
	end,
	["a binding ends with its owner and leaves its sources"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		local derived = {}
		local dispose = r.root(function()
			bind(r, coins, calls)
			bind(r, function()
				return coins() * 2
			end, derived)
		end)
		r.flush()
		spec.eq(observers(coins), 2, "both bindings read the state")
		coins:set(2)
		dispose()
		spec.eq(observers(coins), 0, "no subscription is left")
		r.flush()
		spec.list(calls, {}, "a queued call does not run after the end")
		spec.list(derived, {}, "nor one through a getter")
	end,
	["a binding whose owner ends while a body under it runs leaves its sources at once"] = function()
		local r = world()
		local coins = r.api.state(1)
		local during
		local dispose
		dispose = r.root(function()
			r.api.effect(function()
				bind(r, coins, {})
				dispose()
				during = observers(coins)
			end)
		end)
		r.flush()
		spec.eq(during, 0, "unsubscribed before the run that ended it returned")
	end,
	["under an ended owner a binding is not made, and with no owner it throws"] = function()
		local r = world()
		local coins = r.api.state(1)
		local made = "unset"
		local dispose = r.root(function()
			r.api.onCleanup(function()
				made = bind(r, coins, {})
			end)
		end)
		dispose()
		spec.eq(made, nil, "nothing made under the ended scope")
		spec.eq(observers(coins), 0, "and nothing subscribed")
		local ok, err = pcall(bind, r, coins, {})
		spec.eq(ok, false, "no owner")
		spec.has(err, "needs an owner", "the message")
	end,
	["a root binding outlives the owner around it and ends when retired"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		local binding
		local dispose = r.root(function()
			binding = r.reactive.createRootBinding(coins, coins(), record, calls)
		end)
		dispose()
		coins:set(2)
		r.flush()
		spec.list(calls, { 2 }, "the root binding still updates")
		r.ownership.retire(binding)
		spec.eq(observers(coins), 0, "retiring left the state")
		coins:set(3)
		r.flush()
		spec.list(calls, { 2 }, "and stopped the calls")
	end,
	["creating a binding inside a computed throws"] = function()
		local r = world()
		local coins = r.api.state(1)
		local bad = r.api.computed(function()
			r.reactive.createRootBinding(coins, 1, record, {})
			return 1
		end)
		local ok, err = pcall(bad)
		spec.eq(ok, false, "the read threw")
		spec.has(err, "computed must not create bindings", "the message")
		spec.eq(observers(coins), 0, "nothing subscribed")
	end,
	["a throw in the read is cached like a computed's and stops the binding, reported once, like a watch"] = function()
		local r = world()
		local coins = r.api.state(1)
		local reads = 0
		local calls = {}
		r.root(function()
			bind(r, function()
				reads += 1
				if coins() < 0 then
					error("negative coins")
				end
				return coins()
			end, calls)
		end)
		r.flush()
		local before = reads
		coins:set(-1)
		r.flush()
		spec.eq(reads, before + 1, "one read, whose throw was cached")
		spec.eq(r.logged("negative coins"), 1, "reported once")
		spec.eq(r.logged("binding at"), 1, "as a binding")
		coins:set(5)
		r.flush()
		spec.list(calls, {}, "the stopped binding makes no call")
		spec.eq(reads, before + 1, "and reads nothing")
		spec.eq(observers(coins), 0, "and holds no subscription")
	end,
	["a yield in the read is cached like a computed's and stops the binding, reported once, like a watch"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		local other = {}
		r.root(function()
			bind(r, function()
				if coins() == 2 then
					coroutine.yield()
				end
				return coins()
			end, calls)
			bind(r, coins, other)
		end)
		r.flush()
		coins:set(2)
		r.flush()
		spec.eq(r.logged("yielded"), 1, "reported once")
		spec.list(other, { 2 }, "the flush went on to the next binding")
		coins:set(3)
		r.flush()
		spec.list(calls, {}, "the stopped binding makes no call")
		spec.list(other, { 2, 3 }, "the other binding still updates")
		spec.eq(observers(coins), 1, "only the other binding still reads the state")
	end,
	["a throw in the sink stops only that binding, reported once"] = function()
		local r = world()
		local coins = r.api.state(1)
		local calls = {}
		r.root(function()
			r.reactive.createBinding(coins, 1, function()
				error("bad property")
			end, nil)
			bind(r, coins, calls)
		end)
		coins:set(2)
		r.flush()
		spec.eq(r.logged("bad property"), 1, "reported once")
		spec.list(calls, { 2 }, "the sibling got the value")
		spec.eq(observers(coins), 1, "only the sibling still reads the state")
	end,
	["bindings and effects share one creation order on the flush"] = function()
		local r = world()
		local states = { r.api.state(0), r.api.state(0), r.api.state(0) }
		local order = {}
		r.root(function()
			r.api.effect(function()
				table.insert(order, `effect 1 {states[1]()}`)
			end)
			r.reactive.createBinding(states[2], 0, function(_, value)
				table.insert(order, `binding 2 {value}`)
			end, nil)
			r.api.effect(function()
				table.insert(order, `effect 3 {states[3]()}`)
			end)
		end)
		r.flush()
		table.clear(order)
		for i = 3, 1, -1 do
			states[i]:set(1)
		end
		r.flush()
		spec.list(order, { "effect 1 1", "binding 2 1", "effect 3 1" }, "in creation order")
	end,
	["a sink that writes its own source stops at the per-effect bound, reported once"] = function()
		local r = world()
		local count = r.api.state(0)
		local calls = 0
		r.root(function()
			r.reactive.createBinding(count, 0, function(_, value)
				calls += 1
				count:set(value + 1)
			end, nil)
		end)
		count:set(1)
		r.flush()
		spec.eq(calls, r.limits.MAX_RUNS_PER_EFFECT, "it ran to the bound")
		spec.eq(r.logged(`ran {r.limits.MAX_RUNS_PER_EFFECT} times in one flush`), 1, "and stopped, once")
		spec.eq(observers(count), 0, "and left its source")
	end,
}
