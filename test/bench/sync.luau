-- Run after pnpm build: lune run test/bench/sync.luau [trunk|charm].
-- No argument runs 15 fresh-process pairs, trunk first. The Charm arm loads shipped modules.
-- Workload: FD 5514468's 20 declarations, one four-player job, with synthetic accelerated cadence.
-- Includes task rebuilds, board re-sorts, resampled mood, private anchor load/unload, party changes and reset.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local serde = require("@lune/serde")
local World = require("../lib/sync-world")
local spec = require("../lib/load")
local referenceEquality = fs.readFile("test/bench/trunk/equal.txt")

local PEERS, FRAMES, HZ = 4, 600, 60
local VERSION = "0.4.1"

local function object(n, fields)
	local keys, entries = {}, {}
	for key in fields do
		table.insert(keys, key)
	end
	table.sort(keys)
	for _, key in keys do
		table.insert(entries, { key, fields[key] })
	end
	return n.object(entries)
end

local function shapes(n)
	local function o(fields)
		return object(n, fields)
	end
	local function optional(node)
		if node.kind == "union" then
			local branches = { n["nil"] }
			for _, branch in node.branches do
				table.insert(branches, branch)
			end
			return n.union(branches)
		end
		return n.union({ n["nil"], node })
	end
	local function map(node)
		return n.map(n.string, node)
	end
	local function literals(names)
		table.sort(names)
		local branches = {}
		for _, name in names do
			table.insert(branches, n.literal(name))
		end
		return n.union(branches)
	end
	local f, s, b, c = n.f64, n.string, n.bool, n.color3
	local member = o({ UserId = f, JoinedAt = f })
	local party = o({
		Id = s,
		Members = n.array(member),
		Coming = n.array(o({ UserId = f, JoinedAt = optional(f), Until = f })),
		Invited = map(f),
		Trip = optional(literals({ "Leaving", "Left" })),
		Contract = optional(o({ Id = s, Truck = f, Until = f, Payer = f, RushPay = f, BuyIn = f })),
	})
	local environment = o({
		Lighting = o({
			ClockTime = f,
			GeographicLatitude = f,
			Brightness = f,
			ExposureCompensation = f,
			Ambient = c,
			OutdoorAmbient = c,
			ColorShift_Top = c,
			ColorShift_Bottom = c,
			EnvironmentDiffuseScale = f,
			EnvironmentSpecularScale = f,
			GlobalShadows = b,
			ShadowSoftness = f,
		}),
		Sky = o({
			SkyboxFt = s,
			SkyboxBk = s,
			SkyboxRt = s,
			SkyboxLf = s,
			SkyboxUp = s,
			SkyboxDn = s,
			CelestialBodiesShown = b,
			SunAngularSize = f,
			MoonAngularSize = f,
			StarCount = f,
			SunTextureId = s,
			MoonTextureId = s,
			SkyboxOrientation = n.vector3,
		}),
		Atmosphere = o({ Density = f, Offset = f, Haze = f, Glare = f, Color = c, Decay = c }),
		ColorCorrectionEffect = o({ Enabled = b, Brightness = f, Contrast = f, Saturation = f, TintColor = c }),
		BloomEffect = o({ Enabled = b, Intensity = f, Size = f, Threshold = f }),
		SunRaysEffect = o({ Enabled = b, Intensity = f, Spread = f }),
		DepthOfFieldEffect = o({
			Enabled = b,
			FarIntensity = f,
			NearIntensity = f,
			FocusDistance = f,
			InFocusRadius = f,
		}),
	})
	return {
		["Analytics.Played"] = optional(o({ savedSeconds = f, sessionStartedAt = f })),
		["Departure.Countdowns"] = map(o({ Party = s, Members = n.array(f), EndsAt = f })),
		["Intro.StartedAt"] = optional(f),
		["Intro.SkippedAt"] = optional(f),
		["Intro.Skips"] = n.array(f),
		["Intro.Voters"] = f,
		["Job.Contract"] = optional(s),
		["Job.RushPay"] = optional(f),
		["Job.StartedAt"] = optional(f),
		["Job.EndsAt"] = optional(f),
		["Job.Phase"] = optional(s),
		["Job.Score"] = o({ deliveredValue = f, damageFees = f, damaged = b, missed = f }),
		["Job.ItemCounts"] = optional(o({ delivered = f, broken = f, cracked = f, left = f })),
		["Job.Highlights"] = optional(o({
			awards = n.array(o({
				id = literals({
					"GoldenMover",
					"Clutch",
					"HeavyLifter",
					"GentleTouch",
					"Butterfingers",
					"BigTicket",
					"PatchJob",
					"LongHaul",
					"Workhorse",
					"FirstIn",
				}),
				userId = f,
				name = s,
				value = f,
				item = optional(s),
				fees = optional(f),
			})),
		})),
		["Job.Tasks"] = n.array(o({
			Id = literals({ "Quota", "Careful", "Gentle", "Fragile", "Heavy", "Haul", "Prized" }),
			Text = s,
			Have = f,
			Need = f,
			Status = literals({ "Open", "Done", "Failed" }),
		})),
		["Job.ReturnsToWarehouse"] = b,
		["Job.PlayerPayouts"] = map(o({
			base = f,
			deliveryPay = f,
			deliveryPayPercent = f,
			rush = f,
			tip = f,
			friend = f,
			friendPercent = f,
			earlyFinish = f,
			secondsLeft = f,
			currency = f,
			friendCount = f,
			xp = f,
			mastery = optional(o({
				counted = f,
				eligible = b,
				difficulty = s,
				needed = f,
				from = f,
				goal = f,
				unlocked = optional(s),
				mastered = b,
			})),
		})),
		["Leaderboard.Boards"] = o({
			Money = optional(n.array(o({ UserId = f, Name = s, Value = f }))),
			PlayTime = optional(n.array(o({ UserId = f, Name = s, Value = f }))),
			Jobs = optional(n.array(o({ UserId = f, Name = s, Value = f }))),
		}),
		["Mood.Current"] = optional(o({
			Mood = literals({
				"Morning",
				"Midday",
				"Warehouse",
				"GoldenHour",
				"Dusk",
				"ManorNight",
				"ChaletDay",
				"ChaletNight",
				"Night",
			}),
			Weather = literals({ "Clear", "Snowfall" }),
			Seed = f,
			StartedAt = f,
			From = environment,
			FadeSeconds = f,
		})),
		["Party.Parties"] = map(party),
	}
