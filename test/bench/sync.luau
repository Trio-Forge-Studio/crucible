-- Run after pnpm build: lune run test/bench/sync.luau [trunk|charm|charm-native]. No argument runs 15 interleaved pairs,
-- trunk first in every pair, each in a fresh process. Downloads the pinned trial package into ignored build/bench.
-- FD 5514468: core/{Replication,Analytics,Party,Leaderboard,Mood}, job/{Intro,Job}, warehouse/Departure.
-- Shapes are copied; values and the ten-second workload are synthetic, not a recorded gameplay trace.
-- trunk/ holds the cdbcf09 compiler output for the two changed modules and its extracted equality routine.
-- Both runs use that scheduler. The trial replaces equality, framing and patch application through benchmark hooks.
-- charm-native is a single informational sample through server/client APIs, without Crucible's sync preflight or limits.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local net = require("@lune/net")
local process = require("@lune/process")
local roblox = require("@lune/roblox")
local serde = require("@lune/serde")
local World = require("../lib/sync-world")
local Trial = require("./sync-trial")
local spec = require("../lib/load")
local referenceEquality = fs.readFile("test/bench/trunk/equal.txt")

local PEERS, FRAMES, HZ = 30, 600, 60
local VERSION = "0.4.1"
local ARCHIVE_HASH = "fd4b8532605e4a8a238af2d6ba1b2544690200669a9824da6990c94f14f0b62d"
local CACHE = "build/bench/charm-sync-0.4.1"

local function prepare()
	if not fs.isDir(CACHE) then
		fs.writeDir(CACHE)
	end
	local archive = `{CACHE}/package.tgz`
	if not fs.isFile(archive) then
		local response = net.request(`https://registry.npmjs.org/@rbxts/charm-sync/-/charm-sync-{VERSION}.tgz`)
		assert(response.ok, `npm download failed: {response.statusCode}`)
		fs.writeFile(archive, response.body)
	end
	assert(serde.hash("sha256", fs.readFile(archive)) == ARCHIVE_HASH, "trial archive hash changed")
	-- Always extract the verified archive, including when reusing the cache.
	local result = process.exec("tar", { "-xzf", archive, "-C", CACHE })
	assert(result.ok, result.stderr)
	local package = serde.decode("json", fs.readFile(`{CACHE}/package/package.json`))
	assert(
		package.version == VERSION and package.dependencies["@rbxts/charm"] == "^0.11.1",
		"trial dependency range changed"
	)
	local charm = serde.decode("json", fs.readFile("packages/crucible/node_modules/@rbxts/charm/package.json"))
	assert(charm.version == "0.11.1", "benchmark must use the exact runtime pin")
end

local function loadSync(charm)
	local loaded = {}
	local load
	load = function(name)
		if loaded[name] == nil then
			local source = fs.readFile(`{CACHE}/package/src/{name}.luau`)
			loaded[name] = luau.load(source, {
				debugName = `charm-sync/{name}`,
				environment = {
					-- Force manual flush; spawn's coroutine cost remains inside the measured Heartbeat.
					game = false,
					task = {
						spawn = function(callback, ...)
							local thread = coroutine.create(callback)
							local ok, err = coroutine.resume(thread, ...)
							assert(ok, tostring(err))
							assert(coroutine.status(thread) == "dead", "sync sender yielded")
						end,
					},
					require = function(path)
						if path == "../Charm" then
							return charm
						end
						return load(assert(string.match(path, "^%./(.+)$")))
					end,
				},
			})()
		end
		return loaded[name]
	end
	return { server = load("server"), client = load("client"), patch = load("patch"), config = load("config") }
end

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
		return n.union({ n["nil"], node })
	end
	local function map(node)
		return n.map(n.string, node)
	end
	local f, s, b, c = n.f64, n.string, n.bool, n.color3
	local member = o({ UserId = f, JoinedAt = f })
	local party = o({
		Id = s,
		Members = n.array(member),
		Coming = n.array(o({ UserId = f, JoinedAt = optional(f), Until = f })),
		Invited = map(f),
		Trip = optional(s),
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
			awards = n.array(o({ id = s, userId = f, name = s, value = f, item = optional(s), fees = optional(f) })),
		})),
		["Job.Tasks"] = n.array(o({ Id = s, Text = s, Have = f, Need = f, Status = s })),
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
		["Leaderboard.Boards"] = map(n.array(o({ UserId = f, Name = s, Value = f }))),
		["Mood.Current"] = optional(
			o({ Mood = s, Weather = s, Seed = f, StartedAt = f, From = environment, FadeSeconds = f })
		),
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
	-- PARTY_MAX is four: seven parties of four and one of two account for the 30 simulated players.
	for i = 1, 8 do
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
	-- A delivery every six frames updates the score, counts and task progress together; immutable shared subtrees.
	if frame % 6 == 0 and frame <= 480 then
		local count = frame / 6
		put("Job.Score", { deliveredValue = count * 20, damageFees = 0, damaged = false, missed = 0 })
		put("Job.ItemCounts", { delivered = count, broken = 0, cracked = 0, left = 100 - count })
		local tasks = table.clone(values["Job.Tasks"])
		tasks[1] = table.clone(tasks[1])
		tasks[1].Have = count
		tasks[1].Status = if count >= 70 then "Done" else "Open"
		put("Job.Tasks", tasks)
	end
	if frame == 60 then
		put("Intro.Skips", { 100000001, 100000002 })
		put("Intro.SkippedAt", 1700000001)
		put("Departure.Countdowns", {})
	end
	if frame == 480 then
		put("Job.Phase", "Done")
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
end

