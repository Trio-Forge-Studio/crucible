local process = require("@lune/process")
local spec = require("./lib/load")

return {
	["1 8 K9 the Crucible compiler build type-checks a profile store and rejects a bad template and bad writes"] = function()
		local result = process.exec("node", { "test/lib/profile-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
}
