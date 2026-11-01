local process = require("@lune/process")
local spec = require("./lib/load")

return {
	["6 16 25 the declared API accepts every contract form and rejects each wrong direction payload and callback"] = function()
		local result = process.exec("node", { "test/lib/network-api-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
	["25 K2 K9 the transformed rbxtsc build type-checks every receive callback against its target"] = function()
		local result = process.exec("node", { "test/lib/network-receive-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
}
