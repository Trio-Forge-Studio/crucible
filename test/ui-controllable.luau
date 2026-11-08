local harness = require("./lib/ui-harness")
return {
	["uncontrolled state starts at the default and reports each change once"] = function()
		local h = harness()
		local reported = {}
		local get, set = table.unpack(h.require("use-controllable").useControllable({
			defaultValue = 1,
			onChange = function(next)
				table.insert(reported, next)
			end,
		}))
		assert(get() == 1, "starts at the default")
		set(2)
		set(2)
		assert(get() == 2 and #reported == 1 and reported[1] == 2, "equal values are skipped")
		h.stop()
	end,
	["controlled state reads the value and only reports through onChange"] = function()
		local h = harness()
		local value = h.vide.source("a")
		local reported = {}
		local get, set = table.unpack(h.require("use-controllable").useControllable({
			value = value,
			defaultValue = "ignored",
			onChange = function(next)
				table.insert(reported, next)
			end,
		}))
		assert(get() == "a", "the controlled value wins over the default")
		set("b")
		assert(get() == "a" and reported[1] == "b", "the owner decides whether the value changes")
		set("a")
		assert(#reported == 1, "setting the current value is skipped")
		value("c")
		assert(get() == "c" and #reported == 1, "outside writes do not report")
		h.stop()
	end,
}
