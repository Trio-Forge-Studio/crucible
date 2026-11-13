-- Boots a place built from a copy of the sample game and buys its coin pack, for the CLI's build tests:
-- lune run test/lib/boot-place <place> <coin pack id>. It runs from the repository root, like the specs.
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local World = require("./sample-world")

local place, coinPack = process.args[1], tonumber(process.args[2])
assert(place and coinPack, "usage: boot-place <place> <coin pack id>")

local w = World.new({ place = place })
local booted, bootError = pcall(function()
	w:bootServer()
	w:settle()
end)
assert(booted, `the server boots: {bootError}`)
w:connect("Ann", 4120)
w:settle()

local experience = w.server.load(w.place:GetService("ReplicatedStorage").TS["crucible-experience"])
assert(experience.Products.CoinPack.id == coinPack, `the coin pack's ID is {coinPack}`)
local bought = w:run(function()
	return w.marketplace.ProcessReceipt({
		PlayerId = 4120,
		PurchaseId = "coin-pack-1",
		ProductId = coinPack,
		CurrencySpent = 49,
	})
end)
w:settle(2)
assert(bought.value == roblox.Enum.ProductPurchaseDecision.PurchaseGranted, "the coin pack was granted")
for _, product in experience.Products do
	local read, message = pcall(function()
		return product.id
	end)
	print(`{product.key}: {if read then message else `throws {message}`}`)
end
w.place:Destroy()
