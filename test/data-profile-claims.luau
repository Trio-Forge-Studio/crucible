local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("./lib/load")

local ENUM = { ProductPurchaseDecision = { PurchaseGranted = "GRANTED", NotProcessedYet = "NOTYET" } }
local PRODUCT = 77

local function declare(r)
	local d = { PlayerData = fixture.declare(r), Score = r.emit.replicated("Game.Score", 0, r.n.f64) }
	if r.side == "server" then
		d.Gift = r.emit.profileMessage("FD.Gift", r.n.object({ { "coins", r.n.f64 } }))
	end
	return d
end

local function newWorld(serve, options)
	options = options or {}
	return World.new({
		declare = options.declare or declare,
		serve = serve,
		globals = { Enum = ENUM },
		mustBoot = options.mustBoot,
		backend = options.backend,
	})
end

local function throws(fn, part: string, label: string)
	local ok, err = pcall(fn)
	assert(not ok, `{label}: expected a throw`)
	spec.has(err, part, label)
	return err
end

local function plus(data, field: string, amount: number)
	local copy = table.clone(data)
	copy[field] = data[field] + amount
	return copy
end

local function purchase(w, id: string, product: number?)
	return w:run(function()
		return w.marketplace.ProcessReceipt({
			PlayerId = 4120,
			PurchaseId = id,
			ProductId = product or PRODUCT,
			CurrencySpent = 10,
		})
	end)
end

