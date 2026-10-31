local process = require("@lune/process")
local spec = require("./lib/load")

return {
	["57 Lowering shapes recheck through the physical emit entry"] = function()
		local result = process.exec("node", { "test/lib/network-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
}
