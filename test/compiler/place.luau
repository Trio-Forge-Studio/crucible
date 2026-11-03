local fs = require("@lune/fs")
local process = require("@lune/process")
local roblox = require("@lune/roblox")

local input, runtime, game, output = process.args[1], process.args[2], process.args[3], process.args[4]
assert(input and runtime and game and output, "usage: place <place> <runtime out> <game out> <output place>")

local function luauFiles(dir: string, prefix: string, found: { [string]: boolean })
	for _, name in fs.readDir(dir) do
		local path = `{dir}/{name}`
		if fs.isDir(path) then
			luauFiles(path, `{prefix}{name}/`, found)
		elseif string.match(name, "%.luau$") then
			found[prefix .. name] = true
		end
	end
	return found
end

local SUFFIX = { ModuleScript = "", Script = ".server", LocalScript = ".client" }

-- Replaces the source of every script under `root` with the file at its path in `dir`, and fails if either side
-- has a script the other lacks.
local function swap(root, dir: string): number
	local files = luauFiles(dir, "", {})
	local replaced = 0
	local function visit(instance, path: string)
		local suffix = SUFFIX[instance.ClassName]
		if suffix then
			local file = if path == ""
				then `init{suffix}.luau`
				elseif files[`{path}{suffix}.luau`] then `{path}{suffix}.luau`
				else `{path}/init{suffix}.luau`
			assert(files[file], `{dir} has no module for {instance:GetFullName()}`)
			instance.Source = fs.readFile(`{dir}/{file}`)
			files[file] = nil
			replaced += 1
		end
		for _, child in instance:GetChildren() do
			visit(child, if path == "" then child.Name else `{path}/{child.Name}`)
		end
	end
	visit(root, "")
	local extra = {}
	for file in files do
		table.insert(extra, file)
	end
	table.sort(extra)
	assert(#extra == 0, `the place has no script for {dir}/{table.concat(extra, ", ")}`)
	return replaced
end

local place = roblox.deserializePlace(fs.readFile(input))
local out = place:GetService("ReplicatedStorage").rbxts_include.node_modules["@trioforge"].crucible.out
local modules = swap(out, runtime)
local scripts = swap(place:GetService("ServerScriptService").TS, `{game}/server`)
	+ swap(place:GetService("StarterPlayer").StarterPlayerScripts.TS, `{game}/client`)

fs.writeFile(output, roblox.serializePlace(place))
print(`replaced {modules} runtime modules and {scripts} sample game scripts`)
