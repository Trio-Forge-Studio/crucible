local roblox = require("@lune/roblox")
local spec = require("./lib/load")
local realm = require("./lib/reactive-world")
local life = require("./lib/reactive-life")
local ownable = require("./lib/ownable")

local function world()
	return realm(nil, ownable.globals)
end

local function logger(events)
	return function(label)
		return function()
			table.insert(events, label)
		end
	end
end

return {
	["rule 23 rule 28 own releases each kind of value through its own call when its scope is disposed"] = function()
		local r = world()
		local events = {}
		local log = logger(events)
		local folder = roblox.Instance.new("Folder")
		local part = roblox.Instance.new("Part")
		part.Parent = folder
		local thread = coroutine.create(function() end)
		local connection = ownable.connection(log("connection"))
		local returned = {}
		local dispose = r.root(function()
			table.insert(returned, r.api.own(log("function")))
			table.insert(returned, r.api.own(connection) == connection)
			table.insert(returned, r.api.own(part) == part)
			table.insert(returned, r.api.own(thread) == thread)
			r.api.own({ destroy = log("destroy") })
			r.api.own({ Destroy = log("Destroy") })
			r.api.own({ destroy = log("both: destroy"), Destroy = log("both: Destroy") })
		end)
		spec.eq(typeof(returned[1]), "function", "own returns the function")
		spec.eq(returned[2], true, "own returns the connection")
		spec.eq(returned[3], true, "own returns the Instance")
		spec.eq(returned[4], true, "own returns the thread")
		spec.eq(#events, 0, "nothing is released while the scope lives")
		spec.eq(part.Parent, folder, "the Instance is alive")
		spec.eq(coroutine.status(thread), "suspended", "the thread is alive")
		dispose()
		spec.list(
			events,
			{ "both: destroy", "Destroy", "destroy", "connection", "function" },
			"newest first; an object with both methods goes through destroy()"
		)
		spec.eq(part.Parent, nil, "the Instance was destroyed")
		spec.eq(coroutine.status(thread), "dead", "the thread was cancelled")
		dispose()
		spec.eq(#events, 5, "a second dispose releases nothing twice")
		spec.eq(r.logged("cleanup"), 0, "no failure reported")
	end,
	["rule 21 own in onEnable releases right after onDisable; what awake owns, after onDestroy"] = function()
		local w = life(ownable.globals)
		local object = w.make("Door", {
			awake = function()
				w.api.own(ownable.connection(function()
					w.log("awake connection")
				end))
			end,
			onEnable = function()
				w.api.own(ownable.connection(function()
					w.log("enable connection")
				end))
				w.api.own(ownable.instance("Light", function()
					w.log("enable instance")
				end))
			end,
		})
		w.boot(object)
		local mark = #w.events
		w.life.request(object, w.Want.Disabled)
		spec.list(
			w.since(mark),
			{ "onDisable:Door", "enable instance", "enable connection" },
			"the interval's values, newest first, after onDisable"
		)
		w.life.request(object, w.Want.Enabled)
		w.life.request(object, w.Want.Disabled)
		spec.list(
			w.since(mark + 3),
			{ "onEnable:Door", "onDisable:Door", "enable instance", "enable connection" },
			"a re-enable owns them again, once"
		)
		mark = #w.events
		w.life.request(object, w.Want.Destroyed)
		spec.list(
			w.since(mark),
			{ "onDestroy:Door", "awake connection", "ended:Door" },
			"the generation's value after onDestroy"
		)
	end,
	["rule 28 own and onCleanup share one list, released newest first"] = function()
		local r = world()
		local events = {}
		local log = logger(events)
		r.root(function()
			r.api.onCleanup(log("cleanup 1"))
			r.api.own(ownable.connection(log("connection 2")))
			r.api.own(log("function 3"))
			r.api.own({ destroy = log("destroy 4") })
			r.api.own(ownable.instance("Part", log("instance 5")))
			r.api.onCleanup(log("cleanup 6"))
		end)()
		spec.list(
			events,
			{ "cleanup 6", "instance 5", "destroy 4", "function 3", "connection 2", "cleanup 1" },
			"one order across both"
		)
	end,
	["rule 28 a nil cleanup from Luau is dropped and shifts no release after it"] = function()
		local r = world()
		local events = {}
		local log = logger(events)
		r.root(function()
			r.api.onCleanup(log("before"))
			r.api.onCleanup(nil)
			r.api.own(ownable.connection(log("connection")))
			r.api.own({ destroy = log("destroy") })
		end)()
		spec.list(events, { "destroy", "connection", "before" }, "each released by its own call")
		spec.eq(r.logged("cleanup"), 0, "no failure reported")
	end,
	["rule 28 a release that throws or yields is reported and the rest still run"] = function()
		local r = world()
		local events = {}
		local log = logger(events)
		r.root(function()
			r.api.own(log("first"))
			r.api.own(ownable.instance("Door", function()
				error("parent locked")
			end))
			r.api.own({
				destroy = function()
					coroutine.yield()
				end,
			})
			r.api.own(function()
				error("function failed")
			end)
			r.api.own({
				Destroy = function()
					error("Destroy failed")
				end,
			})
			r.api.own(ownable.connection(log("last")))
		end)()
		spec.list(events, { "last", "first" }, "every other release ran")
		spec.eq(r.logged("cleanup of Door (a root scope) failed: "), 1, "the Instance, named by describe")
		spec.eq(r.logged("parent locked"), 1, "with its error")
		spec.eq(r.logged("cleanup at "), 3, "code that failed is named where it was written")
		spec.eq(r.logged("yielded"), 1, "the yielding destroy() was cancelled and reported")
		spec.eq(r.logged("function failed"), 1, "the function's error")
		spec.eq(r.logged("Destroy failed"), 1, "the Destroy() error")
	end,
	["rule 26 own inside an effect run releases before the next run, after the returned cleanup"] = function()
		local r = world()
		local s = r.charm.atom(1)
		local events = {}
		r.root(function()
			r.api.effect(function()
				local n = s()
				r.api.own(ownable.connection(function()
					table.insert(events, `connection {n}`)
				end))
				return function()
					table.insert(events, `cleanup {n}`)
				end
			end)
		end)
		r.flush()
		s(2)
		r.flush()
		spec.list(events, { "cleanup 1", "connection 1" }, "the run's values ended before the next run")
	end,
	["rule 27 own under an ended owner releases at once and still returns its argument"] = function()
		local r = world()
		local s = r.charm.atom(1)
		local events = {}
		local dispose
		dispose = r.root(function()
			r.api.effect(function()
				if s() == 1 then
					return
				end
				dispose()
				local connection = ownable.connection(function()
					table.insert(events, "disconnected")
				end)
				table.insert(events, if r.api.own(connection) == connection then "returned it" else "returned another")
				r.api.own({
					destroy = function()
						table.insert(events, "destroyed")
					end,
				})
				table.insert(events, "after own")
			end)
		end)
		s(2)
		spec.list(events, { "disconnected", "returned it", "destroyed", "after own" }, "released inside each call")
	end,
	["rule 20 own needs an owner and a value it can release"] = function()
		local r = world()
		local released = false
		local ok, err = pcall(r.api.own, function()
			released = true
		end)
		spec.eq(ok, false, "no owner")
		spec.has(err, "effect() needs an owner", "onCleanup's error")
		spec.eq(released, false, "nothing was released")
		local rejected = {
			{ "number", 5 },
			{ "nil" },
			{ "string", "Door" },
			{ "table", {} },
			{ "table", { destroy = true } },
		}
		r.root(function()
			for _, case in rejected do
				local okOwn, errOwn = pcall(r.api.own, case[2])
				spec.eq(okOwn, false, case[1])
				spec.has(errOwn, "own() takes a function, an RBXScriptConnection", case[1])
				spec.has(errOwn, `got a {case[1]}`, case[1])
			end
		end)()
	end,
}
