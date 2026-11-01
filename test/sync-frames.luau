local World = require("./lib/sync-world")
local spec = require("./lib/load")

local function declare(r)
	local n = r.n
	return {
		Ends = r.emit.replicated("Job.EndsAt", 0, n.f64),
		Phase = r.emit.replicated("Job.Phase", nil, n.union({ n["nil"], n.literal("Running"), n.literal("Waiting") })),
		Score = r.emit.replicated("Job.Score", 0, n.f64),
		Spare = r.emit.replicated("Job.Spare", "unset", n.string),
		Started = r.emit.replicated("Job.StartedAt", 0, n.f64),
	}
end

local function job(s, d, w)
	s.service("JobService", function(self)
		self.ends = s.entry.publish(d.Ends)
		self.phase = s.entry.publish(d.Phase)
		self.score = s.entry.publish(d.Score)
		self.started = s.entry.publish(d.Started)
		w.job = self
	end)
end

local function newWorld()
	return World.new({ declare = declare, serve = job })
end

return {
	["57 58 59 snapshot then update: revision 1 carries the published values and a later write arrives at revision 2"] = function()
		local w = newWorld()
		w.job.score:set(7)
		w.job.phase:set("Waiting")
		local ann = w:join("Ann")
		spec.eq(ann.d.Score(), 7, "the snapshot's value after the first delivery")
		spec.eq(ann.d.Phase(), "Waiting", "every published value")
		spec.eq(ann.api.syncStatus(ann.d.Score), "ready", "ready after its value arrived")
		local frames = w:frames(ann)
		spec.eq(#frames, 1, "one frame at ClientReady")
		spec.eq(frames[1].frame.kind, "snapshot", "a snapshot")
		spec.eq(frames[1].frame.revision, 1, "revision 1")
		spec.list(
			World.keys(frames[1]),
			{ "Job.EndsAt", "Job.Phase", "Job.Score", "Job.StartedAt" },
			"published values by name"
		)
		w.job.score:set(9)
		spec.eq(ann.d.Score(), 7, "the client keeps the old value until a Heartbeat")
		w:heartbeat()
		frames = w:frames(ann)
		spec.eq(#frames, 2, "one update")
		spec.eq(frames[2].frame.kind, "update", "an update")
		spec.eq(frames[2].frame.revision, 2, "revision 2")
		spec.list(World.keys(frames[2]), { "Job.Score" }, "only the changed value")
		spec.eq(ann.d.Score(), 9, "the client applied it")
		w:heartbeat()
		spec.eq(#w:frames(ann), 2, "a peer with nothing new gets nothing")
		w.server.api.state(0):set(1)
		w:heartbeat()
		spec.eq(#w:frames(ann), 2, "an examination that finds nothing new sends nothing")
		w.job.score:set(10)
		w:heartbeat()
		spec.eq(w:frames(ann)[3].frame.revision, 3, "revisions rise by exactly one per accepted frame")
		assert(w:finish() >= 3, "parity checked")
	end,
	["43 45 53 57 root undefined arrives as a ready value while an unpublished declaration stays pending at initial"] = function()
		local w = newWorld()
		w.job.phase:set("Running")
		local ann = w:join("Ann")
		spec.eq(ann.d.Phase(), "Running", "published")
		w.job.phase:set(nil)
		w:heartbeat()
		local frames = w:frames(ann)
		local value, present = World.value(frames[2], "Job.Phase")
		spec.eq(present, true, "root undefined is a value entry")
		spec.eq(value, nil, "carrying nil")
		spec.eq(ann.d.Phase(), nil, "the client reads undefined")
		spec.eq(ann.api.syncStatus(ann.d.Phase), "ready", "and its status is ready")
		spec.eq(ann.d.Spare(), "unset", "an unpublished declaration keeps initial")
		spec.eq(ann.api.syncStatus(ann.d.Spare), "pending", "with status pending")
		for _, decoded in frames do
			local _, sent = World.value(decoded, "Job.Spare")
			spec.eq(sent, false, "an unpublished declaration is never sent")
		end
		spec.eq(w:logged("Job.Spare"), 0, "and never diagnosed")
		w:finish()
	end,
	["58 three writes between Heartbeats are one frame carrying one value with the last write"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		w.job.score:set(1)
		w.job.score:set(2)
		w.job.score:set(3)
		w:heartbeat()
		local frames = w:frames(ann)
		spec.eq(#frames, 2, "snapshot and one update")
		spec.list(World.keys(frames[2]), { "Job.Score" }, "one value")
		spec.eq(World.value(frames[2], "Job.Score"), 3, "the last write")
		w:finish()
	end,
	["60 62 a client effect reading two values written in one server call never sees one without the other"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		local seen = {}
		ann.api.scope(function()
			ann.api.effect(function()
				table.insert(seen, `{ann.d.Phase()} {ann.d.Ends()}`)
			end)
		end)
		w.link:flush()
		w.job.phase:set("Running")
		w.job.ends:set(300)
		w:heartbeat()
		spec.list(seen, { "nil 0", "Running 300" }, "one run with both new values, never one alone")
		w:finish()
	end,
	["T12 the worked update frame of three values is 72 bytes"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		w.job.started:set(12)
		w.job.ends:set(312)
		w.job.phase:set("Running")
		w:heartbeat()
		local frames = w:frames(ann)
		spec.list(World.keys(frames[2]), { "Job.EndsAt", "Job.Phase", "Job.StartedAt" }, "the three values")
		spec.eq(frames[2].bytes, 72, "sync.md's worked frame")
		w:finish()
	end,
	["61 an equal fresh table sends nothing and a changed one sends the whole value"] = function()
		local w = World.new({
			declare = function(r)
				local n = r.n
				return {
					Board = r.emit.replicated(
						"Job.Board",
						{ title = "start", tally = 0 },
						n.object({ { "tally", n.f64 }, { "title", n.string } })
					),
				}
			end,
			serve = function(s, d, world)
				s.service("BoardService", function()
					world.board = s.entry.publish(d.Board)
				end)
			end,
		})
		local ann = w:join("Ann")
		w.board:set({ title = "start", tally = 0 })
		w:heartbeat()
		spec.eq(#w:frames(ann), 1, "an equal fresh table sends nothing")
		w.board:set({ title = "next", tally = 0 })
		w:heartbeat()
		local frames = w:frames(ann)
		spec.eq(#frames, 2, "a changed table sends a frame")
		local value = World.value(frames[2], "Job.Board")
		spec.eq(value.title, "next", "the changed field")
		spec.eq(value.tally, 0, "and the unchanged one: the whole value")
		w:finish()
	end,
	["63 K13 a change to coins keeps settings rawequal on the client, new tables are frozen, and an equal frame writes nothing"] = function()
		local w = World.new({
			declare = function(r)
				local n = r.n
				local settings = n.object({ { "music", n.bool }, { "volume", n.f64 } })
				return {
					Profile = r.emit.replicated(
						"Data.Profile",
						{ coins = 0, settings = { music = true, volume = 1 } },
						n.object({ { "coins", n.f64 }, { "settings", settings } })
					),
				}
			end,
			serve = function(s, d, world)
				s.service("ProfileService", function()
					world.profile = s.entry.publish(d.Profile)
				end)
			end,
		})
		w.profile:set({ coins = 5, settings = { music = false, volume = 0.5 } })
		local ann = w:join("Ann")
		local first = ann.d.Profile()
		spec.eq(table.isfrozen(first), true, "the applied value is frozen")
		spec.eq(table.isfrozen(first.settings), true, "every new table in it")
		local runs, statusRuns = 0, 0
		ann.api.scope(function()
			ann.api.effect(function()
				ann.api.syncStatus(ann.d.Profile)
				statusRuns += 1
			end)
			ann.api.effect(function()
				ann.d.Profile()
				runs += 1
			end)
		end)
		w.link:flush()
		w.profile:set({ coins = 6, settings = { music = false, volume = 0.5 } })
		w:heartbeat()
		local second = ann.d.Profile()
		spec.eq(second.coins, 6, "the new coins")
		spec.eq(rawequal(second.settings, first.settings), true, "settings kept its identity")
		spec.eq(runs, 2, "one rerun for the change")
		w:inject(ann, {
			failed = {},
			kind = "update",
			revision = 3,
			values = { { key = "Data.Profile", value = { coins = 6, settings = { music = false, volume = 0.5 } } } },
		})
		spec.eq(rawequal(ann.d.Profile(), second), true, "an equal frame keeps the stored value")
		spec.eq(runs, 2, "and writes nothing, so nothing reruns")
		spec.eq(ann.api.syncStatus(ann.d.Profile), "ready", "still ready")
		spec.eq(statusRuns, 1, "a status that stays ready is never rewritten, so its readers never rerun")
		w:finish()
	end,
	["64 a key this client never declared is skipped without a diagnostic"] = function()
		local w = newWorld()
		w.job.score:set(4)
		w.job.phase:set("Running")
		local ann = w:join("Ann", {
			declare = function(r)
				return { Score = r.emit.replicated("Job.Score", 0, r.n.f64) }
			end,
		})
		spec.eq(ann.d.Score(), 4, "the declared key applied")
		spec.eq(ann.api.syncStatus(ann.d.Score), "ready", "ready")
		w.job.phase:set("Waiting")
		w.job.score:set(5)
		w:heartbeat()
		spec.eq(ann.d.Score(), 5, "later frames still apply")
		spec.eq(ann.logged("Crucible sync"), 0, "no diagnostic on the client")
		w:finish()
	end,
	["68 a rejoin is a new peer at revision 1 and nothing from the old peer survives"] = function()
		local w = newWorld()
		local ann = w:join("Ann")
		w.job.score:set(1)
		w:heartbeat()
		w.job.score:set(2)
		w:heartbeat()
		spec.eq(w:frames(ann)[3].frame.revision, 3, "the first session reached revision 3")
		w.link:leave(ann)
		w.link:flush()
		w.job.score:set(3)
		w:heartbeat()
		spec.eq(#w:frames(ann), 3, "nothing reaches the left player")
		local again = w:join("Ann")
		local frames = w:frames(again)
		spec.eq(#frames, 4, "one fresh frame after the old session's three on the same name")
		spec.eq(frames[4].frame.revision, 1, "at revision 1")
		spec.eq(frames[4].frame.kind, "snapshot", "a snapshot, not an update")
		spec.eq(again.d.Score(), 3, "current values")
		local players = 0
		for _ in w:sync().peers do
			players += 1
		end
		spec.eq(players, 1, "only the new peer remains")
		spec.eq(#w:sync().order, 1, "and only its view is in the round robin")
		w:finish()
	end,
	["56 ServerSync is constructed first, so the snapshot precedes what game ClientReady callbacks send"] = function()
		local w = World.new({
			declare = function(r)
				local d = declare(r)
				d.Note = r.emit.toClient("Job.Note", r.n.string)
				return d
			end,
			serve = function(s, d, world)
				job(s, d, world)
				s.service("Greeter", function(self)
					self.ready = s.emit.receive(self, "ready", s.ClientReady, function(player)
						s.Network.send(player, d.Note, "hello")
					end)
				end)
			end,
		})
		w.job.score:set(4)
		local notes = {}
		local ann = w:join("Ann", {
			declare = function(c)
				local d = declare(c)
				d.Note = c.emit.toClient("Job.Note", c.n.string)
				c.service("Listener", function(self)
					self.note = c.emit.receive(self, "note", d.Note, function()
						table.insert(notes, c.d.Score())
					end)
				end)
				return d
			end,
		})
		spec.list(notes, { 4 }, "the game message's handler already reads the snapshot's value")
		local order = {}
		for _, entry in
			w.link:sent(function(e)
				return e.to == ann.name and e.opcode == 0
			end)
		do
			local header = w.wire.envelope.readHeader(2, entry.packet, {})
			table.insert(order, if header.index == w.wire.frame.SyncFrameMessage.index then "frame" else "note")
		end
		spec.list(order, { "frame", "note" }, "the snapshot first, then the game's ClientReady message")
		w:finish()
	end,
	["56 a game Subsystem defined before boot still sends its ClientReady message after the snapshot"] = function()
		local function withNote(r)
			local d = declare(r)
			d.Note = r.emit.toClient("Job.Note", r.n.string)
			return d
		end
		local function frameworkBuilt(realm, id: string): boolean
			for _, record in realm.registry.records() do
				if record.id == id then
					return record.state ~= nil and record.state.object ~= nil
				end
			end
			return false
		end
		local serverSaw
		local w = World.new({
			declare = withNote,
			serve = job,
			patch = function(world)
				local server = world.server
				server.service("Early", function(self)
					serverSaw = frameworkBuilt(server, "Crucible.ServerSync")
					self.ready = server.emit.receive(self, "ready", server.ClientReady, function(player)
						server.Network.send(player, server.d.Note, "hello")
					end)
				end)
			end,
		})
		spec.eq(serverSaw, true, "ServerSync was constructed before a game Subsystem defined ahead of it")
		w.job.score:set(4)
		local notes = {}
		local clientSaw
		local ann = w:join("Ann", {
			declare = function(c)
				local d = withNote(c)
				c.service("Listener", function(self)
					self.note = c.emit.receive(self, "note", d.Note, function()
						table.insert(notes, c.d.Score())
					end)
				end)
				return d
			end,
			patch = function(c)
				c.service("EarlyClient", function()
					clientSaw = frameworkBuilt(c, "Crucible.ClientSync")
				end)
			end,
		})
		spec.eq(clientSaw, true, "ClientSync was constructed before a game Subsystem defined ahead of it")
		spec.list(notes, { 4 }, "the game message's handler reads the snapshot's value")
		spec.eq(ann.d.Score(), 4, "the snapshot arrived")
		w:finish()
	end,
	["61 63 a value that loses an element or a key arrives without it, and a grown array arrives longer"] = function()
		local w = World.new({
			declare = function(r)
				local n = r.n
				return {
					List = r.emit.replicated("Shrink.List", {}, n.array(n.f64)),
					Map = r.emit.replicated("Shrink.Map", {}, n.map(n.string, n.f64)),
					Object = r.emit.replicated(
						"Shrink.Object",
						{ a = 1 },
						n.object({ { "a", n.f64 }, { "b", n.union({ n["nil"], n.f64 }) } })
					),
				}
			end,
			serve = function(s, d, world)
				s.service("ShrinkService", function()
					world.list = s.entry.publish(d.List)
					world.map = s.entry.publish(d.Map)
					world.object = s.entry.publish(d.Object)
				end)
			end,
		})
		w.list:set({ 1, 2, 3 })
		w.map:set({ x = 1, y = 2 })
		w.object:set({ a = 1, b = 2 })
		local ann = w:join("Ann")
		w.list:set({ 1, 2 })
		w.map:set({ x = 1 })
		w.object:set({ a = 1 })
		w:heartbeat()
		spec.list(ann.d.List(), { 1, 2 }, "the array lost its last element on the client")
		spec.eq(ann.d.Map().y, nil, "the map lost its key on the client")
		spec.eq(ann.d.Map().x, 1, "and kept the other")
		spec.eq(ann.d.Object().b, nil, "the object lost its optional field on the client")
		w.list:set({ 1, 2, 3, 4 })
		w:heartbeat()
		spec.list(ann.d.List(), { 1, 2, 3, 4 }, "a grown array arrives longer")
		w.list:set({ 1, 2, 3 })
		w:heartbeat()
		spec.list(ann.d.List(), { 1, 2, 3 }, "and shrinks again")
		w:finish()
	end,
}