local function run(protocol)
	local native = protocol == "charm-native"
	local values, handles, schemas, keys = initial(), {}, nil, {}
	local trial, adapter
	spec.rewrite(function(path, source)
		local name = string.match(path, "^packages/crucible/out/internal/sync/(.+)$")
		if name == "server-sync" or name == "frame" then
			local fixture = fs.readFile(`test/bench/trunk/{name}.txt`)
			if name == "frame" and protocol == "charm" then
				local count
				fixture, count = string.gsub(
					fixture,
					'%{ "key", n%.string %}, %{ "value", n%.dynamic %}',
					'{ "key", n.string }, { "mode", n.union({ n.literal("full"), n.literal("patch"), n.literal("same") }) }, { "value", n.dynamic }'
				)
				assert(count == 1, "expected the trunk value-entry schema")
			end
			return fixture
		end
		if name == "values" and not string.find(source, "\nlocal function equal(", 1, true) then
			local rewritten, count = string.gsub(source, "\nreturn {", function()
				return "\n" .. referenceEquality .. "\nreturn {\n equal = equal,"
			end)
			assert(count == 1, "expected the compiled values module's export table")
			return rewritten
		end
		return source
	end)

	local w = World.new({
		patch = function(world)
			if protocol == "charm" then
				local server = world.server
				trial = loadSync(server.charm)
				adapter = Trial(trial.patch, server.values)
				server.values.equal = function(a, b, work)
					return adapter.difference(a, b, work) == nil
				end
				local message = world.wire.frame.SyncFrameMessage
				local encodeBody, packet = world.wire.envelope.encodeBody, world.wire.envelope.frame
				local frameBytes = server.reactiveLimits.SYNC_FRAME_BYTES
				world.wire.frame.frameCost = function(frame, index)
					local ok, cost = pcall(function()
						local body = encodeBody(message.name, message.input, "payload", frame)
						local encoded = packet(body, 0, { index })
						return {
							bytes = buffer.len(encoded.bytes),
							entries = body.writer.budget.entries,
							visits = body.writer.budget.visits,
						}
					end)
					return if ok then cost else { bytes = frameBytes + 1, entries = 0, visits = 0 }
				end
			end
		end,
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
				declarations[key] = emit(key, values[key], schemas[key])
			end
			return declarations
		end,
		serve = function(s, declarations, world)
			world.played = s.charm.atom(0)
			world.project = function(player)
				return { savedSeconds = player.UserId + world.played(), sessionStartedAt = 1700000000 }
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
	local referenceEqual = luau.load(referenceEquality .. "\nreturn equal", {
		environment = {
			spend = s.values.spend,
			SYNC_FRAME_VALUE_VISITS = s.reactiveLimits.SYNC_FRAME_VALUE_VISITS,
		},
	})()
	local envelope = w.wire.envelope
	local index = w.wire.frame.SyncFrameMessage.index
	local pending, clients, players = {}, {}, {}
	local nativeClients = {}
	local nativeSync = if native then loadSync(s.charm) else nil
	local totalBytes, largestFrame, packets = 0, 0, 0
	local function send(player, payload, wireCodec)
		local body = envelope.encodeBody("Bench.Sync", wireCodec, "payload", payload)
		local packet = envelope.frame(body, 0, { index })
		local bytes = buffer.len(packet.bytes)
		totalBytes += bytes
		largestFrame = math.max(largestFrame, bytes)
		packets += 1
		table.insert(pending, {
			player = player,
			packet = packet,
			codec = wireCodec,
			payload = payload,
			entries = body.writer.budget.entries,
			visits = body.writer.budget.visits,
		})
		return true
	end
	s.graph.module("internal/sync/server-sync").installServerSync(function(player, frame)
		send(player, frame, w.wire.frame.SyncFrameMessage.input)
		return true
	end)
	if nativeSync then
		local nativeCodec = w.wire.codec.compile(s.n.array(object(s.n, {
			data = s.n.dynamic,
			type = s.n.union({ s.n.literal("init"), s.n.literal("patch") }),
		})))
		nativeSync.server.connect(function(player, payloads)
			local privateKey = `Analytics.Played/{player.UserId}`
			for _, payload in payloads do
				if payload.data[privateKey] then
					payload.data["Analytics.Played"] = payload.data[privateKey]
					payload.data[privateKey] = nil
				end
			end
			send(player, payloads, nativeCodec)
		end)
	end
	if adapter then
		local stage = sync.stage
		local declarationNamed = s.declarations.declarationNamed
		sync.stage = function(self, view, work)
			local staged = stage(self, view, work)
			for _, entry in staged.values do
				local declaration = declarationNamed(entry.key)
				local accepted = view.accepted[declaration]
				local before = if accepted then accepted.value else declaration.initial
				if view.revision == 0 then
					entry.mode, entry.value = "full", adapter.snapshot(entry.value)
				else
					local delta = adapter.difference(before, entry.value, work)
					entry.mode = if delta == nil then "same" else "patch"
					entry.value = delta
				end
			end
			return staged
		end
	end
	local function tick()
		if nativeSync then
			nativeSync.server.flush()
			return
		end
		if adapter then
			adapter.beginPatches()
		end
		s.engine.frame(5, 1 / HZ)
	end
	local function deliver()
		for _, sent in pending do
			local header = assert(envelope.readHeader(2, sent.packet.bytes, sent.packet.sidecar))
			local reader = envelope.openBody(sent.packet.bytes, sent.packet.sidecar, header)
			local decoded = envelope.decodeBody(sent.codec, "payload", reader)
			local client = clients[sent.player]
			local first = next(client) == nil
			local cost = if native
				then { bytes = buffer.len(sent.packet.bytes), entries = sent.entries, visits = sent.visits }
				else w.wire.frame.frameCost(sent.payload, index)
			assert(buffer.len(sent.packet.bytes) == cost.bytes, "preflight/encoder byte parity")
			assert(
				reader.budget.entries == cost.entries and reader.budget.visits == cost.visits,
				"preflight/decoder budget parity"
			)
			assert(decoded.kind ~= "failed", "server sync failed")
			if native then
				nativeClients[sent.player].client.patch(decoded)
			else
				for _, entry in decoded.values do
					if protocol == "trunk" then
						client[entry.key] = { value = entry.value }
					else
						local current = client[entry.key]
						local before = if current then current.value else nil
						local raw = if entry.mode == "full"
							then entry.value
							elseif entry.mode == "same" then before
							else trial.patch.apply(before, entry.value)
						client[entry.key] = { value = raw }
					end
				end
			end
			if first then
				for _, key in keys do
					assert(client[key], `first payload omitted {key}`)
				end
			end
		end
		table.clear(pending)
	end
	local publicSignals = {}
	if native then
		for key, handle in handles do
			publicSignals[key] = function()
				return { value = handle() }
			end
		end
	end
	for i = 1, PEERS do
		local player = { Name = `Peer{i}`, UserId = 100000000 + i }
		players[i], clients[player] = player, {}
		if adapter then
			adapter.beginPatches()
		end
		if nativeSync then
			local signals = table.clone(publicSignals)
			local plan = s.values.compilePlan(schemas["Analytics.Played"])
			signals[`Analytics.Played/{player.UserId}`] = function()
				return { value = s.values.admit(plan, "Analytics.Played", w.project(player)) }
			end
			nativeSync.server.addSignalsToClient(player, signals)
			local clientSync, setters = loadSync(s.charm), {}
			nativeClients[player] = clientSync
			for _, key in keys do
				setters[key] = function(value)
					clients[player][key] = if type(value) == "function" then value(clients[player][key]) else value
					return clients[player][key]
				end
			end
			clientSync.client.addSignals(setters)
		else
			sync:snapshot(player)
		end
	end
	if nativeSync then
		nativeSync.server.flush()
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
			local received = if trial
				then adapter.apply(nil, clients[player][key].value, true)
				else clients[player][key].value
			local actual = s.values.admit(plan, key, received)
			assert(referenceEqual(actual, s.values.admit(plan, key, expected)), `peer {player.Name} stale at {key}`)
		end
	end
	assert(#w.link.kicks == 0, "no peer was kicked")
	w.link:assertClean()
	return {
		protocol = protocol,
		informationalOnly = native,
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

prepare()
local protocol = process.args[1]
if protocol then
	assert(
		protocol == "trunk" or protocol == "charm" or protocol == "charm-native",
		"expected trunk, charm or charm-native"
	)
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