return {
	["6 a claim outside a server Subsystem's construction throws"] = function()
		local w = newWorld()
		throws(function()
			w.server.d.PlayerData:grant(PRODUCT, function(_, data)
				return data
			end)
		end, "FD.Players.grant runs while a server Subsystem is constructed; make it a field initializer", "grant")
		throws(function()
			w.server.d.PlayerData:onLoad(function(_, data)
				return data
			end)
		end, "FD.Players.onLoad runs while a server Subsystem is constructed", "onLoad")
		w.server.construction.setClaim({ record = { kind = 1, id = "Crate" } })
		local ok, err = pcall(function()
			w.server.d.PlayerData:receive(w.server.d.Gift, function(_, data)
				return data
			end)
		end)
		w.server.construction.clearClaim()
		assert(not ok, "a claim while a behaviour is constructed throws")
		spec.has(err, "FD.Players.receive runs while a server Subsystem is constructed", "naming the claim")
	end,

	["6 18 onLoad claims run in construction order then field order, get plain data, and their change saves at once"] = function()
		local order = {}
		local w = newWorld(function(s, d)
			s.service("First", function(self)
				self.a = d.PlayerData:onLoad(function(_, data, reason)
					table.insert(order, `First.a {reason}`)
					assert(getmetatable(data) == nil and table.isfrozen(data), "a claim gets plain frozen data")
					return plus(data, "currency", 1)
				end)
				self.b = d.PlayerData:onLoad(function(_, data)
					table.insert(order, "First.b")
					local copy = table.clone(data)
					copy.currency = data.currency * 10
					return copy
				end)
			end)
			s.service("Second", function(self)
				self.c = d.PlayerData:onLoad(function(_, data)
					table.insert(order, "Second.c")
					return plus(data, "currency", 2)
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:settle()
		spec.list(order, { "First.a join", "First.b", "Second.c" }, "claim order")
		spec.eq(w.server.d.PlayerData(ann.player).currency(), 12, "each claim got the previous claim's result")
		w:advance(1)
		local stored = w:envelope(fixture.STORE, 4120)
		spec.eq(stored.payload.currency, 12, "the changed onLoad result saved at once")
		spec.eq(stored.commit.revision, 3, "each changed claim result is one revision")
		w:finish()
	end,

	["6 18 an onLoad result that fails admission fails the load naming the claim's owner"] = function()
		local w = newWorld(function(s, d)
			s.service("Bad", function(self)
				self.claim = d.PlayerData:onLoad(function(_, data)
					local copy = table.clone(data)
					copy.currency = "SECRET-VALUE"
					return copy
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:settle()
		spec.eq(w.server.d.PlayerData:loaded(ann.player), false, "the load failed")
		spec.eq(w:kicked(ann.player), "Your data could not load. Please rejoin.", "kicked")
		assert(w:logged("onLoad claim of Bad failed") >= 1, "the diagnostic names the owner")
		for _, line in w.server.diagnostics() do
			assert(string.find(line, "SECRET-VALUE", 1, true) == nil, "110 no diagnostic carries the value")
		end
		w:finish()
	end,

	["18 20 a leave during the load writes only the abandonment and runs no onRelease claim"] = function()
		local released = 0
		local w = newWorld(function(s, d)
			s.service("Leaving", function(self)
				self.claim = d.PlayerData:onRelease(function(_, data)
					released += 1
					return data
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:advance(0.06)
		w:leave(ann)
		w:settle(2)
		spec.eq(released, 0, "no onRelease claim ran")
		local stored = w:envelope(fixture.STORE, 4120)
		spec.eq(stored.payload, nil, "no payload was written")
		spec.eq(stored.lease, nil, "the acquisition was abandoned")
		w:finish()
	end,

	["6 a second receiver for one message or grant for one product fails boot naming both owners"] = function()
		local w = newWorld(function(s, d)
			s.service("GiftA", function(self)
				self.claim = d.PlayerData:receive(d.Gift, function(_, data)
					return data
				end)
			end)
			s.service("GiftB", function(self)
				self.claim = d.PlayerData:receive(d.Gift, function(_, data)
					return data
				end)
			end)
		end, { mustBoot = false })
		assert(not w.booted, "boot failed")
		spec.has(w.bootError, "FD.Gift is received by GiftA and GiftB", "both receivers named")
		local g = newWorld(function(s, d)
			s.service("SkinA", function(self)
				self.claim = d.PlayerData:grant(PRODUCT, function(_, data)
					return data
				end)
			end)
			s.service("SkinB", function(self)
				self.claim = d.PlayerData:grant(PRODUCT, function(_, data)
					return data
				end)
			end)
		end, { mustBoot = false })
		assert(not g.booted, "boot failed")
		spec.has(g.bootError, "product 77 is granted by SkinA and SkinB", "both grants named")
	end,

	["13 A4 K7 atom and publish writes inside every kind of data callback land, and their effects wait for it"] = function()
		local results = {}
		local w
		local function probe(kind: string)
			local before = w.seen
			w.localState(function(v)
				return v + 1
			end)
			w.score(function(v)
				return v + 1
			end)
			results[kind] = w.seen == before
		end
		w = newWorld(function(s, d, world)
			s.service("Probe", function(self)
				world.score = s.entry.publish(d.Score)
				world.localState = s.charm.atom(0)
				world.seen = 0
				s.api.effect(function()
					world.localState()
					world.seen += 1
				end)
				self.load = d.PlayerData:onLoad(function(_, data)
					probe("onLoad")
					return data
				end)
				self.release = d.PlayerData:onRelease(function(_, data)
					probe("onRelease")
					return data
				end)
				self.gift = d.PlayerData:receive(d.Gift, function(_, data, payload)
					probe("receive")
					return plus(data, "currency", payload.coins)
				end)
				self.skin = d.PlayerData:grant(PRODUCT, function(_, data)
					probe("grant")
					return plus(data, "xp", 1)
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:settle()
		local data = w.server.d.PlayerData
		data(ann.player):update(function(d)
			probe("gate")
			return d
		end)
		data(ann.player).currency:update(function(c)
			probe("handle update")
			return c
		end)
		local sent = w:run(function()
			return data:send(4120, w.server.d.Gift, { coins = 5 })
		end)
		w:settle(2)
		spec.eq(sent.value.ok, true, "the message was queued")
		local bought = purchase(w, "buy-1")
		w:settle(2)
		spec.eq(bought.value, "GRANTED", "the purchase was granted")
		w:leave(ann)
		w:settle(2)
		for _, kind in { "gate", "handle update", "onLoad", "onRelease", "receive", "grant" } do
			spec.eq(results[kind], true, `{kind}: the write's effect waited for the callback`)
		end
		spec.eq(w.localState(), 6, "every atom write landed")
		spec.eq(w.score(), 6, "every publish write landed")
		w:finish()
	end,

	["57 104 a grant claim installs ProcessReceipt at activation and grants after the save confirms"] = function()
		local effects = 0
		local w = newWorld(function(s, d)
			s.service("Skins", function(self)
				self.claim = d.PlayerData:grant(PRODUCT, function(_, data, bought, afterGrant)
					assert(bought.id == "buy-1" and bought.productId == PRODUCT, "the purchase")
					afterGrant(function()
						effects += 1
					end)
					return plus(data, "xp", 10)
				end)
			end)
		end)
		spec.eq(w.marketplace.ProcessReceipt, nil, "activation waits one frame after boot")
		w:advance(0.05)
		assert(type(w.marketplace.ProcessReceipt) == "function", "ProcessReceipt is installed at activation")
		local ann = w:player("Ann", 4120)
		local held = purchase(w, "buy-1")
		spec.eq(held.done, nil, "a purchase before the load is held")
		w:settle(2)
		spec.eq(held.value, "GRANTED", "then granted once the save confirmed")
		spec.eq(effects, 1, "afterGrant ran once")
		spec.eq(w.server.d.PlayerData(ann.player).xp(), 10, "the grant's change")
		local unknown = purchase(w, "buy-2", 99)
		w:settle()
		spec.eq(unknown.value, "NOTYET", "an unknown product is refused")
		w:finish()
	end,

	["57 104 without a grant claim ProcessReceipt stays unset"] = function()
		local w = newWorld()
		w:advance(0.05)
		spec.eq(w.marketplace.ProcessReceipt, nil, "the game may own ProcessReceipt")
	end,

	["6 107 a retired claim never runs again, its product is refused and its messages stay queued"] = function()
		local granted, received = 0, 0
		local w = newWorld(function(s, d, world)
			world.owner = s.service("Gifts", function(self)
				world.gifts = self
				self.gift = d.PlayerData:receive(d.Gift, function(_, data)
					received += 1
					return data
				end)
				self.skin = d.PlayerData:grant(PRODUCT, function(_, data)
					granted += 1
					return data
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:settle()
		w.server.life.fault(w.server.state(w.gifts), 8, "retired")
		w.link:flush()
		local bought = purchase(w, "buy-9")
		w:settle()
		spec.eq(bought.value, "NOTYET", "a retired grant's product is unknown")
		spec.eq(granted, 0, "the grant never ran")
		local sent = w:run(function()
			return w.server.d.PlayerData:send(4120, w.server.d.Gift, { coins = 1 })
		end)
		w:settle(3)
		spec.eq(sent.value.ok, true, "queued")
		spec.eq(received, 0, "the retired receiver never ran")
		local inbox = w:envelope(fixture.STORE, 4120).inbox
		spec.eq(#inbox, 1, "the message stays queued")
		w:leave(ann)
		w:finish()
	end,

	["6 a retired onLoad claim never runs again and claims are frozen once DataRuntime started"] = function()
		local ran = 0
		local w = newWorld(function(s, d, world)
			s.service("Stamp", function(self)
				world.stamp = self
				self.claim = d.PlayerData:onLoad(function(_, data)
					ran += 1
					return data
				end)
			end)
		end)
		w:advance(0.05)
		w.server.life.fault(w.server.state(w.stamp), 8, "retired")
		w.link:flush()
		local ann = w:player("Ann", 4120)
		w:settle()
		spec.eq(w.server.d.PlayerData:loaded(ann.player), true, "the load succeeded")
		spec.eq(ran, 0, "the retired claim never ran")
		local construction = w.server.construction
		construction.setClaim(w.server.state(w:dataRuntime()))
		local ok, err = pcall(function()
			w.server.d.PlayerData:onLoad(function(_, data)
				return data
			end)
		end)
		construction.clearClaim()
		assert(not ok, "a claim after activation throws")
		spec.has(err, "claims are frozen once the data runtime has started", "naming the freeze")
		w:finish()
	end,

	["17 104 one load per profile store for each joining player"] = function()
		local w = newWorld(nil, {
			declare = function(r)
				local small = r.n.object({ { "level", r.n.f64 } })
				return {
					PlayerData = fixture.declare(r),
					Stats = r.emit.profileStore("FD.Stats", { template = { level = 1 } }, small, small, {}),
				}
			end,
		})
		local ann = w:player("Ann", 4120)
		w:settle()
		spec.eq(w.server.d.PlayerData:loaded(ann.player), true, "FD.Players loaded")
		spec.eq(w.server.d.Stats:loaded(ann.player), true, "FD.Stats loaded")
		spec.eq(#w.backend:callsTo({ store = "FD.Stats", key = "4120" }) >= 1, true, "FD.Stats was acquired")
		w:finish()
	end,

	["65 66 67 73 115 send, receive, quarantined and clearQuarantined through the store map their results"] = function()
		local w = newWorld(function(s, d)
			s.service("Gifts", function(self)
				self.gift = d.PlayerData:receive(d.Gift, function(_, data, payload)
					if payload.coins < 0 then
						error("negative gift")
					end
					return plus(data, "currency", payload.coins)
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:settle()
		local data = w.server.d.PlayerData
		local sent = w:run(function()
			return data:send(4120, w.server.d.Gift, { coins = 7 })
		end)
		w:settle(2)
		spec.eq(sent.value.ok, true, "ok")
		spec.eq(data(ann.player).currency(), 7, "the receiver applied the payload to the profile")
		throws(function()
			data:send(4120, w.server.d.Gift, { coins = "x" })
		end, "the payload of message FD.Gift is invalid", "an invalid payload throws at the call")
		local listed = w:run(function()
			return data:quarantined(4120)
		end)
		w:settle()
		spec.eq(listed.value.ok, true, "quarantined is a result")
		spec.eq(#listed.value.value, 0, "nothing quarantined")
		local cleared = w:run(function()
			return data:clearQuarantined(4120, {}, "discard")
		end)
		w:settle()
		spec.eq(cleared.value.ok, true, "clearQuarantined is a result")
		local inspected = w:run(function()
			return data:inspect(4120)
		end)
		w:settle()
		spec.eq(inspected.value.ok, true, "inspect")
		spec.eq(inspected.value.value.currency, 7, "inspect reads the live session as plain data")
		spec.eq(getmetatable(inspected.value.value), nil, "never a handle")
		local saved = w:run(function()
			return data:save(ann.player)
		end)
		w:settle()
		spec.eq(saved.value.ok, true, "save maps ok")
		local away = w:run(function()
			return data:save({ Name = "Ghost", UserId = 1 })
		end)
		w:settle()
		spec.eq(away.value.ok, false, "save for an absent player")
		spec.eq(away.value.reason, "unavailable", "is unavailable")
		w:finish()
	end,

	["13 send, clearQuarantined, save and handoff inside a data callback throw"] = function()
		local w = newWorld()
		local ann = w:player("Ann", 4120)
		w:settle()
		local data = w.server.d.PlayerData
		local outcomes = {}
		data(ann.player):update(function(d)
			for name, call in
				{
					send = function()
						data:send(4120, w.server.d.Gift, { coins = 1 })
					end,
					clear = function()
						data:clearQuarantined(4120, {}, "retry")
					end,
					save = function()
						data:save(ann.player)
					end,
					handoff = function()
						data:handoff(ann.player)
					end,
				}
			do
				local ok, err = pcall(call)
				outcomes[name] = if ok then "ran" else tostring(err)
			end
			return d
		end)
		spec.has(outcomes.send, "a write inside a data callback", "send")
		spec.has(outcomes.clear, "a write inside a data callback", "clearQuarantined")
		spec.has(outcomes.save, "a write inside a data callback", "save")
		spec.has(outcomes.handoff, "inside a data callback", "handoff")
		w:finish()
	end,

	["13 a handoff of another player from a thread a data callback spawned throws"] = function()
		local w = newWorld()
		local ann = w:player("Ann", 4120)
		local bea = w:player("Bea", 4121)
		w:settle()
		local data = w.server.d.PlayerData
		local spawned
		data(ann.player):update(function(d)
			World.spawn(function()
				spawned = { pcall(function()
					return data:handoff(bea.player)
				end) }
			end)
			return d
		end)
		spec.eq(type(spawned), "table", "the spawned thread's handoff ended at once instead of waiting on a release")
		spec.eq(spawned[1], false, "the spawned thread's handoff throws")
		spec.has(spawned[2], "a write inside a data callback", "with the callback message")
		spec.eq(data:loaded(bea.player), true, "Bea's session is still open")
		w:finish()
	end,

	["9 10 11 loaded, players and await follow the session"] = function()
		local w = newWorld()
		local data = w.server.d.PlayerData
		local ann = w:player("Ann", 4120)
		local waited = w:run(function()
			return data:await(ann.player)
		end)
		spec.eq(waited.done, nil, "await yields while the load runs")
		spec.eq(#data:players(), 0, "players() is empty while loading")
		w:settle()
		spec.eq(waited.value, true, "await returned true once writable")
		spec.eq(data:loaded(ann.player), true, "loaded")
		spec.eq(data:players()[1], ann.player, "players() lists Ann")
		local box = w:run(function()
			return data:handoff(ann.player)
		end)
		w:settle(2)
		spec.eq(box.value.ok, true, "handoff")
		spec.eq(data:loaded(ann.player), false, "sealed")
		spec.eq(#data:players(), 0, "players() is empty after the seal")
		local after = w:run(function()
			return data:await(ann.player)
		end)
		spec.eq(after.value, false, "await on a sealed session returns false at once")
		throws(function()
			data:loaded()
		end, "FD.Players.loaded needs its player on the server", "loaded with no player on the server")
		w:finish()
	end,

	["104 107 a rolled-back boot never activates, and retirement disconnects players and refuses receipts"] = function()
		local failed = newWorld(function(s, d)
			s.service("Skins", function(self)
				self.claim = d.PlayerData:grant(PRODUCT, function(_, data)
					return data
				end)
			end)
			s.service("Broken", function()
				error("constructor failed")
			end)
		end, { mustBoot = false })
		assert(not failed.booted, "boot failed")
		failed:advance(1)
		spec.eq(failed.marketplace.ProcessReceipt, nil, "a rolled-back boot installs no ProcessReceipt")
		spec.eq(#failed.added, 0, "and connects no players")
		local w = newWorld(function(s, d)
			s.service("Skins", function(self)
				self.claim = d.PlayerData:grant(PRODUCT, function(_, data)
					return data
				end)
			end)
		end)
		local ann = w:player("Ann", 4120)
		w:settle()
		w:retireData()
		spec.eq(#w.added, 0, "retirement disconnects PlayerAdded")
		spec.eq(w.server.d.PlayerData:loaded(ann.player), false, "every session sealed")
		local bought = purchase(w, "buy-3")
		w:settle()
		spec.eq(bought.value, "NOTYET", "the NotProcessedYet handler is set")
		local bea = w:player("Bea", 4121)
		w:settle()
		spec.eq(w.server.d.PlayerData:loaded(bea.player), false, "a player joining after retirement never loads")
		w:advance(3)
		spec.eq(w:kicked(ann.player), "Your data session ended. Please rejoin.", "kicked once the release settled")
		spec.eq(w:envelope(fixture.STORE, 4120).lease, nil, "released")
	end,
}