end

local function initial()
	local rgb = roblox.Color3.fromRGB
	local sky = {
		CelestialBodiesShown = true,
		SunAngularSize = 0,
		MoonAngularSize = 0,
		StarCount = 0,
		SunTextureId = "rbxassetid://6196665106",
		MoonTextureId = "rbxassetid://6444320592",
		SkyboxOrientation = roblox.Vector3.zero,
	}
	for _, side in { "Ft", "Bk", "Rt", "Lf", "Up", "Dn" } do
		sky[`Skybox{side}`] = "rbxassetid://1234567890"
	end
	local boards = {}
	-- FD LeaderboardConfig.Size is 100, with Money, PlayTime and Jobs boards; refresh interval is 120 seconds.
	for _, id in { "Money", "PlayTime", "Jobs" } do
		local entries = {}
		for i = 1, 100 do
			entries[i] = { UserId = 100000000 + i, Name = string.format("Player%014d", i), Value = 100000 - i }
		end
		boards[id] = entries
	end
	local parties, departures = {}, {}
	-- One job server has at most four party members.
	for i = 1, math.ceil(PEERS / 4) do
		local members = {}
		for p = (i - 1) * 4 + 1, math.min(i * 4, PEERS) do
			table.insert(members, { UserId = 100000000 + p, JoinedAt = 1700000000 })
		end
		local id = string.format("00000000-0000-4000-8000-%012d", i)
		parties[id] = {
			Id = id,
			Members = members,
			Coming = {},
			Invited = {},
			Contract = {
				Id = "SuburbanHouse.Hard",
				Truck = i,
				Until = 1700000600,
				Payer = members[1].UserId,
				RushPay = 100,
				BuyIn = 50,
			},
		}
		local ids = {}
		for _, member in members do
			table.insert(ids, member.UserId)
		end
		departures[tostring(i)] = { Party = id, Members = ids, EndsAt = 1700000010 }
	end
	return {
		["Departure.Countdowns"] = departures,
		["Intro.StartedAt"] = 1700000000,
		["Intro.Skips"] = {},
		["Intro.Voters"] = 4,
		["Job.Contract"] = "SuburbanHouse.Hard",
		["Job.RushPay"] = 100,
		["Job.StartedAt"] = 1700000000,
		["Job.EndsAt"] = 1700000300,
		["Job.Phase"] = "Running",
		["Job.Score"] = { deliveredValue = 0, damageFees = 0, damaged = false, missed = 0 },
		["Job.ItemCounts"] = { delivered = 0, broken = 0, cracked = 0, left = 100 },
		["Job.Tasks"] = {
			{ Id = "Quota", Text = "Deliver 70% of the house value", Have = 0, Need = 70, Status = "Open" },
			{ Id = "Careful", Text = "Break no more than 1 item", Have = 0, Need = 1, Status = "Open" },
			{ Id = "Heavy", Text = "Deliver 75% of the heavy items", Have = 0, Need = 75, Status = "Open" },
		},
		["Job.ReturnsToWarehouse"] = false,
		["Job.PlayerPayouts"] = {},
		["Leaderboard.Boards"] = boards,
		["Mood.Current"] = {
			Mood = "Morning",
			Weather = "Clear",
			Seed = 42,
			StartedAt = 1700000000,
			FadeSeconds = 10,
			From = {
				Lighting = {
					ClockTime = 10.486,
					GeographicLatitude = 57.22,
					Brightness = 2.3,
					ExposureCompensation = 0.3,
					Ambient = rgb(165, 155, 170),
					OutdoorAmbient = rgb(160, 150, 170),
					ColorShift_Top = rgb(255, 215, 190),
					ColorShift_Bottom = rgb(170, 180, 230),
					EnvironmentDiffuseScale = 0.5,
					EnvironmentSpecularScale = 0.2,
					GlobalShadows = true,
					ShadowSoftness = 0.2,
				},
				Sky = sky,
				Atmosphere = {
					Density = 0.35,
					Offset = 0.2,
					Haze = 1.2,
					Glare = 0.3,
					Color = rgb(235, 215, 225),
					Decay = rgb(170, 170, 210),
				},
				ColorCorrectionEffect = {
					Enabled = true,
					Brightness = 0,
					Contrast = 0.05,
					Saturation = 0.15,
					TintColor = rgb(255, 245, 245),
				},
				BloomEffect = { Enabled = true, Intensity = 0.5, Size = 24, Threshold = 2 },
				SunRaysEffect = { Enabled = true, Intensity = 0.12, Spread = 0.8 },
				DepthOfFieldEffect = {
					Enabled = false,
					FarIntensity = 0.1,
					NearIntensity = 0.75,
					FocusDistance = 0.05,
					InFocusRadius = 30,
				},
			},
		},
		["Party.Parties"] = parties,
	}
