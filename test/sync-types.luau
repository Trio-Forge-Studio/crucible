local process = require("@lune/process")
local spec = require("./lib/load")

return {
	["44 78 K9 the Crucible compiler build type-checks replicated declarations and rejects client writes"] = function()
		local result = process.exec("node", { "test/lib/replicated-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
}
