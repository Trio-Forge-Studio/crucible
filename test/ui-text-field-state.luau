local M = require("./lib/ui-load")()("text-field-state")
return {
	["text completion preserves typed text and appends only the remaining suffix"] = function()
		assert(M.completionText("hel", "hello") == "hello", "completion suffix")
		assert(M.completionText("HE", "hello") == "HEllo", "typed case survives")
		assert(M.completionText("longer", "hi") == "longer", "short completion adds nothing")
		assert(M.completionText("hello", nil) == nil, "missing completion stays absent")
		assert(M.limitText("hello", 3) == "hel", "input and submission share limit")
		assert(M.limitText("hello", 0) == "", "zero length")
		assert(M.limitText("hello", nil) == "hello", "unlimited text")
	end,
	["text limits reject negative fractional and nonfinite lengths"] = function()
		M.validateMaxLength(nil)
		M.validateMaxLength(0)
		for _, value in { -1, 1.5, math.huge, 0 / 0 } do
			assert(not pcall(M.validateMaxLength, value), "invalid length rejected")
		end
	end,
}