end

local function writes(frame, write, values)
	local function put(key, value)
		values[key] = value
		write(key, value)
	end
	-- FD evaluates every task afresh on each delivery; item counts wait for settlement.
	if frame % 6 == 0 and frame <= 480 then
		local count = frame / 6
		put("Job.Score", { deliveredValue = count * 20, damageFees = 0, damaged = false, missed = 0 })
		local tasks = {}
		for i, task in values["Job.Tasks"] do
			tasks[i] = table.clone(task)
		end
		tasks[1].Have = count
		tasks[1].Status = if count >= 70 then "Done" else "Open"
		put("Job.Tasks", tasks)
	end
	if frame == 60 then
		put("Intro.Skips", { 100000001, 100000002 })
		put("Intro.SkippedAt", 1700000001)
		put("Departure.Countdowns", {})
	end
	if frame == 300 then
		local boards = {}
		for id, entries in values["Leaderboard.Boards"] do
			local fresh = {}
			local last = entries[#entries]
			fresh[1] = { UserId = last.UserId, Name = last.Name, Value = 200000 }
			for i = 1, #entries - 1 do
				local entry = entries[i]
				fresh[i + 1] = { UserId = entry.UserId, Name = entry.Name, Value = entry.Value }
			end
			boards[id] = fresh
		end
		put("Leaderboard.Boards", boards)
	end
	if frame == 360 then
		local mood = table.clone(values["Mood.Current"])
		mood.Mood, mood.Seed, mood.StartedAt = "Dusk", 43, 1700000360
		local from = {}
		for group, fields in mood.From do
			local fresh = {}
			for key, value in fields do
				fresh[key] = if type(value) == "number" then value * 0.9 + 0.01 else value
			end
			from[group] = fresh
		end
		mood.From = from
		put("Mood.Current", mood)
	end
	if frame == 480 then
		put("Job.Phase", "Settling")
		put("Job.ItemCounts", { delivered = 80, broken = 0, cracked = 0, left = 20 })
		put("Job.Highlights", {
			awards = {
				{ id = "GoldenMover", userId = 100000001, name = "Player00000000000001", value = 500 },
				{
					id = "HeavyLifter",
					userId = 100000002,
					name = "Player00000000000002",
					value = 20,
					item = "Sofa",
				},
			},
		})
		local payouts = {}
		for i = 1, 4 do
			payouts[tostring(100000000 + i)] = {
				base = 100,
				deliveryPay = 160,
				deliveryPayPercent = 80,
				rush = 100,
				tip = 20,
				friend = 10,
				friendPercent = 5,
				earlyFinish = 10,
				secondsLeft = 292,
				currency = 400,
				friendCount = 3,
				xp = 200,
				mastery = {
					counted = 2,
					eligible = true,
					difficulty = "Hard",
					needed = 3,
					from = 1,
					goal = 3,
					mastered = false,
				},
			}
		end
		put("Job.PlayerPayouts", payouts)
	end
	if frame == 540 then
		put("Job.ReturnsToWarehouse", true)
		put("Job.Highlights", nil)
	end
	if frame == 120 then
		local parties = {}
		for id, party in values["Party.Parties"] do
			local fresh = table.clone(party)
			fresh.Invited = { ["100000099"] = 1700000120 }
			fresh.Trip = "Leaving"
			parties[id] = fresh
		end
		put("Party.Parties", parties)
	end
	if frame == 570 then
		put("Job.Phase", "Resetting")
		put("Job.EndsAt", nil)
		put("Job.StartedAt", nil)
		put("Job.ItemCounts", nil)
		put("Job.Highlights", nil)
		put("Job.Tasks", {})
		put("Job.PlayerPayouts", {})
		put("Job.Score", { deliveredValue = 0, damageFees = 0, damaged = false, missed = 0 })
		put("Job.Phase", "Waiting")
	end
end

local function run(protocol)
	local values, handles, schemas, keys = initial(), {}, nil, {}
	if protocol == "trunk" then
		spec.rewrite(function(path, source)
			local name = string.match(path, "^packages/crucible/out/internal/sync/(.+)$")
			if name == "server-sync" or name == "frame" then
				return fs.readFile(`test/bench/trunk/{name}.txt`)
			end
			if name == "values" then
				local rewritten, count = string.gsub(source, "\nreturn {", function()
					return "\n" .. referenceEquality .. "\nreturn {\n equal = equal,"
				end)
				assert(count == 1, "expected one compiled export table")
				return rewritten
			end
			return source
		end)
	end

	local w = World.new({
		declare = function(r)
			schemas = shapes(r.n)
			local declarations = {}
			for key in schemas do
				table.insert(keys, key)
			end
			table.sort(keys)
			assert(#keys == 20, "all FD declarations are present")
			for _, key in keys do
				local emit = if key == "Analytics.Played" then r.emit.replicatedPerPlayer else r.emit.replicated
				local empty = if key == "Job.Score"
					then { deliveredValue = 0, damageFees = 0, damaged = false, missed = 0 }
					elseif key == "Job.ReturnsToWarehouse" then false
					elseif key == "Intro.Voters" then 0
					elseif
						key == "Intro.Skips"
						or key == "Job.Tasks"
						or key == "Job.PlayerPayouts"
						or key == "Leaderboard.Boards"
						or key == "Party.Parties"
						or key == "Departure.Countdowns"
					then {}
					else nil
				declarations[key] = emit(key, empty, schemas[key])
			end
			return declarations
		end,
		serve = function(s, declarations, world)
			world.played = s.charm.atom(false)
			world.project = function(player)
				if not world.played() then
					return nil
				end
				return { savedSeconds = player.UserId, sessionStartedAt = 1700000000 }
			end
			s.service("Publisher", function()
				for _, key in keys do
					if key == "Analytics.Played" then
						s.entry.publish(declarations[key], world.project)
					else
						handles[key] = s.entry.publish(declarations[key])
					end
				end
			end)
		end,
	})
	local s, sync = w.server, w:sync()
	for key, handle in handles do
		handle(function()
			return values[key]
		end)
	end
	local adapter = if protocol == "charm" then s.graph.module("internal/sync/patches") else nil
	local referenceEqual = luau.load(referenceEquality .. "\nreturn equal", {
		environment = {
			spend = s.values.spend,
			SYNC_FRAME_VALUE_VISITS = s.reactiveLimits.SYNC_FRAME_VALUE_VISITS,
		},
	})()
	local envelope = w.wire.envelope
	local index = w.wire.frame.SyncFrameMessage.index
	local pending, clients, players = {}, {}, {}
	local totalBytes, largestFrame, packets = 0, 0, 0
	local function send(player, payload, wireCodec, packet)
		if protocol == "trunk" then
			local body = envelope.encodeBody("Bench.Sync", wireCodec, "payload", payload)
			packet = envelope.frame(body, 0, { index })
		end
		assert(packet, "preflight packet")
		local bytes = buffer.len(packet.bytes)
		totalBytes += bytes
		largestFrame = math.max(largestFrame, bytes)
		packets += 1
		table.insert(pending, {
			player = player,
			packet = packet,
			codec = wireCodec,
			payload = payload,
		})
		return true
	end
	s.graph.module("internal/sync/server-sync").installServerSync(function(player, frame, packet)
		send(player, frame, w.wire.frame.SyncFrameMessage.input, packet)
		return true
	end)
	local function tick()
		s.engine.frame(5, 1 / HZ)
	end
	local function deliver()
		for _, sent in pending do
			local header = assert(envelope.readHeader(2, sent.packet.bytes, sent.packet.sidecar))
			local reader = envelope.openBody(sent.packet.bytes, sent.packet.sidecar, header)
			local decoded = envelope.decodeBody(sent.codec, "payload", reader)
			local client = clients[sent.player]
			local first = next(client) == nil
			local cost = w.wire.frame.frameCost(sent.payload, index)
			assert(buffer.len(sent.packet.bytes) == cost.bytes, "preflight/encoder byte parity")
			assert(
				reader.budget.entries == cost.entries and reader.budget.visits == cost.visits,
				"preflight/decoder budget parity"
			)
			assert(decoded.kind ~= "failed", "server sync failed")
			for _, entry in decoded.values do
				local current = client[entry.key]
				local before = if current then current.value else nil
				local raw = if protocol == "trunk"
					then entry.value
					elseif entry.mode == "same" then before
					else adapter.apply(before, entry.value, entry.mode == "full")
				client[entry.key] = { value = raw }
			end
			if first then
				for _, key in keys do
					assert(client[key], `first payload omitted {key}`)
				end
			end
		end
		table.clear(pending)
	end
	for i = 1, PEERS do
		local player = { Name = `Peer{i}`, UserId = 100000000 + i }
		players[i], clients[player] = player, {}
		sync:snapshot(player)
	end
	-- Warm both implementations on 60 quiet Heartbeats before timing, without discarding snapshot byte counts.
	for _ = 1, 60 do
		tick()
	end
	local elapsed = 0
	for frame = 1, FRAMES do
		writes(frame, function(key, value)
			handles[key](function()
				return value
			end)
		end, values)
		if frame == 240 then
			w.played(true)
		end
		if frame == 540 then
			w.played(false)
		end
		local started = os.clock()
		tick()
		elapsed += os.clock() - started
	end
	local before = packets
	for _ = 1, 60 do
		tick()
	end
	assert(packets == before, "sync round never settles (TRI-166)")
	assert(not sync.incomplete and sync.examined == 0, "quiet sync round never settles (TRI-166)")
	-- Decode only after all timed frames. Client allocations must not shift garbage collection into a later
	-- server Heartbeat in this shared Lune VM. Both protocols still cross the actual encoder and decoder.
	deliver()
	for _, player in players do
		for _, key in keys do
			local expected = if key == "Analytics.Played" then w.project(player) else values[key]
			local plan = s.values.compilePlan(schemas[key])
			local received = clients[player][key].value
			local actual = s.values.admit(plan, key, received)
			assert(referenceEqual(actual, s.values.admit(plan, key, expected)), `peer {player.Name} stale at {key}`)
		end
	end
	assert(#w.link.kicks == 0, "no peer was kicked")
	w.link:assertClean()
	return {
		protocol = protocol,
		peers = PEERS,
		declarations = #keys,
		frames = FRAMES,
		hz = HZ,
		bytesPerSecond = totalBytes / (FRAMES / HZ),
		largestFrame = largestFrame,
		usPerHeartbeat = elapsed / FRAMES * 1e6,
		packets = packets,
		settled = true,
		frameBudget = s.reactiveLimits.SYNC_FRAME_BYTES,
		charmSync = VERSION,
		charm = "0.11.1",
	}
end

local protocol = process.args[1]
if protocol then
	assert(protocol == "trunk" or protocol == "charm", "expected trunk or charm")
	print(serde.encode("json", run(protocol)))
else
	for pair = 1, 15 do
		for _, name in { "trunk", "charm" } do
			local result = process.exec("lune", { "run", "test/bench/sync.luau", name })
			assert(result.ok, result.stderr)
			local row = serde.decode("json", result.stdout)
			row.pair = pair
			print(serde.encode("json", row))
		end
	end
end
