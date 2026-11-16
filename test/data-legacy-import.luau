local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("../packages/crucible/lune")

local USER = 4120
local LOAD_FAILED = "Your data could not load. Please rejoin."
local SESSION_ENDED = "Your data session ended. Please rejoin."

-- A legacy DataStore by user id. A held read parks until `release`, the way a slow GetAsync yields.
local function legacyStore(records)
	local store = { records = records or {}, reads = 0, stored = {}, held = false, waiting = {}, fail = nil }
	function store.read(userId: number)
		store.reads += 1
		if store.held then
			table.insert(store.waiting, coroutine.running())
			coroutine.yield()
		end
		if store.fail then
			error(store.fail, 0)
		end
		return store.records[userId]
	end
	function store.release()
		store.held = false
		local waiting = store.waiting
		store.waiting = {}
		for _, thread in waiting do
			local ok, err = coroutine.resume(thread)
			assert(ok, tostring(err))
		end
	end
	return store
end

-- Version 1 of the FD store renames the legacy `gold` to `currency` and replaces the settings whole.
local function migrations(api)
	return {
		function(old)
			return {
				currency = if type(old.gold) == "number" then old.gold else 0,
				gold = api.migrationDelete,
				settings = if old.loud then api.migrationReplace({ vibration = false, volume = 1 }) else nil,
				loud = api.migrationDelete,
			}
		end,
	}
end

local function declare(r)
	return { PlayerData = fixture.declare(r, { migrations = migrations(r.graph.module("")) }) }
end

-- A world whose Importer Subsystem imports from `legacy` with `read`, the default turning a legacy record into a
-- version 0 record. Logger records every event.
local function newWorld(legacy, options)
	options = options or {}
	local events = {}
	local w = World.new({
		declare = declare,
		mustBoot = options.mustBoot,
		serve = function(s, d)
			s.service("Logger", function(self)
				self.events = d.PlayerData:observe(function(event)
					table.insert(events, event)
				end)
			end)
			s.service("Importer", function(self)
				self.legacy = d.PlayerData:importLegacy(options.read or function(player, stored)
					legacy.stored[player.UserId] = stored or false
					local record = legacy.read(player.UserId)
					return if record == nil then nil else { record = record, version = 0 }
				end)
			end)
			if options.serve then
				options.serve(s, d)
			end
		end,
	})
	w.events = events
	return w
end

local function kinds(events)
	local out = {}
	for _, event in events do
		table.insert(out, event.kind)
	end
	return out
end

local function seed(w, payload, version: number, fields)
	local envelope = {
		format = 1,
		kind = "profiles",
		schemaVersion = version,
		payload = payload,
		epoch = 3,
		receipts = {},
		inbox = {},
		quarantine = {},
		seen = {},
		seenFloor = 0,
		operations = {},
		operationFloor = 0,
	}
	for key, value in fields or {} do
		envelope[key] = value
	end
	w.backend:seed(fixture.STORE, tostring(USER), envelope, { USER })
end

local function profile(w, player)
	return w.server.d.PlayerData(player)()
end

