local fs = require("@lune/fs")
local process = require("@lune/process")
local roblox = require("@lune/roblox")

local input, runtime, output = process.args[1], process.args[2], process.args[3]
assert(input and runtime and output, "usage: place <place> <runtime out> <output place>")

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

local files = luauFiles(runtime, "", {})
local place = roblox.deserializePlace(fs.readFile(input))
local out = place:GetService("ReplicatedStorage").rbxts_include.node_modules["@trioforge"].crucible.out

local replaced = 0
local function swap(instance, path: string)
	if instance:IsA("ModuleScript") then
		local file = if path == ""
			then "init.luau"
			elseif files[`{path}.luau`] then `{path}.luau`
			else `{path}/init.luau`
		assert(files[file], `{runtime} has no module for {instance:GetFullName()}`)
		instance.Source = fs.readFile(`{runtime}/{file}`)
		files[file] = nil
		replaced += 1
	end
	for _, child in instance:GetChildren() do
		swap(child, if path == "" then child.Name else `{path}/{child.Name}`)
	end
end
swap(out, "")

local extra = {}
for file in files do
	table.insert(extra, file)
end
table.sort(extra)
assert(#extra == 0, `the place has no ModuleScript for {table.concat(extra, ", ")}`)

fs.writeFile(output, roblox.serializePlace(place))
print(`replaced {replaced} runtime modules`)
