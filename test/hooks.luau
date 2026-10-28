local spec = require("./lib/load")
local fs = require("@lune/fs")

local function sorted(list)
	local copy = table.clone(list)
	table.sort(copy)
	return copy
end
local function methods(class)
	local names = {}
	for key, value in class do
		if type(value) == "function" and key ~= "constructor" then
			table.insert(names, key)
		end
	end
	return sorted(names)
end
return {
	["a roblox-ts consumer resolves the physical emit folder and compiles dot calls"] = function()
		local result = require("@lune/process").exec("node", { "test/lib/emit-consumer.mjs" })
		spec.eq(result.code, 0, result.stdout .. result.stderr)
	end,
	["the runtime hook names match protected declarations including onShutdown"] = function()
		local graph = spec.runtime()
		local hooks = graph.module("internal/hooks")
		local object = graph.module("crucible-object").CrucibleObject
		local subsystem = graph.module("subsystem").Subsystem
		spec.list(methods(object), sorted(hooks.OBJECT_HOOK_NAMES), "object hooks")
		spec.list(methods(subsystem), sorted(hooks.SUBSYSTEM_HOOK_NAMES), "subsystem hooks")
		spec.list(
			hooks.BEHAVIOUR_HOOK_NAMES,
			{ "onEnable", "onDisable", "onDestroy" },
			"behaviour contract owned by R3"
		)
		local declared = {}
		for _, path in { "crucible-object", "subsystem" } do
			for name in string.gmatch(fs.readFile(`packages/crucible/out/{path}.d.ts`), "protected ([%w]+)%(") do
				table.insert(declared, name)
			end
		end
		local runtime = table.clone(hooks.OBJECT_HOOK_NAMES)
		for _, name in hooks.SUBSYSTEM_HOOK_NAMES do
			table.insert(runtime, name)
		end
		spec.list(sorted(declared), sorted(runtime), "protected declaration contract")
		for _, name in hooks.FRAME_HOOK_NAMES do
			spec.eq(table.find(hooks.OBJECT_HOOK_NAMES, name) ~= nil, true, name)
		end
		for _, name in hooks.BEHAVIOUR_HOOK_NAMES do
			spec.eq(subsystem[name], nil, "no behaviour hook on Subsystem")
		end
	end,
	["overrides finds direct and inherited frame hooks on compiled class tables"] = function()
		local graph = spec.runtime()
		local object = graph.module("crucible-object").CrucibleObject
		local overrides = graph.module("internal/hooks").hooks().overrides
		local base = setmetatable({ update = function() end, preRender = function() end }, { __index = object })
		local leaf = setmetatable({}, { __index = base })
		local plain = setmetatable({ awake = function() end }, { __index = object })
		spec.eq(overrides(base), 5, "direct")
		spec.eq(overrides(leaf), 5, "inherited")
		spec.eq(overrides(plain), 0, "non-frame")
		spec.eq(overrides(object), 0, "defaults")
	end,
	["internal declarations are empty and the root exports CrucibleObject as a type only"] = function()
		for _, name in fs.readDir("packages/crucible/out/internal") do
			if string.match(name, "%.d%.ts$") then
				local text = fs.readFile(`packages/crucible/out/internal/{name}`)
				spec.eq(string.gsub(text, "%s", ""), "export{};", name)
			end
		end
		local graph = spec.runtime()
		local root = graph.module("")
		spec.eq(root.CrucibleObject, nil, "type only")
		spec.eq(type(root.Subsystem), "table", "public Subsystem")
		spec.eq(type(root.inject), "function", "public inject")
		spec.eq(type(root.attribute), "function", "public attribute")
	end,
}
