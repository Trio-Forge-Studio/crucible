-- Runs the Vide JSX fixture's emitted Luau against Vide 0.6.1 on the fake Instance.
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local fake = require("./instance")
local graph = require("./load")

local out, vide = process.args[1], process.args[2]
assert(out and vide, "usage: vide.luau <out> <@rbxts/vide folder>")

local heartbeat = { Connect = function() end }
local classes = {}
for _, name in { "Frame", "TextLabel", "TextButton", "UICorner", "UIPadding" } do
	table.insert(classes, { Name = name })
end
local game = {
	GetService = function(_, service: string)
		if service == "RunService" then
			return { Heartbeat = heartbeat }
		end
		return {
			GetClasses = function()
				return classes
			end,
		}
	end,
}
local modules = graph({ ["@rbxts/vide"] = vide }, {
	game = game,
	Instance = fake.Instance,
	typeof = fake.typeof,
	Enum = roblox.Enum,
	Color3 = roblox.Color3,
})
local Vide = modules.load(`{vide}/src`)
Vide.strict = false
local panel = modules.load(`{out}/panel`)

local failures = 0
local function eq(label: string, actual: any, expected: any)
	if actual ~= expected then
		failures += 1
		print(`FAIL {label}: expected {tostring(expected)}, got {tostring(actual)}`)
	end
end
local function list(label: string, actual: { any }, expected: { any })
	eq(`{label} count`, #actual, #expected)
	for i = 1, math.max(#expected, #actual) do
		eq(`{label}[{i}]`, actual[i], expected[i])
	end
end

local title = Vide.source("Title")
local open = Vide.source(false)
local items = Vide.source({ "a", "b" })
local closed = 0
local _, root = Vide.root(function()
	return panel.Panel({
		title = title,
		open = open,
		items = items,
		onClose = function()
			closed += 1
		end,
	})
end)

eq("root class", root.ClassName, "Frame")
eq("root name", root.Name, "Panel")
eq("a bare attribute is true", root.Visible, true)
list("labels", fake.texts(root, "Label"), { "Title", "a", "b" })
list("label orders", fake.texts(root, "Label", "LayoutOrder"), { 1, 11, 12 })
local button = fake.named(root, "Close")[1]
eq("a spread attribute names the button", button and button.ClassName, "TextButton")
eq("a spread attribute sets its text", button and button.Text, "x")
eq("the fragment's children reach the frame", #fake.named(root, "Corner") + #fake.named(root, "Padding"), 2)

title("Renamed")
eq("a function prop follows its source", fake.named(root, "Label")[1].Text, "Renamed")

fake.fire(button, "Activated")
eq("an event prop connects its handler", closed, 1)

open(true)
list("Show mounts its branch", fake.texts(root, "Label"), { "Renamed", "open", "a", "b" })
open(false)
list("Show unmounts its branch", fake.texts(root, "Label"), { "Renamed", "a", "b" })

items({ "c", "a" })
list("For follows its list", fake.texts(root, "Label"), { "Renamed", "c", "a" })

if failures > 0 then
	process.exit(1)
end
print("vide fixtures pass")
