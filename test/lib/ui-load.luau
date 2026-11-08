-- Loads the compiled `ui` entry from packages/crucible/out/ui the way a place requires it. Returns a require for module
-- names under that folder that shares one module cache, so every module sees the same state. Each call is a new realm.
-- Pass `packages` to replace a package such as @rbxts/vide, `services` for game:GetService, and `globals` for engine
-- globals. Charm loads from the runtime's node_modules unless a spec replaces it.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local task = require("@lune/task")

local ROOT = "packages/crucible/out/ui"
local CHARM = "packages/crucible/node_modules/@rbxts/charm/src"

export type Options = {
	packages: { [string]: any }?,
	services: { [string]: any }?,
	globals: { [string]: any }?,
}

-- Lune's fs calls yield, and Vide and Charm fail a yield in strict mode, so every file is read before a spec loads one.
local sources: { [string]: string } = {}
for _, folder in { ROOT, CHARM } do
	for _, name in fs.readDir(folder) do
		if string.match(name, "%.luau$") then
			sources[`{folder}/{name}`] = fs.readFile(`{folder}/{name}`)
		end
	end
end

local function source(path: string): string
	return sources[`{path}.luau`]
		or sources[`{path}/init.luau`]
		or error(`no compiled module at {path}; run pnpm build`)
end

local DEFAULT_RUN_SERVICE = {
	IsStudio = function()
		return false
	end,
}

return function(options: Options?): (...string) -> any
	local given: Options = options or {}
	local globals = given.globals or {}
	local services = given.services or {}
	local game = {
		GetService = function(_self, name: string)
			if name == "RunService" then
				return services.RunService or DEFAULT_RUN_SERVICE
			end
			return services[name] or error(`The package asked for the {name} service`)
		end,
	}

	-- Charm requires its own files by `@self/name`.
	local charmModules: { [string]: any } = {}
	local function loadCharm(path: string): any
		if charmModules[path] == nil then
			charmModules[path] = luau.load(source(path), {
				debugName = path,
				environment = {
					game = game,
					task = task,
					require = function(target: string)
						local child = assert(string.match(target, "^@self/(.+)$"), `unsupported require {target}`)
						return loadCharm(`{CHARM}/{child}`)
					end,
				},
			})()
		end
		return charmModules[path]
	end
	local packages = table.clone(given.packages or {})
	if packages["@rbxts/charm"] == nil then
		packages["@rbxts/charm"] = loadCharm(CHARM)
	end

	local cache: { [string]: any } = {}
	local load: (path: string) -> any
	local TS = {}
	function TS.getModule(_caller, scope: string, name: string)
		local package = `{scope}/{name}`
		assert(packages[package] ~= nil, `the ui entry imports {package}, which the spec did not provide`)
		return { src = { package = package } }
	end
	function TS.import(_caller, base, ...: string)
		if base.package then
			return packages[base.package]
		end
		local path = base.path
		for _, name in { ... } do
			path ..= "/" .. name
		end
		return load(path)
	end
	local realm = setmetatable({}, {
		__index = function()
			return TS
		end,
	})
	function load(path: string): any
		if cache[path] == nil then
			local environment: { [string]: any } = table.clone(globals)
			environment._G = realm
			environment.game = game
			environment.task = globals.task or task
			environment.script = { path = path, Parent = { path = string.match(path, "^(.*)/[^/]+$") } }
			cache[path] = luau.load(source(path), { debugName = path, environment = environment })()
		end
		return cache[path]
	end

	return function(...: string): any
		return TS.import(nil, { path = ROOT }, ...)
	end
end
