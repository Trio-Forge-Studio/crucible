local spec = require("./lib/load")
local World = require("./lib/binding-world")
local tests = {}
local function test(name, fn)
	tests[name] = fn
end
do
	test("F1 probe: get inside a hook on an untagged instance", function()
		local w = World()
		local B = w.define("B")
		local plain = w.engine.instance("Plain")
		w.define("A", {
			awake = function()
				B:get(plain)
			end,
		})
		w.tagged("I", "A")
		w.boot()
		print("F1 untagged warnings", w.engine.logged("get pending"))
		spec.eq(w.engine.logged("get pending"), 0, "untagged is not pending")
	end)

	test("F1 control: get inside a hook on a pending pair still warns per site", function()
		local w = World()
		local B = w.define("B")
		local target = w.tagged("T", "B")
		w.define("A", {
			awake = function()
				for _ = 1, 2 do
					B:get(target)
				end
				B:get(target)
			end,
		})
		w.tagged("I", "A")
		w.boot()
		spec.eq(w.engine.logged("get pending"), 2, "two sites")
	end)

	test("F1 control: tag added but signal undelivered still warns", function()
		local w = World()
		local B = w.define("B")
		local A = w.define("A", {
			awake = function(self)
				B:get(self.instance)
			end,
		})
		w.boot()
		local i = w.engine.instance("I")
		w.engine.addTag(i, "B")
		w.engine.addTag(i, "A")
		A:get(i)
		print("F1 undelivered warnings", w.engine.logged("get pending"))
		spec.eq(w.engine.logged("get pending"), 1, "pending tag warns")
	end)

	test("F2 probe: refresh of an already-pending pair from a hook run by outside get", function()
		local w = World()
		local _, br = w.define("B")
		local target = w.engine.instance("T")
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(target)
			end,
		})
		w.boot()
		w.engine.addTag(target, "B")
		table.remove(w.engine.deferred, 1)()
		local i = w.tagged("I", "A")
		A:get(i)
		print("F2 pending pair bound after outside get", br.bound[target] ~= nil)
		spec.eq(br.bound[target] ~= nil, true, "bound in the same drain")
	end)

	test("E1: parked pair refreshed twice in one outside get binds once", function()
		local w = World()
		local awakes = 0
		local _, br = w.define("B", {
			awake = function()
				awakes += 1
			end,
		})
		local target = w.engine.instance("T")
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(target)
				w.behaviour:refresh(target)
			end,
			onEnable = function()
				w.behaviour:refresh(target)
			end,
		})
		w.boot()
		w.engine.addTag(target, "B")
		table.remove(w.engine.deferred, 1)()
		local i = w.tagged("I", "A")
		A:get(i)
		local first = br.bound[target]
		w.engine.flush()
		print("E1 bound", first ~= nil, "awakes", awakes, "same generation after flush", br.bound[target] == first)
		spec.eq(first ~= nil, true, "bound in the outside drain")
		spec.eq(awakes, 1, "one generation")
		spec.eq(br.bound[target], first, "flush keeps it")
	end)

	test("E2: outside get on a pair queued by an earlier signal, with an unrelated parked pair", function()
		local w = World()
		local B, br = w.define("B")
		local _, cr = w.define("C")
		w.boot()
		local t = w.tagged("T", "B")
		local u = w.tagged("U", "C")
		table.remove(w.engine.deferred, 1)()
		table.remove(w.engine.deferred, 1)()
		local got = B:get(t)
		local cDuring = cr.bound[u]
		w.engine.flush()
		print("E2 own bound", got ~= nil, "parked during", cDuring ~= nil, "parked after flush", cr.bound[u] ~= nil)
		spec.eq(got ~= nil and br.bound[t] ~= nil and br.bound[t].object == got, true, "own pair bound")
		spec.eq(cDuring, nil, "parked pair waits")
		spec.eq(cr.bound[u] ~= nil, true, "parked pair restored and bound")
	end)

	test("E2b: hook get inside an outside drain on a parked pair", function()
		local w = World()
		local B, br = w.define("B")
		local target = w.engine.instance("T")
		local seen
		local A = w.define("A", {
			awake = function()
				seen = B:get(target)
			end,
		})
		w.boot()
		w.engine.addTag(target, "B")
		table.remove(w.engine.deferred, 1)()
		local i = w.tagged("I", "A")
		A:get(i)
		local during = br.bound[target]
		w.engine.flush()
		print(
			"E2b hook saw",
			seen,
			"bound after outside get",
			during ~= nil,
			"warnings",
			w.engine.logged("get pending"),
			"same after flush",
			br.bound[target] == during
		)
		spec.eq(seen, nil, "hook sees nothing")
		spec.eq(during ~= nil, true, "hook get queues the parked pair in the same drain")
		spec.eq(w.engine.logged("get pending"), 1, "parked pair warns once")
		spec.eq(br.bound[target] ~= nil, true, "bound")
	end)

	test("E3: parked pair whose tag was removed before the outside get is not bound", function()
		local w = World()
		local _, br = w.define("B")
		local target = w.engine.instance("T")
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(target)
			end,
		})
		w.boot()
		w.engine.addTag(target, "B")
		table.remove(w.engine.deferred, 1)()
		w.engine.removeTag(target, "B")
		local i = w.tagged("I", "A")
		A:get(i)
		local during = br.bound[target]
		w.engine.flush()
		print("E3 bound during", during ~= nil, "after", br.bound[target] ~= nil, "logs", #w.engine.logs)
		spec.eq(during, nil, "not bound")
		spec.eq(br.bound[target], nil, "never bound")
	end)

	test("E4: two outside gets in a row keep a third parked pair", function()
		local w = World()
		local A, ar = w.define("A")
		local B, br = w.define("B")
		local _, cr = w.define("C")
		w.boot()
		local a = w.tagged("a", "A")
		local b = w.tagged("b", "B")
		local c = w.tagged("c", "C")
		for _ = 1, 3 do
			table.remove(w.engine.deferred, 1)()
		end
		A:get(a)
		B:get(b)
		local cDuring = cr.bound[c]
		w.engine.flush()
		spec.eq(ar.bound[a] ~= nil and br.bound[b] ~= nil, true, "both gets bound")
		spec.eq(cDuring, nil, "c waits")
		spec.eq(cr.bound[c] ~= nil, true, "c bound after flush")
	end)

	test("F1 residual: hook get on a tagged wrong-class instance", function()
		local w = World()
		local B = w.define("B")
		local wrong = w.engine.instance("W", "Part")
		w.engine.addTag(wrong, "B")
		local A = w.define("A", {
			awake = function()
				B:get(wrong)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		print(
			"F1 residual wrong-class warnings",
			w.engine.logged("get pending"),
			"class warnings",
			w.engine.logged("needs a Model")
		)
		spec.eq(w.engine.logged("get pending"), 0, "wrong class is not pending")
	end)

	test("F1 residual: hook get on a tagged instance rejected by accepts", function()
		local w = World()
		local B = w.define("B", {
			accepts = function()
				return false
			end,
		})
		local rejected = w.tagged("R", "B")
		local A = w.define("A", {
			awake = function()
				B:get(rejected)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		print("F1 residual accepts-rejected warnings", w.engine.logged("get pending"))
		spec.eq(w.engine.logged("get pending"), 0, "rejected is not pending")
	end)

	test("F1 residual: hook get on a tagged instance in ServerStorage", function()
		local w = World()
		local B = w.define("B")
		local stored = w.engine.instance("S")
		stored.container = "ServerStorage"
		w.engine.addTag(stored, "B")
		local A = w.define("A", {
			awake = function()
				B:get(stored)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		print("F1 residual excluded warnings", w.engine.logged("get pending"))
		spec.eq(w.engine.logged("get pending"), 0, "excluded is not pending")
	end)

	test("F8: wrong-class warning names the class", function()
		local w = World()
		local A = w.define("game/vehicles/server/Van:Van", {}, { tag = "Van" })
		w.boot()
		local i = w.engine.instance("Workspace.Van1", "Part")
		w.engine.addTag(i, "Van")
		A:get(i)
		for _, entry in w.engine.logs do
			print("F8 log", entry.message)
		end
		spec.eq(w.engine.logged("Workspace.Van1 is a Part; Van needs a Model"), 1, "names class")
		spec.eq(w.engine.logged("game/vehicles"), 0, "no record id")
	end)

	test("F8b: class name differs from the tag", function()
		local w = World()
		local A = w.define("game/vehicles/server/Van:Truck", {}, { tag = "Van" })
		w.boot()
		local i = w.engine.instance("Workspace.Van1", "Part")
		w.engine.addTag(i, "Van")
		A:get(i)
		for _, entry in w.engine.logs do
			print("F8b log", entry.message)
		end
		spec.eq(w.engine.logged("Workspace.Van1 is a Part; Truck needs a Model"), 1, "names class not tag")
	end)
end

do
	test("PN2 hook get on a removed (suppressed) tagged instance does not warn pending", function()
		local w = World()
		local B, br = w.define("B")
		local target = w.tagged("T", "B")
		local A = w.define("A", {
			awake = function()
				B:get(target)
			end,
		})
		w.boot()
		B:get(target):remove()
		w.engine.flush()
		print("PN2 bound after remove", br.bound[target] ~= nil, "suppressed", br.suppressed[target] ~= nil)
		local i = w.tagged("I", "A")
		A:get(i)
		print("PN2 warnings", w.engine.logged("get pending"))
		spec.eq(br.bound[target], nil, "removed")
		spec.eq(w.engine.logged("get pending"), 0, "suppressed is not pending")
	end)

	test("PN4 refreshed parked pair is reconciled once", function()
		local w = World()
		local calls = 0
		local _, br = w.define("B", {
			accepts = function()
				calls += 1
				return true
			end,
		})
		local target = w.engine.instance("T")
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(target)
			end,
		})
		w.boot()
		w.engine.addTag(target, "B")
		table.remove(w.engine.deferred, 1)()
		local i = w.tagged("I", "A")
		A:get(i)
		w.engine.flush()
		print("PN4 accepts calls", calls)
		spec.eq(br.bound[target] ~= nil, true, "bound")
		spec.eq(calls, 1, "one reconcile")
	end)

	test("PN1 restored parked pair is reconciled once when refreshed before the deferred drain", function()
		local w = World()
		local calls = 0
		local B = w.define("B")
		local _, cr = w.define("C", {
			accepts = function()
				calls += 1
				return true
			end,
		})
		w.boot()
		local t = w.tagged("T", "B")
		local u = w.tagged("U", "C")
		table.remove(w.engine.deferred, 1)()
		table.remove(w.engine.deferred, 1)()
		B:get(t)
		w.behaviour:refresh(u)
		w.engine.flush()
		print("PN1 accepts calls", calls)
		spec.eq(cr.bound[u] ~= nil, true, "bound")
		spec.eq(calls, 1, "one reconcile")
	end)
end

do
	local function numeric(required)
		return {
			name = "Speed",
			required = required,
			check = function(v)
				return type(v) == "number"
			end,
		}
	end

	test(
		"X1 accepted limit: rejected pair, tag removed and re-added in the hook before get, binds after flush",
		function()
			local w = World()
			local allow = false
			local B, br = w.define("B", {
				accepts = function()
					return allow
				end,
			})
			local r = w.tagged("R", "B")
			local A = w.define("A", {
				awake = function()
					allow = true
					w.engine.removeTag(r, "B")
					w.engine.addTag(r, "B")
					B:get(r)
				end,
			})
			w.boot()
			local i = w.tagged("I", "A")
			A:get(i)
			local warned = w.engine.logged("get pending")
			w.engine.flush()
			print("X1 warnings", warned, "bound after flush", br.bound[r] ~= nil)
			spec.eq(br.bound[r] ~= nil, true, "pair was pending: it binds after the flush")
			spec.eq(warned, 0, "undelivered tag round trip keeps user accepts rejection")
		end
	)

	test("X2 invalid attribute made valid in the hook before get, binds after flush", function()
		local w = World()
		local B, br = w.define("B", {}, { attributes = { speed = numeric(true) } })
		local r = w.tagged("R", "B", { Speed = "fast" })
		local A = w.define("A", {
			awake = function()
				w.engine.setAttribute(r, "Speed", 5)
				B:get(r)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("X2 warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(br.bound[r] ~= nil, true, "pair was pending: it binds after the flush")
		spec.eq(warned, 1, "pending pair warns")
	end)

	test("X3 two hook gets at two sites on a wrong-class instance in one hook", function()
		local w = World()
		local B, br = w.define("B")
		local wrong = w.engine.instance("W", "Part")
		w.engine.addTag(wrong, "B")
		local A = w.define("A", {
			awake = function()
				B:get(wrong)
				B:get(wrong)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		w.engine.flush()
		print("X3 warnings", w.engine.logged("get pending"), "bound", br.bound[wrong] ~= nil)
		spec.eq(br.bound[wrong], nil, "never binds")
		spec.eq(w.engine.logged("get pending"), 0, "wrong class is not pending")
	end)

	test("X4 two behaviours' hooks in one drain get a rejected instance", function()
		local w = World()
		local B, br = w.define("B", {
			accepts = function()
				return false
			end,
		})
		local r = w.tagged("R", "B")
		w.define("A", {
			awake = function()
				B:get(r)
			end,
		})
		w.define("C", {
			awake = function()
				B:get(r)
			end,
		})
		w.boot()
		w.tagged("I", "A")
		w.tagged("J", "C")
		w.engine.flush()
		print("X4 warnings", w.engine.logged("get pending"), "bound", br.bound[r] ~= nil)
		spec.eq(br.bound[r], nil, "never binds")
		spec.eq(w.engine.logged("get pending"), 0, "rejected is not pending")
	end)

	test("X5 awake and onEnable of one object get a wrong-class instance", function()
		local w = World()
		local B = w.define("B")
		local wrong = w.engine.instance("W", "Part")
		w.engine.addTag(wrong, "B")
		local A = w.define("A", {
			awake = function()
				B:get(wrong)
			end,
			onEnable = function()
				B:get(wrong)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		print("X5 warnings", w.engine.logged("get pending"))
		spec.eq(w.engine.logged("get pending"), 0, "wrong class is not pending")
	end)

	test("X6 two hook gets at two sites on an untagged instance in one hook", function()
		local w = World()
		local B = w.define("B")
		local plain = w.engine.instance("Plain")
		local A = w.define("A", {
			awake = function()
				B:get(plain)
				B:get(plain)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		print("X6 warnings", w.engine.logged("get pending"))
		spec.eq(w.engine.logged("get pending"), 0, "untagged is not pending")
	end)

	test("X7 hook get on a tagged instance whose required attribute is invalid", function()
		local w = World()
		local B, br = w.define("B", {}, { attributes = { speed = numeric(true) } })
		local r = w.tagged("R", "B", { Speed = "fast" })
		local A = w.define("A", {
			awake = function()
				B:get(r)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		w.engine.flush()
		print("X7 warnings", w.engine.logged("get pending"), "bound", br.bound[r] ~= nil)
		spec.eq(br.bound[r], nil, "never binds")
		spec.eq(w.engine.logged("get pending"), 0, "invalid attribute is not pending")
	end)
end

do
	local function check(name, required)
		return {
			name = name,
			required = required,
			check = function(v)
				return type(v) == "number"
			end,
		}
	end

	test("Y1 two invalid attributes, hook fixes only one, then get", function()
		local w = World()
		local B, br = w.define("B", {}, { attributes = { speed = check("Speed", true), size = check("Size", true) } })
		local r = w.tagged("R", "B", { Speed = "fast", Size = "big" })
		local A = w.define("A", {
			awake = function()
				w.engine.setAttribute(r, "Speed", 5)
				B:get(r)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Y1 warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(br.bound[r], nil, "still invalid: never binds")
		spec.eq(warned, 0, "a still-invalid attribute is not pending")
	end)

	test("Y1b two invalid attributes, hook fixes both, then get", function()
		local w = World()
		local B, br = w.define("B", {}, { attributes = { speed = check("Speed", true), size = check("Size", true) } })
		local r = w.tagged("R", "B", { Speed = "fast", Size = "big" })
		local A = w.define("A", {
			awake = function()
				w.engine.setAttribute(r, "Speed", 5)
				w.engine.setAttribute(r, "Size", 2)
				B:get(r)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Y1b warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(br.bound[r] ~= nil, true, "binds after flush")
		spec.eq(warned, 1, "pending pair warns")
	end)

	test("Y2 wrong class, hook refreshes it, then get", function()
		local w = World()
		local B, br = w.define("B")
		local wrong = w.engine.instance("W", "Part")
		w.engine.addTag(wrong, "B")
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(wrong)
				B:get(wrong)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Y2 warnings", warned, "bound after flush", br.bound[wrong] ~= nil)
		spec.eq(br.bound[wrong], nil, "never binds")
		spec.eq(warned, 0, "wrong class is not pending after a refresh")
	end)

	test("Y2b default scope (ServerStorage), hook refreshes it, then get", function()
		local w = World()
		local B, br = w.define("B")
		local stored = w.engine.instance("S")
		stored.container = "ServerStorage"
		w.engine.addTag(stored, "B")
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(stored)
				B:get(stored)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Y2b warnings", warned, "bound after flush", br.bound[stored] ~= nil)
		spec.eq(br.bound[stored], nil, "never binds")
		spec.eq(warned, 0, "default scope is not pending after a refresh")
	end)

	test("Y2d invalid required attribute, hook refreshes it, then get", function()
		local w = World()
		local B, br = w.define("B", {}, { attributes = { speed = check("Speed", true) } })
		local r = w.tagged("R", "B", { Speed = "fast" })
		local A = w.define("A", {
			awake = function()
				w.behaviour:refresh(r)
				B:get(r)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Y2d warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(br.bound[r], nil, "never binds")
		spec.eq(warned, 0, "invalid attribute is not pending after a refresh")
	end)

	test("Y2c wrong class, signal delivered before an outside get whose hook gets it", function()
		local w = World()
		local B, br = w.define("B")
		local wrong = w.engine.instance("W", "Part")
		w.engine.addTag(wrong, "B")
		local A = w.define("A", {
			awake = function()
				B:get(wrong)
			end,
		})
		w.boot()
		local i = w.tagged("I", "A")
		A:get(i)
		local before = w.engine.logged("get pending")
		w.behaviour:refresh(wrong)
		print("Y2c warnings", before, "bound", br.bound[wrong] ~= nil)
		spec.eq(before, 0, "control")
	end)

	test("Y3 accepted limit: suppressed pair, hook removes and re-adds the tag, then get", function()
		local w = World()
		local B, br = w.define("B")
		local t = w.tagged("T", "B")
		local A = w.define("A", {
			awake = function()
				w.engine.removeTag(t, "B")
				w.engine.addTag(t, "B")
				B:get(t)
			end,
		})
		w.boot()
		B:get(t):remove()
		w.engine.flush()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Y3 warnings", warned, "bound after flush", br.bound[t] ~= nil)
		spec.eq(warned, 0, "suppressed is not pending (definition)")
		spec.eq(br.bound[t] ~= nil, true, "tag delivery releases suppression")
	end)

	for _, order in { "r first", "j first" } do
		test(
			"Y4 " .. order .. ": accepts-rejected pair admitted after a hook get, seen not live by another hook",
			function()
				local w = World()
				local allow = false
				local phase
				local B, br = w.define("B", {
					accepts = function()
						return allow
					end,
				})
				local r = w.tagged("R", "B")
				local C = w.define("C", {
					awake = function()
						phase = if br.bound[r] then br.bound[r].phase else "none"
						B:get(r)
					end,
				})
				local A = w.define("A", {
					awake = function()
						local j = w.engine.instance("J")
						w.engine.addTag(j, "C")
						if order == "j first" then
							C:get(j)
						end
						allow = true
						B:get(r)
						if order == "r first" then
							C:get(j)
						end
					end,
				})
				w.boot()
				A:get(w.tagged("I", "A"))
				w.engine.flush()
				local warned = w.engine.logged("get pending")
				local sites = {}
				for _, entry in w.engine.logs do
					if string.find(entry.message, "get pending", 1, true) then
						table.insert(sites, string.match(entry.message, ":(%d+)$"))
					end
				end
				print(
					"Y4",
					order,
					"warnings",
					warned,
					"sites",
					table.concat(sites, ","),
					"B phase seen by C",
					phase,
					"bound",
					br.bound[r] ~= nil
				)
				spec.eq(
					warned,
					if order == "j first" then 2 else 1,
					"C:get(j) is pending; B:get(r) in C awake is pending while B is not live"
				)
			end
		)
	end
end

do
	test("Z1 bound pair re-evaluated and rejected by user accepts, then a hook get", function()
		local w = World()
		local allow = true
		local B, br = w.define("B", {
			accepts = function()
				return allow
			end,
		})
		local r = w.tagged("R", "B")
		local A = w.define("A", {
			awake = function()
				B:get(r)
			end,
		})
		w.boot()
		w.engine.flush()
		local before = br.bound[r] ~= nil
		spec.eq(before, true, "pair starts bound")
		allow = false
		w.behaviour:refresh(r)
		w.engine.flush()
		local after = br.bound[r] ~= nil
		spec.eq(after, false, "reevaluation unbinds the pair")
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print(
			"Z1 bound before",
			before,
			"after refresh",
			after,
			"warnings",
			warned,
			"bound after flush",
			br.bound[r] ~= nil
		)
		spec.eq(br.bound[r], nil, "never rebinds")
		spec.eq(warned, 0, "user accepts rejection of a bound pair is not pending")
	end)

	test("Z2 bound pair moved into ServerStorage with default accepts, then a hook get", function()
		local w = World()
		local B, br = w.define("B")
		local r = w.tagged("R", "B")
		local A = w.define("A", {
			awake = function()
				B:get(r)
			end,
		})
		w.boot()
		w.engine.flush()
		local before = br.bound[r] ~= nil
		spec.eq(before, true, "pair starts bound")
		w.engine.move(r, "ServerStorage")
		w.engine.flush()
		local after = br.bound[r] ~= nil
		spec.eq(after, false, "reevaluation unbinds the pair")
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print(
			"Z2 bound before",
			before,
			"after move",
			after,
			"warnings",
			warned,
			"bound after flush",
			br.bound[r] ~= nil
		)
		spec.eq(br.bound[r], nil, "never rebinds")
		spec.eq(warned, 0, "default scope rejection of a bound pair is not pending")
	end)

	test("Z1b bound pair rejected by user accepts gets itself from its own destroy hook", function()
		local w = World()
		local allow = true
		local B, br
		B, br = w.define("B", {
			accepts = function()
				return allow
			end,
			onDestroy = function(self)
				B:get(self.instance)
			end,
		})
		local r = w.tagged("R", "B")
		w.boot()
		w.engine.flush()
		allow = false
		w.behaviour:refresh(r)
		w.engine.flush()
		local warned = w.engine.logged("get pending")
		print("Z1b warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(warned, 0, "a pair destroyed by user accepts is not pending")
	end)

	test("Z3 user accepts consults default scope on another instance, then rejects", function()
		local w = World()
		local other = w.engine.instance("O")
		other.container = "ServerStorage"
		local B, br
		B, br = w.define("B", {
			accepts = function(_, instance)
				return w.behaviour:accepts(other) and instance ~= nil
			end,
		})
		local r = w.tagged("R", "B")
		local A = w.define("A", {
			awake = function()
				B:get(r)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Z3 warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(br.bound[r], nil, "never binds")
		spec.eq(warned, 0, "user accepts rejection of an in-scope instance is not pending")
	end)

	test("Z4 user accepts calls default scope (true) then rejects", function()
		local w = World()
		local B, br = w.define("B", {
			accepts = function(_, instance)
				return w.behaviour:accepts(instance) and false
			end,
		})
		local r = w.tagged("R", "B")
		local A = w.define("A", {
			awake = function()
				B:get(r)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		print("Z4 warnings", warned, "bound after flush", br.bound[r] ~= nil)
		spec.eq(warned, 0, "user accepts rejection is not pending")
	end)

	test("Z5 scope rejection then an unrelated user accepts rejection in one world", function()
		local w = World()
		local _, br = w.define("B")
		local stored = w.engine.instance("S")
		stored.container = "ServerStorage"
		w.engine.addTag(stored, "B")
		local C, cr = w.define("C", {
			accepts = function()
				return false
			end,
		})
		local c = w.tagged("K", "C")
		local A = w.define("A", {
			awake = function()
				C:get(c)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		print("Z5 warnings", warned, "B bound", br.bound[stored] ~= nil, "C bound", cr.bound[c] ~= nil)
		spec.eq(warned, 0, "accepts rejection after a scope rejection is not pending")
	end)

	test("Z6 accepted behaviour: fresh wrong-class pair, tag signal undelivered, hook get", function()
		local w = World()
		local B = w.define("B")
		local A
		A = w.define("A", {
			awake = function()
				local wrong = w.engine.instance("W", "Part")
				w.engine.addTag(wrong, "B")
				B:get(wrong)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		spec.eq(warned, 1, "never evaluated means not yet rejected for class")
	end)

	test("Z7 accepted behaviour: fresh ServerStorage pair, tag signal undelivered, hook get", function()
		local w = World()
		local B = w.define("B")
		local A = w.define("A", {
			awake = function()
				local s = w.engine.instance("S")
				s.container = "ServerStorage"
				w.engine.addTag(s, "B")
				B:get(s)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		spec.eq(warned, 1, "never evaluated means not yet rejected for scope")
	end)

	test("Z8 attribute-rejected pair in ServerStorage, hook fixes attribute, then get", function()
		local w = World()
		local B, br = w.define("B", {}, {
			attributes = {
				speed = {
					name = "Speed",
					required = true,
					check = function(v)
						return type(v) == "number"
					end,
				},
			},
		})
		local r = w.tagged("R", "B", { Speed = "fast" })
		r.container = "ServerStorage"
		local A = w.define("A", {
			awake = function()
				w.engine.setAttribute(r, "Speed", 5)
				B:get(r)
			end,
		})
		w.boot()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		w.engine.flush()
		spec.eq(warned, 1, "corrected attributes are pending before scope evaluation")
		spec.eq(br.bound[r], nil, "scope still prevents admission")
	end)

	test("Z5b scope rejection at boot, later user accepts rejection, then a hook get", function()
		local w = World()
		local _, br = w.define("B")
		local stored = w.engine.instance("S")
		stored.container = "ServerStorage"
		w.engine.addTag(stored, "B")
		local C, cr = w.define("C", {
			accepts = function()
				return false
			end,
		})
		local c
		local A = w.define("A", {
			awake = function()
				C:get(c)
			end,
		})
		w.boot()
		w.engine.flush()
		c = w.tagged("K", "C")
		w.engine.flush()
		A:get(w.tagged("I", "A"))
		local warned = w.engine.logged("get pending")
		print("Z5b warnings", warned, "B bound", br.bound[stored] ~= nil, "C bound", cr.bound[c] ~= nil)
		spec.eq(warned, 0, "accepts rejection after an earlier scope rejection is not pending")
	end)
end

return tests
