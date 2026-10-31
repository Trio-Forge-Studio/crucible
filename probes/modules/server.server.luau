local Players = game:GetService("Players")
local RS = game:GetService("ReplicatedStorage")
local Run = game:GetService("RunService")
local frame = 0
local function log(q, msg)
	print(string.format("S0|server|%.6f|f%d|%s|%s", os.clock(), frame, q, msg))
end
local remote = Instance.new("RemoteEvent")
remote.Name = "ProbeRemote"
remote:SetAttribute("Published", true)
remote:SetAttribute("Catalog867", string.rep("a", 867))
remote:SetAttribute("Catalog66559", string.rep("b", 66559))
remote.Parent = RS
local far = Instance.new("Part")
far.Name = "FarPart"
far.Anchored = true
far.Position = Vector3.new(100000, 0, 100000)
far.Parent = workspace
local signal = Instance.new("BindableEvent")
signal.Event:Connect(function()
	if frame <= 35 then
		log("4", "signal")
		task.defer(function()
			log("4", "defer-signal")
		end)
	end
end)
for _, phase in { "PreSimulation", "PostSimulation", "Heartbeat" } do
	Run[phase]:Connect(function()
		if phase == "Heartbeat" then
			frame += 1
		end
		if frame <= 35 then
			log("4", phase)
			if phase == "Heartbeat" then
				task.defer(function()
					log("4", "defer-heartbeat")
				end)
				signal:Fire()
			end
		end
	end)
end
Players.PlayerRemoving:Connect(function(player)
	log("3kick", "PlayerRemoving " .. player.Name)
	remote:FireAllClients("left", { player = player, keep = "intact" })
	log("1c", "FireAllClients from PlayerRemoving Parent=" .. tostring(player.Parent))
end)
local function added(player)
	log("5b", "PlayerAdded " .. player.Name)
	if workspace:GetAttribute("KickMode") == "startup" then
		remote:FireClient(player, "fail", "startup")
		log("3kick", "startup before Kick")
		player:Kick("S0 startup kick")
		log("3kick", "startup after Kick")
	end
end
Players.PlayerAdded:Connect(added)
for _, player in Players:GetPlayers() do
	added(player)
end
remote.OnServerEvent:Connect(function(player, kind, payload)
	if kind == "ready" then
		log("1a", "send far=" .. far:GetFullName())
		local ok, err = pcall(function()
			remote:FireClient(player, "far", { part = far, keep = "intact", number = 42 })
		end)
		log("1a", "FireClient ok=" .. tostring(ok) .. " error=" .. tostring(err))
	elseif kind == "local-part" then
		log("1b", "part=" .. tostring(payload.part) .. " keep=" .. tostring(payload.keep))
	elseif kind == "load" then
		log("3order", "start=" .. payload)
		if payload == 1 then
			task.wait(1)
		end
		log("3order", "end=" .. payload)
	elseif kind == "tick" then
		log("4remote", "handler=" .. payload)
		task.defer(function()
			log("4remote", "defer=" .. payload)
		end)
	elseif kind == "kick" then
		remote:FireClient(player, "fail", "handler")
		log("3kick", "handler before Kick")
		player:Kick("S0 handler kick")
		log("3kick", "handler after Kick")
		task.wait(2)
		log("3kick", "handler return")
	end
end)
local configComplete = false
task.delay(60, function()
	if not configComplete then
		log("6timeout", "GetConfigAsync has not returned after 60 seconds in unpublished place")
	end
end)
task.spawn(function()
	local config = game:GetService("ConfigService")
	for key, value in { S0String = "hello", S0Json = '{"answer":42}', S0Table = { answer = 42 }, ["S0.Dotted"] = 42 } do
		local ok, err = pcall(function()
			config:SetTestingValue(key, value)
		end)
		log("6set", key .. " ok=" .. tostring(ok) .. " result=" .. tostring(err))
	end
	local ok, snapshot = pcall(function()
		return config:GetConfigAsync()
	end)
	configComplete = true
	log("6fetch", "ok=" .. tostring(ok) .. " result=" .. tostring(snapshot))
	if ok then
		for _, key in { "S0String", "S0Json", "S0Table", "S0.Dotted" } do
			local value = snapshot:GetValue(key)
			log(
				"6get",
				key .. " type=" .. typeof(value) .. " value=" .. game:GetService("HttpService"):JSONEncode(value)
			)
		end
		log("6outdated", "before=" .. tostring(snapshot.Outdated))
		snapshot.UpdateAvailable:Connect(function()
			log("6outdated", "UpdateAvailable=" .. tostring(snapshot.Outdated))
		end)
		config:SetTestingValue("S0String", "changed")
		log("6outdated", "after-write=" .. tostring(snapshot.Outdated))
		task.wait(2)
		log("6outdated", "after-wait=" .. tostring(snapshot.Outdated))
		snapshot:Refresh()
		log(
			"6outdated",
			"after-refresh=" .. tostring(snapshot.Outdated) .. " value=" .. tostring(snapshot:GetValue("S0String"))
		)
	end
end)
task.spawn(function()
	local market = game:GetService("MarketplaceService")
	market.ProcessReceipt = function(receipt)
		log("5receipt", "callback " .. tostring(receipt.PurchaseId))
		return Enum.ProductPurchaseDecision.NotProcessedYet
	end
	local ok, result = pcall(function()
		return market:GetDeveloperProductsAsync():GetCurrentPage()
	end)
	log(
		"5products",
		"gameId="
			.. game.GameId
			.. " placeId="
			.. game.PlaceId
			.. " ok="
			.. tostring(ok)
			.. " result="
			.. tostring(result)
	)
end)
log("ready", "Deferred streaming instrument")
