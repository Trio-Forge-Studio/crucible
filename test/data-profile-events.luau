local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("../packages/crucible/lune")

local USER = 4120
local EVENT_KEYS = {
	writable = { "attempts", "kind", "player", "reason", "revision", "seconds", "store" },
	save = { "attempts", "kind", "player", "result", "revision", "saved", "seconds", "store" },
	lost = { "kind", "player", "reason", "revision", "saved", "store" },
	released = { "attempts", "kind", "player", "reason", "result", "revision", "saved", "seconds", "store" },
}

local function declare(r)
	return { PlayerData = fixture.declare(r) }
end

-- A world whose Logger service records every event it observes, checking each carries only its kind's fields.
local function newWorld(serve, declareStores)
	local events = {}
	local w = World.new({
		declare = declareStores or declare,
		serve = function(s, d, world)
			s.service("Logger", function(self)
				self.events = d.PlayerData:observe(function(event)
					assert(table.isfrozen(event), "an event is frozen")
					assert(event.result == nil or table.isfrozen(event.result), "so is its result")
					local keys = {}
					for key in event do
						table.insert(keys, key)
					end
					table.sort(keys)
					spec.list(keys, EVENT_KEYS[event.kind], `{event.kind} carries metadata only`)
					table.insert(events, event)
				end)
			end)
			if serve then
				serve(s, d, world)
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

local function addCoins(w, player, amount: number)
	return w.server.d.PlayerData(player).currency:update(function(c)
		return c + amount
	end)
end

local function saveNow(w, player)
	local box = w:run(function()
		return w.server.d.PlayerData:save(player)
	end)
	w:settle()
	return box.value
end

return {
	["a join, an automatic save and a leave report writable, save and released once each, in order"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		spec.list(kinds(w.events), { "writable" }, "the load reports writable")
		local writable = w.events[1]
		spec.eq(writable.store, fixture.STORE, "with the store")
		spec.eq(writable.player, ann.player, "and the player")
		spec.eq(writable.reason, "join", "and why it loaded")
		spec.eq(writable.revision, 0, "a new profile is revision 0, as the store holds it")
		spec.eq(writable.attempts, 1, "one lease request")
		assert(writable.seconds > 0 and writable.seconds < 1, `the load took one round trip, not {writable.seconds}`)
		w:advance(45)
		spec.eq(#w.events, 1, "renewals of the profile as loaded report nothing")
		spec.eq(addCoins(w, ann.player, 5), true, "written")
		w:advance(25)
		spec.list(kinds(w.events), { "writable", "save" }, "the renewal that carried the write reports a save")
		local save = w.events[2]
		spec.eq(save.result.ok, true, "the save is ok")
		spec.eq(save.revision, 1, "at revision 1")
		spec.eq(save.saved, 1, "which the store now holds")
		spec.eq(save.attempts, 1, "in one request")
		assert(save.seconds > 0 and save.seconds < 1, `the save took one round trip, not {save.seconds}`)
		spec.eq(w:envelope(fixture.STORE, USER).payload.currency, 5, "the store holds the write")
		w:advance(45)
		spec.eq(#w.events, 2, "renewals that carry nothing new report nothing")
		w:leave(ann)
		w:settle(2)
		spec.list(kinds(w.events), { "writable", "save", "released" }, "the leave reports released last")
		local released = w.events[3]
		spec.eq(released.reason, "leave", "released because the player left")
		spec.eq(released.result.ok, true, "its final write confirmed")
		spec.eq(released.revision, 1, "at the last revision")
		spec.eq(released.saved, 1, "with every revision saved")
		spec.eq(w:envelope(fixture.STORE, USER).lease, nil, "the lease is gone")
		w:finish()
	end,

	["an explicit save reports once, and a save with nothing new reports nothing"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 3)
		addCoins(w, ann.player, 4)
		spec.eq(saveNow(w, ann.player).ok, true, "save confirmed")
		spec.list(kinds(w.events), { "writable", "save" }, "the explicit save reports")
		spec.eq(w.events[2].revision, 2, "both writes in one save")
		spec.eq(w.events[2].saved, 2, "both saved")
		spec.eq(saveNow(w, ann.player).ok, true, "a second save confirms at once")
		spec.eq(#w.events, 2, "and reports nothing, having nothing new")
		w:finish()
	end,

	["a failed save reports its reason, and the next save of the same revisions reports ok"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 9)
		w.backend:fail("reject", { kind = "update" }, 1, { code = 500 })
		local box = w:run(function()
			return w.server.d.PlayerData:save(ann.player, 1)
		end)
		w:settle(2)
		spec.eq(box.value.ok, false, "save() did not confirm")
		spec.eq(box.value.reason, "timeout", "save() waited out its deadline")
		spec.list(kinds(w.events), { "writable", "save" }, "the rejected write reports a save")
		local failed = w.events[2]
		spec.eq(failed.result.ok, false, "that failed")
		spec.eq(failed.result.reason, "unavailable", "because the store refused it")
		spec.eq(failed.revision, 1, "carrying revision 1")
		spec.eq(failed.saved, 0, "of which nothing is saved")
		spec.eq(w.server.d.PlayerData:loaded(ann.player), true, "the session stays writable")
		w:advance(25)
		spec.list(kinds(w.events), { "writable", "save", "save" }, "the next renewal carries it again")
		spec.eq(w.events[3].result.ok, true, "and saves it")
		spec.eq(w.events[3].saved, 1, "revision 1")
		spec.eq(w:envelope(fixture.STORE, USER).payload.currency, 9, "the store holds it")
		w:finish()
	end,

	["a refusal after an ambiguous attempt reports uncertain"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 2)
		w.backend:fail("transient", { kind = "update" }, 1)
		w.backend:fail("reject", { kind = "update" }, 1, { code = 500 })
		w:run(function()
			return w.server.d.PlayerData:save(ann.player, 1)
		end)
		w:settle(3)
		spec.list(kinds(w.events), { "writable", "save" }, "the write reports once")
		spec.eq(w.events[2].attempts, 2, "after two requests")
		spec.eq(w.events[2].result.reason, "uncertain", "uncertain, since the first may have landed")
		w:finish()
	end,

	["an ambiguous save that gives up reports uncertain"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 2)
		w.backend:fail("transient", { kind = "update" }, 1000)
		w:run(function()
			return w.server.d.PlayerData:save(ann.player, 1)
		end)
		w:advance(80)
		local saves = 0
		for index, event in w.events do
			if event.kind == "save" then
				saves += 1
				spec.eq(
					event.result.reason,
					"uncertain",
					`save {index} reports uncertain, as an attempt may have landed`
				)
				spec.eq(event.saved, 0, `save {index} saved nothing`)
			end
		end
		assert(saves >= 1, "a write that gave up reports")
		assert(w.events[2].attempts > 1, `the first write retried, {w.events[2].attempts} attempts`)
		spec.eq(w.events[#w.events].kind, "lost", "the session is then lost")
		w:advance(60)
		spec.eq(w.events[#w.events].kind, "released", "and released last")
		spec.eq(w.events[#w.events].result.ok, false, "without confirming")
		w:finish()
	end,

	["a fenced session reports its failed save, lost, then released"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 1)
		w.backend:entry(fixture.STORE, tostring(USER)).value.lease.token = "server2-token1"
		w:advance(25)
		spec.list(kinds(w.events), { "writable", "save", "lost", "released" }, "once each, in order")
		spec.eq(w.events[2].result.reason, "unavailable", "the fenced save failed")
		local lost = w.events[3]
		spec.eq(lost.reason, "fenced", "another server holds the profile")
		spec.eq(lost.revision, 1, "at revision 1")
		spec.eq(lost.saved, 0, "with nothing saved")
		local released = w.events[4]
		spec.eq(released.reason, "lost", "released because it was lost")
		spec.eq(released.result.ok, false, "without saving")
		spec.eq(w:kicked(ann.player), "Your data session ended. Please rejoin.", "the player is kicked")
		w:advance(60)
		spec.eq(#w.events, 4, "nothing more")
		w:finish()
	end,

	["a session that cannot renew in time reports lost for authority"] = function()
		local w = newWorld()
		local ann = w:player("Ann", USER)
		w:settle()
		local gate = w.backend:gate("transform", { kind = "update" })
		w:advance(75)
		spec.list(kinds(w.events), { "writable", "lost" }, "lost once its local bound passes")
		spec.eq(w.events[2].reason, "authority", "this server could not confirm its lease")
		spec.eq(w.server.d.PlayerData:loaded(ann.player), false, "no longer writable")
		gate.release()
		w:advance(30)
		spec.list(kinds(w.events), { "writable", "lost", "released" }, "the release reports last")
		spec.eq(w.events[3].reason, "lost", "for the lost session")
		w:finish()
	end,

	["a handoff, a resume and a shutdown report their reasons"] = function()
		local w = newWorld()
		local data = w.server.d.PlayerData
		local ann = w:player("Ann", USER)
		w:settle()
		local handed = w:run(function()
			return data:handoff(ann.player)
		end)
		w:settle(2)
		spec.eq(handed.value.ok, true, "handed off")
		spec.list(kinds(w.events), { "writable", "released" }, "the handoff reports released")
		spec.eq(w.events[2].reason, "handoff", "for the handoff")
		spec.eq(w.events[2].result.ok, true, "confirmed")
		local resumed = w:run(function()
			return data:resume(ann.player)
		end)
		w:settle(2)
		spec.eq(resumed.value, true, "resumed")
		spec.list(kinds(w.events), { "writable", "released", "writable" }, "the resume reports writable")
		spec.eq(w.events[3].reason, "resume", "for the resume")
		w:retireData()
		w:advance(3)
		spec.list(kinds(w.events), { "writable", "released", "writable", "released" }, "the shutdown reports released")
		spec.eq(w.events[4].reason, "shutdown", "for the shutdown")
	end,

	["an observer that throws, yields or writes is reported once per kind and the data path goes on"] = function()
		local seen = {}
		local w = newWorld(function(s, d)
			s.service("Thrower", function(self)
				self.events = d.PlayerData:observe(function()
					error("observer broke")
				end)
			end)
			s.service("Yielder", function(self)
				self.events = d.PlayerData:observe(function()
					coroutine.yield()
				end)
			end)
			s.service("Writer", function(self)
				self.events = d.PlayerData:observe(function(event)
					d.PlayerData(event.player).currency:set(1000)
				end)
			end)
			s.service("After", function(self)
				self.events = d.PlayerData:observe(function(event)
					table.insert(seen, event.kind)
				end)
			end)
		end)
		local ann = w:player("Ann", USER)
		w:settle()
		spec.eq(w.server.d.PlayerData:loaded(ann.player), true, "the load still made the profile writable")
		spec.eq(w.server.d.PlayerData(ann.player).currency(), 0, "the writing observer changed nothing")
		addCoins(w, ann.player, 2)
		spec.eq(saveNow(w, ann.player).ok, true, "the save still confirmed")
		addCoins(w, ann.player, 2)
		spec.eq(saveNow(w, ann.player).ok, true, "and again")
		w:leave(ann)
		w:settle(2)
		spec.list(kinds(w.events), { "writable", "save", "save", "released" }, "the first observer saw every event")
		spec.list(seen, { "writable", "save", "save", "released" }, "and so did the one after the failing ones")
		spec.eq(w:envelope(fixture.STORE, USER).payload.currency, 4, "the store holds the writes, not the observer's")
		for _, owner in { "Thrower", "Yielder", "Writer" } do
			spec.eq(w:logged(`the observer of {owner} failed on writable for {USER}`), 1, `{owner} reported`)
			spec.eq(w:logged(`the observer of {owner} failed on save`), 1, `{owner} reported once for two saves`)
		end
		spec.eq(w:logged("observer broke"), 3, "each report carries the error: writable, save and released")
		w:finish()
	end,

	["a listener cannot change a result that other listeners and later events see"] = function()
		local tried = {}
		local seen = {}
		local w = newWorld(function(s, d)
			s.service("Tamperer", function(self)
				self.events = d.PlayerData:observe(function(event)
					if event.result ~= nil then
						local changed = pcall(function()
							event.result.ok = not event.result.ok
						end)
						table.insert(tried, if changed then "changed" else "refused")
					end
				end)
			end)
			s.service("After", function(self)
				self.events = d.PlayerData:observe(function(event)
					if event.result ~= nil then
						table.insert(seen, `{event.kind} {event.result.ok} {event.result.reason}`)
					end
				end)
			end)
		end)
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 1)
		spec.eq(saveNow(w, ann.player).ok, true, "the first save confirmed")
		addCoins(w, ann.player, 1)
		w.backend:fail("reject", { kind = "update" }, 1, { code = 500 })
		w:run(function()
			return w.server.d.PlayerData:save(ann.player, 1)
		end)
		w:settle(2)
		addCoins(w, ann.player, 1)
		spec.eq(saveNow(w, ann.player).ok, true, "a later save confirmed")
		w:leave(ann)
		w:settle(2)
		spec.list(tried, { "refused", "refused", "refused", "refused" }, "every write to a result threw")
		spec.list(
			seen,
			{ "save true nil", "save false unavailable", "save true nil", "released true nil" },
			"the next listener and every later success saw the results as they were"
		)
		w:finish()
	end,

	["an observer that throws on two stores is reported once per kind, not once per store"] = function()
		local w = newWorld(function(s, d)
			s.service("Thrower", function(self)
				self.players = d.PlayerData:observe(function()
					error("observer broke")
				end)
				self.stats = d.Stats:observe(function()
					error("observer broke")
				end)
			end)
		end, function(r)
			local small = r.n.object({ { "level", r.n.f64 } })
			return {
				PlayerData = fixture.declare(r),
				Stats = r.emit.profileStore("FD.Stats", { template = { level = 1 } }, small, small, {}),
			}
		end)
		local ann = w:player("Ann", USER)
		w:settle()
		spec.eq(w.server.d.Stats:loaded(ann.player), true, "FD.Stats loaded")
		spec.eq(w:logged("the observer of Thrower failed on writable"), 1, "both stores' writable report once")
		w:leave(ann)
		w:settle(2)
		spec.eq(w:logged("the observer of Thrower failed on released"), 1, "both stores' released report once")
		w:finish()
	end,

	["an event raised while an observer runs reaches every observer after the one it interrupted"] = function()
		local second = {}
		local w
		w = newWorld(function(s, d)
			s.service("Late", function(self)
				self.events = d.PlayerData:observe(function(event)
					if event.kind == "save" then
						w.link.now += 80
						d.PlayerData:loaded(event.player)
					end
				end)
			end)
			s.service("Second", function(self)
				self.events = d.PlayerData:observe(function(event)
					table.insert(second, event.kind)
				end)
			end)
		end)
		local ann = w:player("Ann", USER)
		w:settle()
		addCoins(w, ann.player, 1)
		saveNow(w, ann.player)
		spec.list(kinds(w.events), { "writable", "save", "lost" }, "the read past the bound lost the session")
		spec.list(second, { "writable", "save", "lost" }, "a later observer still sees the save before the loss")
		w:finish()
	end,

	["a retired Subsystem's observer hears nothing more"] = function()
		local other = {}
		local w = newWorld(function(s, d)
			s.service("Other", function(self)
				self.events = d.PlayerData:observe(function(event)
					table.insert(other, event.kind)
				end)
			end)
		end)
		local ann = w:player("Ann", USER)
		w:settle()
		for _, record in w.server.registry.records() do
			if record.id == "Logger" then
				w.server.life.fault(w.server.state(record.state.object), 8, "retired")
			end
		end
		w.link:flush()
		w:leave(ann)
		w:settle(2)
		spec.list(kinds(w.events), { "writable" }, "the retired observer stopped")
		spec.list(other, { "writable", "released" }, "the live one heard the release")
		w:finish()
	end,

	["observe outside a server Subsystem's construction throws"] = function()
		local w = newWorld()
		local ok, err = pcall(function()
			w.server.d.PlayerData:observe(function() end)
		end)
		assert(not ok, "observe after boot throws")
		spec.has(err, "FD.Players.observe runs while a server Subsystem is constructed", "naming the member")
		w:finish()
	end,
}
