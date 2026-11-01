local spec = require("./lib/load")

-- A Roblox surface for port.ts alone: services record every call, so a spec sees which backing a write reached.
local function roblox(options)
	local calls = {}
	local stored = {}
	local keyInfo = {
		GetUserIds = function()
			return { 4120, 77 }
		end,
		GetMetadata = function()
			return { region = "eu" }
		end,
	}
	local dataStores = {
		GetDataStore = function(_, store)
			table.insert(calls, `GetDataStore {store}`)
			return {
				UpdateAsync = function(_, key, fn)
					if options.fail then
						error(options.fail, 0)
					end
					local value, userIds, metadata = fn(stored[key], if stored[key] == nil then nil else keyInfo)
					table.insert(calls, { key = key, value = value, userIds = userIds, metadata = metadata })
					stored[key] = value
					return value
				end,
				GetAsync = function(_, key)
					return stored[key], keyInfo
				end,
			}
		end,
		GetRequestBudgetForRequestType = function()
			return 7
		end,
	}
	local services = {
		DataStoreService = dataStores,
		RunService = {
			IsStudio = function()
				return options.studio
			end,
		},
		ServerStorage = {
			GetAttribute = function(_, name)
				table.insert(calls, `GetAttribute {name}`)
				return if name == "CrucibleLiveData" then options.live else nil
			end,
		},
		HttpService = {
			GenerateGUID = function()
				return "guid"
			end,
		},
		Players = {
			GetPlayers = function()
				return {}
			end,
		},
	}
	local game = {
		JobId = "job",
		GetService = function(_, name)
			table.insert(calls, `GetService {name}`)
			return services[name]
		end,
	}
	local globals = {
		game = game,
		Random = {
			new = function()
				return {
					NextNumber = function()
						return 0.5
					end,
				}
			end,
		},
		DateTime = {
			now = function()
				return { UnixTimestampMillis = 1700000000250 }
			end,
		},
		Enum = { DataStoreRequestType = { UpdateAsync = "UpdateAsync", GetAsync = "GetAsync" } },
	}
	local module = spec.fresh("internal/data/port", globals)
	return module, calls, stored
end

local function writeOne(port)
	local seen = {}
	local outcome = port.update("FD.Players", "4120", function(current)
		table.insert(seen, current or false)
		return {
			value = { epoch = 1 },
			userIds = { 4120, 77 },
			metadata = { region = "eu" },
		}
	end)
	return outcome, seen
end

local function reached(calls, text: string): boolean
	for _, call in calls do
		if call == text then
			return true
		end
	end
	return false
end

return {
	["rule 106 Studio uses the in-memory port unless CrucibleLiveData is true, and nothing runs at import"] = function()
		local module, calls = roblox({ studio = true, live = nil })
		spec.eq(#calls, 0, "importing the port touches no service")
		local port = module.createRobloxDataPort()
		spec.eq(reached(calls, "GetService DataStoreService"), false, "Studio never opens DataStoreService")
		local outcome = writeOne(port)
		spec.eq(outcome.kind, "ok", "the memory backing ran the transform")
		spec.eq(port.read("FD.Players", "4120").value.value.epoch, 1, "and reads back what it stored")
		spec.eq(reached(calls, "GetDataStore FD.Players"), false, "nothing reached a live store")

		local _, flagCalls = roblox({ studio = true, live = false })
		spec.eq(reached(flagCalls, "GetDataStore FD.Players"), false, "an attribute that is not true stays in memory")

		local liveModule, liveCalls = roblox({ studio = true, live = true })
		writeOne(liveModule.createRobloxDataPort())
		spec.eq(reached(liveCalls, "GetDataStore FD.Players"), true, "CrucibleLiveData true reaches DataStoreService")

		local serverModule, serverCalls = roblox({ studio = false })
		writeOne(serverModule.createRobloxDataPort())
		spec.eq(reached(serverCalls, "GetDataStore FD.Players"), true, "a live server always reaches DataStoreService")
		spec.eq(reached(serverCalls, "GetAttribute CrucibleLiveData"), false, "a live server reads no attribute")
	end,
	["rule 105 and rule 41 the live port carries UserIds, metadata, wall time and error codes"] = function()
		local module, calls = roblox({ studio = false })
		local port = module.createRobloxDataPort()
		spec.eq(port.wall(), 1700000000.25, "wall is DateTime.now in seconds")
		spec.eq(port.budget("update"), 7, "the budget is the engine's")
		local _, firstSeen = writeOne(port)
		spec.eq(firstSeen[1], false, "an absent key reaches the transform as undefined")
		local _, seen = writeOne(port)
		spec.eq(seen[1].userIds[2], 77, "the transform sees the stored UserIds")
		spec.eq(seen[1].metadata.region, "eu", "and the stored metadata")
		local last = calls[#calls]
		spec.eq(last.userIds[1], 4120, "UpdateAsync receives the UserIds back")
		spec.eq(last.userIds[2], 77, "all of them")
		spec.eq(last.metadata.region, "eu", "and the metadata")
		local read = port.read("FD.Players", "4120")
		spec.eq(read.value.userIds[2], 77, "a read carries the UserIds")
		spec.eq(read.value.metadata.region, "eu", "and the metadata")

		local failing = roblox({ studio = false, fail = "503: service unavailable" })
		local outcome = writeOne(failing.createRobloxDataPort())
		spec.eq(outcome.kind, "error", "an engine error is an outcome")
		spec.eq(outcome.code, 503, "with its code")
		spec.eq(outcome.retryable and outcome.ambiguous, true, "classified")
	end,
}