return {
	["legacy import: a legacy record goes through migrations, is saved with its mark in one write, and is never read again"] = function()
		local legacy = legacyStore({ [USER] = { gold = 40, loud = true, xp = 3 } })
		local w = newWorld(legacy)
		local ann = w:player("Ann", USER)
		w:settle()
		spec.eq(legacy.stored[USER], false, "a new player has no stored record")
		spec.list(kinds(w.events), { "writable", "save" }, "the profile turns writable after the import, then saves it")
		spec.eq(w.events[1].revision, 1, "the import is the session's first change")
		local data = profile(w, ann.player)
		spec.eq(data.currency, 40, "migrations renamed gold to currency")
		spec.eq(data.xp, 3, "kept the legacy xp")
		spec.eq(data.settings.volume, 1, "and migrationReplace replaced the settings")
		spec.eq(data.settings.vibration, false, "whole")
		spec.eq(data.gold, nil, "migrationDelete removed gold")
		spec.eq(data.loud, nil, "and loud")
		spec.eq(w.events[2].result.ok, true, "at once")
		local envelope = w:envelope(fixture.STORE, USER)
		spec.eq(envelope.imported, true, "the store holds the mark")
		spec.eq(envelope.payload.currency, 40, "in the same write as the imported record")
		spec.eq(envelope.payload.gold, nil, "as migrated")
		spec.eq(envelope.schemaVersion, 1, "at the store's version")
		w:leave(ann)
		w:settle(2)
		local again = w:player("Ann", USER)
		w:settle(2)
		spec.eq(legacy.reads, 1, "a finished import never reads the legacy store again")
		spec.eq(profile(w, again.player).currency, 40, "and the profile is what was saved")
		w:finish()
	end,

	["legacy import: an existing profile is handed in, and a merge at its version keeps it"] = function()
		local legacy = legacyStore({ [USER] = { strength = 7 } })
		local w = newWorld(legacy, {
			read = function(player, stored)
				legacy.stored[player.UserId] = stored
				local old = legacy.read(player.UserId)
				local record = table.clone(stored.record)
				record.xp += old.strength
				return { record = record, version = stored.version }
			end,
		})
		seed(w, { currency = 5, xp = 2, notes = "kept" }, 1)
		local ann = w:player("Ann", USER)
		w:settle(1)
		local stored = legacy.stored[USER]
		spec.eq(stored.version, 1, "the stored record comes with its version")
		spec.eq(stored.record.currency, 5, "and its data")
		local data = profile(w, ann.player)
		spec.eq(data.xp, 9, "the merge added the legacy value")
		spec.eq(data.currency, 5, "and kept the profile")
		spec.eq(data.notes, "kept", "server-only fields too")
		local envelope = w:envelope(fixture.STORE, USER)
		spec.eq(envelope.imported, true, "marked")
		spec.eq(envelope.payload.xp, 9, "and saved")
		w:finish()
	end,

	["legacy import: nothing to import keeps the stored record, and marks it so the next load reads nothing"] = function()
		local legacy = legacyStore({})
		local w = newWorld(legacy)
		seed(w, { currency = 12 }, 1)
		local ann = w:player("Ann", USER)
		w:settle(1)
		spec.eq(legacy.reads, 1, "the import read the legacy store")
		spec.eq(profile(w, ann.player).currency, 12, "the stored record stays")
		spec.eq(w:envelope(fixture.STORE, USER).imported, true, "marked")
		w:leave(ann)
		w:settle(2)
		w:player("Ann", USER)
		w:settle(2)
		spec.eq(legacy.reads, 1, "and never read again")
		w:finish()
	end,

	["legacy import: a profile that holds the mark never runs it, with legacy data still present"] = function()
		local legacy = legacyStore({ [USER] = { gold = 99 } })
		local w = newWorld(legacy)
		seed(w, { currency = 4 }, 1, { imported = true })
		local ann = w:player("Ann", USER)
		w:settle(1)
		spec.eq(legacy.reads, 0, "the import did not run")
		spec.eq(profile(w, ann.player).currency, 4, "the stored record loaded")
		spec.list(kinds(w.events), { "writable" }, "with nothing to save")
		spec.eq(w.events[1].revision, 0, "at revision 0")
		w:finish()
	end,

	["legacy import: the profile is not writable while the import yields, and await waits for it"] = function()
		local legacy = legacyStore({ [USER] = { gold = 8 } })
		legacy.held = true
		local w = newWorld(legacy)
		local data = w.server.d.PlayerData
		local ann = w:player("Ann", USER)
		w:settle(5)
		spec.eq(#legacy.waiting, 1, "the import is reading")
		spec.eq(data:loaded(ann.player), false, "not writable")
		spec.eq(#data:players(), 0, "not listed")
		spec.eq(#w.events, 0, "no event yet")
		spec.eq(
			w:run(function()
				return data(ann.player).xp:set(5)
			end).value,
			false,
			"a write is refused"
		)
		local waited = w:run(function()
			return data:await(ann.player)
		end)
		w:settle(5)
		spec.eq(waited.done, nil, "await still waits")
		legacy.release()
		w:settle(1)
		spec.eq(waited.value, true, "await returns once the import is in")
		spec.eq(data:loaded(ann.player), true, "writable")
		spec.eq(profile(w, ann.player).currency, 8, "with the imported record")
		spec.list(kinds(w.events), { "writable", "save" }, "then writable and its save")
		w:finish()
	end,

	["legacy import: one that throws fails the load, kicks, reports no event, writes nothing, and runs again on the next join"] = function()
		local legacy = legacyStore({ [USER] = { gold = 30 } })
		legacy.fail = "legacy DataStore unavailable"
		local w = newWorld(legacy)
		local ann = w:player("Ann", USER)
		w:settle(2)
		spec.eq(w:kicked(ann.player), LOAD_FAILED, "the player is kicked")
		spec.eq(#w.events, 0, "a failed load reports nothing")
		spec.eq(w:logged("legacy import threw; the load failed"), 1, "the failure is reported")
		local envelope = w:envelope(fixture.STORE, USER)
		spec.eq(envelope.lease, nil, "the load removed its own lease")
		spec.eq(envelope.payload, nil, "and wrote no record")
		spec.eq(envelope.imported, nil, "and no mark")
		w:leave(ann)
		w:settle(2)
		legacy.fail = nil
		local again = w:player("Ann", USER)
		w:settle(2)
		spec.eq(legacy.reads, 2, "the next join imports again")
		spec.eq(profile(w, again.player).currency, 30, "and loads the legacy record")
		spec.eq(w:envelope(fixture.STORE, USER).imported, true, "marked this time")
		w:finish()
	end,

	["legacy import: a result that is not a versioned record, or fails migrations or admission, fails the load"] = function()
		local results = {
			{ value = 5, reason = "returned number, not a VersionedRecord or undefined" },
			{ value = { record = "x", version = 0 }, reason = "returned a record of string, not a table" },
			{ value = { record = {}, version = 0.5 }, reason = "returned a version that is not a whole number from 0" },
			{ value = { record = {}, version = 2 }, reason = "returned version 2, above the store's 1" },
			{ value = { record = { currency = "lots" }, version = 1 }, reason = "did not load: FD.Players.currency" },
		}
		for index, case in results do
			local w = newWorld(nil, {
				read = function()
					return case.value
				end,
			})
			local ann = w:player("Ann", USER)
			w:settle(2)
			spec.eq(w:kicked(ann.player), LOAD_FAILED, `case {index} kicks`)
			spec.eq(w:logged(case.reason), 1, `case {index} reports: {case.reason}`)
			spec.eq(#w.events, 0, `case {index} reports no event`)
			local envelope = w:envelope(fixture.STORE, USER)
			spec.eq(envelope.payload, nil, `case {index} writes no record`)
			spec.eq(envelope.imported, nil, `case {index} writes no mark`)
			w:finish()
		end
	end,

	["legacy import: authority lost while the import yields ends the load, and nothing it read is written"] = function()
		local legacy = legacyStore({ [USER] = { gold = 50 } })
		legacy.held = true
		local w = newWorld(legacy)
		local ann = w:player("Ann", USER)
		w:advance(69)
		spec.eq(w:kicked(ann.player), nil, "the import may take until the safe bound")
		w:advance(2)
		spec.eq(w:kicked(ann.player), SESSION_ENDED, "past it, the load ends")
		spec.eq(w:logged("legacy import ran past the session's safe bound"), 1, "and says why")
		legacy.release()
		w:settle(5)
		spec.eq(#w.events, 0, "the late result never made the profile writable")
		spec.eq(w.server.d.PlayerData:loaded(ann.player), false, "nor loaded")
		local envelope = w:envelope(fixture.STORE, USER)
		spec.eq(envelope.payload, nil, "nothing the import read was written")
		spec.eq(envelope.imported, nil, "and no mark")
		spec.eq(envelope.lease, nil, "the load removed its own lease")
		w:finish()
	end,

	["legacy import: a player who leaves while it yields ends the load, and nothing it read is written"] = function()
		local legacy = legacyStore({ [USER] = { gold = 50 } })
		legacy.held = true
		local w = newWorld(legacy)
		local ann = w:player("Ann", USER)
		w:settle(3)
		w:leave(ann)
		w:settle(2)
		legacy.release()
		w:settle(5)
		spec.eq(w:logged("legacy import"), 0, "a leave is not a failed import")
		spec.eq(#w.events, 0, "no event")
		local envelope = w:envelope(fixture.STORE, USER)
		spec.eq(envelope.payload, nil, "nothing written")
		spec.eq(envelope.imported, nil, "no mark")
		spec.eq(envelope.lease, nil, "lease removed")
		local again = w:player("Ann", USER)
		w:settle(2)
		spec.eq(legacy.reads, 2, "the next join imports again")
		spec.eq(profile(w, again.player).currency, 50, "and loads it")
		w:finish()
	end,

	["legacy import: a handoff carries the mark, so a resume does not import again"] = function()
		local legacy = legacyStore({ [USER] = { gold = 6 } })
		local w = newWorld(legacy)
		local data = w.server.d.PlayerData
		w.backend:fail("reject", { kind = "update" }, 1, { code = 500, after = 1 })
		local ann = w:player("Ann", USER)
		w:settle(1)
		spec.eq(w.events[2].result.reason, "unavailable", "the import's own save was refused")
		spec.eq(w:envelope(fixture.STORE, USER).imported, nil, "so the store has no mark yet")
		local handed = w:run(function()
			return data:handoff(ann.player)
		end)
		w:settle(3)
		spec.eq(handed.value.ok, true, "handed off")
		spec.eq(w:envelope(fixture.STORE, USER).imported, true, "the release stored the mark")
		local resumed = w:run(function()
			return data:resume(ann.player)
		end)
		w:settle(3)
		spec.eq(resumed.value, true, "resumed")
		spec.eq(legacy.reads, 1, "without importing again")
		spec.eq(profile(w, ann.player).currency, 6, "with the imported record")
		w:finish()
	end,

	["legacy import: onLoad claims see the imported record"] = function()
		local legacy = legacyStore({ [USER] = { gold = 21 } })
		local seen
		local w = newWorld(legacy, {
			serve = function(s, d)
				s.service("Welcome", function(self)
					self.load = d.PlayerData:onLoad(function(_, data)
						seen = data.currency
						return data
					end)
				end)
			end,
		})
		w:player("Ann", USER)
		w:settle(1)
		spec.eq(seen, 21, "onLoad ran on the imported record")
		w:finish()
	end,

	["legacy import: a second Subsystem importing the same store fails boot, naming both"] = function()
		local w = newWorld(legacyStore({}), {
			mustBoot = false,
			serve = function(s, d)
				s.service("Other", function(self)
					self.legacy = d.PlayerData:importLegacy(function()
						return nil
					end)
				end)
			end,
		})
		spec.eq(w.booted, false, "boot failed")
		assert(
			string.find(tostring(w.bootError), "is imported by", 1, true),
			`the error names both Subsystems: {w.bootError}`
		)
	end,
}
