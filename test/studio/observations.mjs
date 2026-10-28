export const server = String.raw`
local main = game.ServerScriptService.TS.main
while not main:GetAttribute("CrucibleServer") do task.wait() end
local Member = require(main.Parent.systems.member).Member
local tags = game:GetService("CollectionService")
local part = Instance.new("Part")
part.Name = "S2ObservedMember"
part.Anchored = true
part:SetAttribute("Identity", "studio-83")
part.Parent = workspace
tags:AddTag(part, "S2Member")
task.wait()
local first = Member:get(part)
assert(first and first.identity == "studio-83" and first.speed == 17)
print("S2 observed tag added bound identity=studio-83 speed=17")
tags:RemoveTag(part, "S2Member")
task.wait()
assert(Member:get(part) == nil)
print("S2 observed tag removed unbound")
tags:AddTag(part, "S2Member")
task.wait()
local second = Member:get(part)
assert(second and second ~= first)
print("S2 observed tag readded new generation")
part.Parent = nil
task.wait()
assert(Member:get(part) == nil and tags:HasTag(part, "S2Member"))
print("S2 observed Parent=nil unbound tag retained")
part.Parent = workspace
task.wait()
assert(Member:get(part) and Member:get(part) ~= second)
print("S2 observed reparent bound new generation")
part:Destroy()
print("S2 server observations complete")
`;

export const client = String.raw`
local player = game:GetService("Players").LocalPlayer
while not player:GetAttribute("CrucibleBooted") do task.wait() end
assert(player:GetAttribute("CrucibleBooted") == true)
print("S2 observed BOOTED_ATTRIBUTE=true on LocalPlayer")
local Renderer = require(game.StarterPlayer.StarterPlayerScripts.TS.systems.renderer).Renderer
local original = Renderer.preRender
local run = game:GetService("RunService")
local sequence = {}
local frames = 0
Renderer.preRender = function(self, dt)
    table.insert(sequence, "Crucible201")
    original(self, dt)
end
run:BindToRenderStep("s2-observer-200", 200, function()
    table.insert(sequence, "Bind200")
end)
local connection
connection = run.PreRender:Connect(function()
    table.insert(sequence, "PreRender")
    frames += 1
    if frames <= 5 then
        print("S2 observed render order " .. table.concat(sequence, ","))
        assert(table.concat(sequence, ",") == "Bind200,Crucible201,PreRender")
    end
    table.clear(sequence)
    if frames == 5 then
        Renderer.preRender = function() error("S2 intentional frame fault") end
    elseif frames == 8 then
        connection:Disconnect()
        run:UnbindFromRenderStep("s2-observer-200")
        print("S2 client observations complete")
    end
end)
`;
