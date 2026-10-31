local Run = game:GetService("RunService")
local RS = game:GetService("ReplicatedStorage")
local frame = 0
local function log(q, msg)
	print(string.format("S0|client|%.6f|f%d|%s|%s", os.clock(), frame, q, msg))
end
local remote = RS:WaitForChild("ProbeRemote")
log("2a", "WaitForChild published=" .. tostring(remote:GetAttribute("Published")))
for _, size in { 867, 66559 } do
	local value = remote:GetAttribute("Catalog" .. size)
	log(
		"2b",
		"size="
			.. size
			.. " type="
			.. typeof(value)
			.. " length="
			.. (if type(value) == "string" then #value else -1)
			.. " intact="
			.. tostring(value == string.rep(if size == 867 then "a" else "b", size))
	)
end
remote.OnClientEvent:Connect(function(kind, payload)
	if kind == "far" then
		log(
			"1a",
			"local-far="
				.. tostring(workspace:FindFirstChild("FarPart"))
				.. " part="
				.. tostring(payload.part)
				.. " keep="
				.. tostring(payload.keep)
				.. " number="
				.. tostring(payload.number)
		)
	elseif kind == "fail" then
		log("3kick", "FAIL received " .. payload)
	elseif kind == "left" then
		log("1c", "player=" .. tostring(payload.player) .. " keep=" .. tostring(payload.keep))
	end
end)
remote:FireServer("ready")
local part = Instance.new("Part")
part.Name = "ClientOnly"
part.Parent = workspace
remote:FireServer("local-part", { part = part, keep = "intact" })
for index = 1, 200 do
	remote:FireServer("load", index)
end
local signal = Instance.new("BindableEvent")
signal.Event:Connect(function()
	log("4", "signal")
	task.defer(function()
		log("4", "defer-signal")
	end)
end)
for _, phase in { "PreRender", "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	Run[phase]:Connect(function()
		if phase == "Heartbeat" then
			frame += 1
		end
		if frame >= 1 and frame <= 35 then
			log("4", phase)
			if phase == "Heartbeat" or phase == "PreRender" then
				task.defer(function()
					log("4", "defer-" .. phase)
				end)
			end
			if phase == "Heartbeat" then
				signal:Fire()
				remote:FireServer("tick", frame)
			end
		end
		if phase == "Heartbeat" and frame == 36 then
			log("complete", "35 intervals")
		end
	end)
end
