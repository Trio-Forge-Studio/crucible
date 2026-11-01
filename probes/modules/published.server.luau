local Players = game:GetService("Players")
local Market = game:GetService("MarketplaceService")
local Config = game:GetService("ConfigService")
local Http = game:GetService("HttpService")

local PRODUCT = 3726289768
local mode = workspace:GetAttribute("ProbeMode") or "hold"
local holds = {}
for _, text in string.split(workspace:GetAttribute("ProbeHolds") or "5,60,120", ",") do
	table.insert(holds, tonumber(text))
end
local outdatedSeconds = workspace:GetAttribute("ProbeOutdatedSeconds") or 150

local function log(q, msg)
	print(string.format("G1|%.3f|%s|%s", os.clock(), q, msg))
end

log("env", `mode={mode} placeId={game.PlaceId} gameId={game.GameId} holds={table.concat(holds, ",")}`)

local ordinals = {}
local order = 0
local calls = {}
local returned = {}
local startedAt = {}

local function handler(receipt)
	local id = receipt.PurchaseId
	calls[id] = (calls[id] or 0) + 1
	if calls[id] == 1 then
		order += 1
		ordinals[id] = order
		startedAt[id] = os.clock()
	end
	local ordinal = ordinals[id]
	log(
		"5a",
		`callback ordinal={ordinal} callsForId={calls[id]} purchaseId={id} product={receipt.ProductId} player={receipt.PlayerId} players={#Players:GetPlayers()}`
	)
	if calls[id] > 1 then
		log("5a", `duplicate ordinal={ordinal} sinceFirst={string.format("%.3f", os.clock() - startedAt[id])}`)
		return Enum.ProductPurchaseDecision.NotProcessedYet
	end
	if mode == "leave" then
		log("5a", `returning NotProcessedYet ordinal={ordinal}`)
		return Enum.ProductPurchaseDecision.NotProcessedYet
	end
	if mode == "join" then
		log("5b", `granting at once ordinal={ordinal}`)
		returned[ordinal] = os.clock()
		return Enum.ProductPurchaseDecision.PurchaseGranted
	end
	local hold = holds[ordinal] or 0
	log("5a", `holding ordinal={ordinal} for {hold}s`)
	task.wait(hold)
	log("5a", `returning PurchaseGranted ordinal={ordinal} heldFor={string.format("%.3f", os.clock() - startedAt[id])}`)
	returned[ordinal] = os.clock()
	return Enum.ProductPurchaseDecision.PurchaseGranted
end

local assignAfter = workspace:GetAttribute("ProbeProcessAfter") or 0
local function install()
	Market.ProcessReceipt = handler
	log("5b", "ProcessReceipt assigned")
end
if mode == "join" and assignAfter > 0 then
	task.delay(assignAfter, install)
else
	install()
end

Market.PromptProductPurchaseFinished:Connect(function(userId, productId, isPurchased)
	log("5a", `PromptProductPurchaseFinished user={userId} product={productId} purchased={tostring(isPurchased)}`)
end)

local function added(player)
	log("5b", `PlayerAdded {player.Name} userId={player.UserId}`)
	if mode == "join" then
		return
	end
	task.spawn(function()
		task.wait(3)
		for index = 1, mode == "leave" and 1 or #holds do
			log("5a", `prompting ordinal={index}`)
			local promptedAt = os.clock()
			Market:PromptProductPurchase(player, PRODUCT)
			local deadline = os.clock() + (holds[index] or 0) + 120
			while not returned[index] and mode ~= "leave" and os.clock() < deadline do
				task.wait(0.25)
			end
			if mode == "leave" then
				task.wait(20)
			else
				log(
					"5a",
					`ordinal={index} promptToReturn={returned[index] and string.format(
						"%.3f",
						returned[index] - promptedAt
					) or "never"}`
				)
				task.wait(20)
			end
		end
		log("5a", "purchase sequence complete")
	end)
end
Players.PlayerAdded:Connect(added)
for _, player in Players:GetPlayers() do
	added(player)
end

task.spawn(function()
	local ok, snapshot = pcall(function()
		return Config:GetConfigAsync()
	end)
	log("6fetch", `ok={tostring(ok)} result={tostring(snapshot)}`)
	if not ok then
		return
	end
	local value = snapshot:GetValue("GlobalEvent")
	local encoded = typeof(value) == "table" and Http:JSONEncode(value) or tostring(value)
	log("6a", `GlobalEvent typeof={typeof(value)} type={type(value)} value={encoded}`)
	if typeof(value) == "table" then
		log("6a", `GlobalEvent.multiplier typeof={typeof(value.multiplier)} value={tostring(value.multiplier)}`)
	end
	local missingOk, missing = pcall(function()
		return snapshot:GetValue("NoSuchKey")
	end)
	log("6a", `NoSuchKey ok={tostring(missingOk)} typeof={typeof(missing)} value={tostring(missing)}`)
	log("6b", `initial Outdated={tostring(snapshot.Outdated)}`)
	snapshot.UpdateAvailable:Connect(function()
		log("6b", `UpdateAvailable fired Outdated={tostring(snapshot.Outdated)}`)
	end)
	local last = snapshot.Outdated
	local deadline = os.clock() + outdatedSeconds
	while os.clock() < deadline do
		task.wait(1)
		if snapshot.Outdated ~= last then
			last = snapshot.Outdated
			log("6b", `Outdated changed to {tostring(last)} without a write`)
		end
	end
	log("6b", `idle {outdatedSeconds}s Outdated={tostring(snapshot.Outdated)}`)
	local fresh = Config:GetConfigAsync()
	log("6b", `fresh GetConfigAsync sameObject={tostring(fresh == snapshot)} Outdated={tostring(fresh.Outdated)}`)
	local setOk, setErr = pcall(function()
		Config:SetTestingValue("GlobalEvent", '{"multiplier":3,"endsAt":0}')
	end)
	log("6b", `SetTestingValue ok={tostring(setOk)} result={tostring(setErr)}`)
	log("6b", `after SetTestingValue Outdated={tostring(snapshot.Outdated)} fresh.Outdated={tostring(fresh.Outdated)}`)
	task.wait(5)
	log("6b", `5s later Outdated={tostring(snapshot.Outdated)}`)
	local newer = Config:GetConfigAsync()
	local newValue = newer:GetValue("GlobalEvent")
	log(
		"6b",
		`GetConfigAsync after write sameObject={tostring(newer == snapshot)} typeof={typeof(newValue)} value={typeof(
			newValue
		) == "table" and Http:JSONEncode(newValue) or tostring(newValue)}`
	)
	log("6b", `old snapshot still reads typeof={typeof(snapshot:GetValue("GlobalEvent"))}`)
	local refreshOk, refreshErr = pcall(function()
		snapshot:Refresh()
	end)
	log("6b", `Refresh ok={tostring(refreshOk)} result={tostring(refreshErr)} Outdated={tostring(snapshot.Outdated)}`)
	local after = snapshot:GetValue("GlobalEvent")
	log("6b", `after Refresh value={typeof(after) == "table" and Http:JSONEncode(after) or tostring(after)}`)
	log("6done", "config probe complete")
end)

log("ready", "published-place instrument")
