-- Loads compiled modules in Lune as roblox-ts's runtime library would. A module's `script` is a node named by its
-- path, `TS.import` follows nodes to files, and `TS.getModule` finds a package's folder in `packages`. Every module
-- sees `globals`, and a package module may also `require` a node or a `./` path. Each graph loads its modules once.
local fs = require("@lune/fs")
local luau = require("@lune/luau")

local function fileOf(path: string): string
	if fs.isFile(`{path}.luau`) then
		return `{path}.luau`
	elseif fs.isFile(`{path}/init.luau`) then
		return `{path}/init.luau`
	end
	error(`no module at {path}`)
end

return function(packages: { [string]: string }, globals: { [string]: any })
	local nodes = {}
	local function node(path: string)
		if nodes[path] == nil then
			nodes[path] = setmetatable({ path = path }, {
				__index = function(self, key)
					if key == "Parent" then
						return node(string.match(self.path, "^(.*)/[^/]+$") or "")
					end
					return node(`{self.path}/{key}`)
				end,
			})
		end
		return nodes[path]
	end

	local loaded = {}
	local TS = {}
	local G = setmetatable({}, {
		__index = function()
			return TS
		end,
	})
	local function load(path: string)
		if loaded[path] == nil then
			local file = fileOf(path)
			local environment = { script = node(path), _G = G }
			for key, value in globals do
				environment[key] = value
			end
			function environment.require(target)
				if type(target) == "string" then
					local relative = assert(string.match(target, "^%./(.+)$"), `unsupported require {target}`)
					return load(`{string.match(file, "^(.*)/[^/]+$")}/{relative}`)
				end
				return load(target.path)
			end
			local chunk = luau.load(fs.readFile(file), { debugName = path, environment = environment })
			loaded[path] = { value = chunk() }
		end
		return loaded[path].value
	end

	function TS.import(_caller, base, ...)
		local path = base.path
		for _, name in { ... } do
			path ..= "/" .. name
		end
		return load(path)
	end

	function TS.getModule(_caller, scope: string, name: string)
		return node(assert(packages[`{scope}/{name}`], `no package {scope}/{name}`))
	end

	return { load = load, node = node }
end
