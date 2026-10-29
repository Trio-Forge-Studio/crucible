local RunService = game:GetService("RunService")
local sequence = 0
local origin = os.clock()
local function log(id, message)
	sequence += 1
	print(string.format("S1|client|%04d|%.6f|%s|%s", sequence, os.clock() - origin, id, message))
end
log("metadata", "version=" .. version() .. " behavior=" .. tostring(workspace:GetAttribute("ProbeSignalBehavior")))
local order = {}
local frame = 0
local function record(name)
	table.insert(order, name)
end
RunService.PreRender:Connect(function()
	record("PreRender-before")
end)
RunService.RenderStepped:Connect(function()
	record("RenderStepped")
end)
for _, priority in { 0, 100, 199, 200, 201, 2000 } do
	RunService:BindToRenderStep("S1_" .. priority, priority, function()
		record("Bind" .. priority)
	end)
end
for _, name in { "A", "B" } do
	RunService:BindToRenderStep("S1_201_" .. name, 201, function()
		record("Bind201" .. name)
	end)
end
RunService.PreRender:Connect(function()
	record("PreRender-after")
end)
for _, name in { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	RunService[name]:Connect(function()
		record(name)
		if name == "Heartbeat" then
			if table.find(order, "Bind0") then
				frame += 1
			end
			if frame > 0 and frame <= 35 and table.find(order, "Bind0") then
				log("3.client", "frame=" .. frame .. " " .. table.concat(order, ","))
			end
			table.clear(order)
		end
	end)
end
local result = table.pack(pcall(function()
	game:BindToClose(function()
		log("5.client", "callback ran")
	end)
end))
for index = 1, result.n do
	log("5.client", "result[" .. index .. "]=" .. tostring(result[index]))
end
