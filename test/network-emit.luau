local fs = require("@lune/fs")
local luau = require("@lune/luau")
local process = require("@lune/process")
local spec = require("./lib/load")
local h = require("./lib/network-codec")

return {
	["57 every Lowering shape type-checks through the physical emit entry and rejected shapes do not"] = function()
		local result = process.exec("node", { "test/lib/network-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
	["57 every Lowering shape constructs and compiles with the real n"] = function()
		local source = fs.readFile("test/lib/network-descriptors.ts")
		local list = assert(string.match(source, "export const descriptors = (%b[]);"), "descriptor list")
		local expected = select(2, string.gsub(list, "\n\t__crucible", ""))
		local r = h.realm()
		local text = string.gsub(string.gsub(string.gsub(list, "__crucible%.n", "n"), "%[", "{"), "%]", "}")
		text = string.gsub(string.gsub(text, "n%.nil", 'n["nil"]'), ",(%s*%))", "%1")
		local descriptors = luau.load(`return {text}`, { environment = { n = r.n } })()
		spec.eq(#descriptors, expected, "every emitted shape evaluated")
		for i, node in descriptors do
			local ok, failure = pcall(r.codec.compile, node)
			spec.eq(ok, true, `shape {i} compiles: {failure}`)
		end
	end,
}
