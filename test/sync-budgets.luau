local World = require("./lib/sync-world")
local spec = require("./lib/load")

local BIG = string.rep("x", 49152)
local KICK = "Connection problem, please rejoin."

-- Counts the keys `pairs` produces for tables a spec marks, in every realm's modules.
local counted = setmetatable({}, { __mode = "k" })
local function countingPairs(t)
	local iterate, state, first = pairs(t)
	if counted[t] == nil then
		return iterate, state, first
	end
	return function(s, key)
		local nextKey, value = iterate(s, key)
		if nextKey ~= nil then
			table.insert(counted[t], nextKey)
		end
		return nextKey, value
	end,
		state,
		first
end
local function watch(t)
	counted[t] = {}
	return t
end

local function kicked(w, name: string): number
	local n = 0
	for _, kick in w.link.kicks do
		if kick.player == name and kick.message == KICK then
			n += 1
		end
	end
	return n
end

local function kinds(w, client): { string }
	local out = {}
	for _, decoded in w:frames(client) do
		table.insert(out, decoded.frame.kind)
	end
	return out
end

-- A per-player value the server builds from a shared `wanted` state: `wanted()(player)` returns what to project.
local function projected(descriptor)
	return {
		declare = function(r)
			local n = r.n
			return {
				Score = r.emit.replicated("Job.Score", 0, n.f64),
				View = r.emit.replicatedPerPlayer("Data.View", nil, n.union({ n["nil"], descriptor(n) })),
			}
		end,
		serve = function(s, d, w)
			w.wanted = s.api.state(function()
				return nil
			end)
			w.pulls = {}
			s.service("ViewService", function()
				w.view = s.entry.publish(d.View, function(player)
					w.pulls[player.Name] = (w.pulls[player.Name] or 0) + 1
					return w.wanted()(player)
				end)
				w.score = s.entry.publish(d.Score)
			end)
		end,
	}
end

local function want(w, project)
	w.wanted:set(project)
end

-- A record of three 49,152-byte strings and a counter: about 147,500 bytes per frame.
local function blobs(n)
	return n.object({ { "a", n.string }, { "b", n.string }, { "c", n.string }, { "count", n.f64 } })
end

-- One hundred optional number fields, f001 to f100.
local function optionalFields(n)
	local fields = {}
	for i = 1, 100 do
		table.insert(fields, { string.format("f%03d", i), n.union({ n["nil"], n.f64 }) })
	end
	return fields
end

-- Numbers 1..count in a fresh table each call, so a recompute builds an equal value with a new identity.
local function numbers(count: number)
	local out = table.create(count)
	for i = 1, count do
		out[i] = i
	end
	return out
end

return {
	["66 69 a frame over SYNC_FRAME_BYTES fails only that peer, sends the failed frame and nothing partial"] = function()
		local options = projected(blobs)
		local declare = options.declare
		options.declare = function(r)
			local d = declare(r)
			local n = r.n
			d.Extra = r.emit.replicatedPerPlayer("Data.Extra", nil, n.union({ n["nil"], blobs(n) }))
			return d
		end
		local serve = options.serve
		options.serve = function(s, d, w)
			serve(s, d, w)
			s.service("ExtraService", function()
				w.extra = s.entry.publish(d.Extra, function(player)
					return w.wanted()(player)
				end)
			end)
		end
		local w = World.new(options)
		want(w, function(player)
			return if player.Name == "Ann" then { a = BIG, b = BIG, c = BIG, count = 1 } else nil
		end)
		local ann = w:join("Ann")
		local bea = w:join("Bea")
		spec.list(kinds(w, ann), { "failed" }, "only the failed frame reached Ann")
		spec.eq(kicked(w, "Ann"), 1, "Ann was kicked")
		spec.eq(
			w:logged("Crucible sync for Ann failed at revision 0: frame over SYNC_FRAME_BYTES"),
			1,
			"one diagnostic"
		)
		spec.eq(w:logged("Data.Extra, Data.View"), 1, "naming the declarations in the frame")
		spec.list(kinds(w, bea), { "snapshot" }, "Bea synced")
		spec.eq(kicked(w, "Bea"), 0, "Bea stays")
		w:finish()
	end,
	["70 58 K6 with frames over SYNC_HEARTBEAT_BYTES later peers get their frame next Heartbeat carrying every change since"] = function()
		local w = World.new(projected(blobs))
		local count = w.server.api.state(0)
		want(w, function()
			return { a = BIG, b = BIG, c = BIG, count = count() }
		end)
		local clients = {}
		for i = 1, 8 do
			clients[i] = w:join(`P{i}`)
		end
		local last = clients[8]
		local declaration = w.server.declarations.declarationOf(w.server.d.View)
		local before = w:sync().peers[last.player].accepted[declaration].version
		w.score:set(1)
		count:set(1)
		w:heartbeat()
		for i = 1, 7 do
			spec.eq(#w:frames(clients[i]), 2, `P{i} got its update`)
		end
		spec.eq(#w:frames(last), 1, "the eighth frame would pass the byte budget, so it waits")
		spec.eq(
			w:sync().peers[last.player].accepted[declaration].version,
			before,
			"a deferred peer keeps its baseline exactly"
		)
		count:set(2)
		w:heartbeat()
		local frames = w:frames(last)
		spec.eq(#frames, 2, "the deferred peer goes first next Heartbeat")
		spec.list(World.keys(frames[2]), { "Data.View", "Job.Score" }, "carrying every change since its baseline")
		spec.eq(World.value(frames[2], "Data.View").count, 2, "with current values")
		spec.eq(last.d.Score(), 1, "the earlier public write included")
		w:finish()
	end,
	["70 K6 with more ready peers than SYNC_HEARTBEAT_PEERS a Heartbeat examines 64 and the next starts at the 65th"] = function()
		local w = World.new(projected(function(n)
			return n.array(n.f64)
		end))
		local bump = w.server.api.state(0)
		want(w, function()
			bump()
			return { 1, 2, 3 }
		end)
		local clients = {}
		for i = 1, 65 do
			clients[i] = w:join(`P{i}`)
		end
		local declaration = w.server.declarations.declarationOf(w.server.d.View)
		local base = w:sync().peers[clients[65].player].accepted[declaration].version
		local pulls = table.clone(w.pulls)
		bump:set(1)
		w:heartbeat()
		local examined = 0
		for i = 1, 65 do
			if w.pulls[`P{i}`] > pulls[`P{i}`] then
				examined += 1
			end
		end
		spec.eq(examined, 64, "one Heartbeat examines SYNC_HEARTBEAT_PEERS peers")
		spec.eq(w.pulls.P65, pulls.P65, "the 65th was not reached")
		spec.eq(w:sync().peers[clients[65].player].accepted[declaration].version, base, "and kept its baseline")
		for i = 1, 65 do
			spec.eq(#w:frames(clients[i]), 1, `an equal rebuilt value sends nothing to P{i}`)
		end
		pulls = table.clone(w.pulls)
		w:heartbeat()
		spec.eq(w.pulls.P65, pulls.P65 + 1, "the next Heartbeat starts at the 65th")
		w:finish()
	end,
	["70 R2 two peers needing 40,001 visits each both run, the third waits, and goes first next Heartbeat"] = function()
		local w = World.new(projected(function(n)
			return n.array(n.f64)
		end))
		local bump = w.server.api.state(0)
		want(w, function()
			bump()
			return numbers(9999)
		end)
		local clients = {}
		for i = 1, 3 do
			clients[i] = w:join(`P{i}`)
		end
		local pulls = table.clone(w.pulls)
		bump:set(1)
		w:heartbeat()
		-- Rule 70's count per peer: two declarations considered; admission of the union node, the array node, 9,999
		-- keys scanned and 9,999 element nodes; comparison of one node pair, then an index and a node pair per element.
		local peer = 2 + (1 + 1 + 9999 + 9999) + (1 + 2 * 9999)
		spec.eq(w:sync().spent, 2 * peer, "the Heartbeat spent both peers' examinations")
		spec.eq(w.pulls.P3, pulls.P3, "80,002 is not below 65,536, so the third did not start")
		pulls = table.clone(w.pulls)
		bump:set(2)
		w:heartbeat()
		spec.eq(w.pulls.P3, pulls.P3 + 1, "the third goes first next Heartbeat")
		w:finish()
	end,
	["70 R2 a first peer needing more than SYNC_HEARTBEAT_VISITS still completes and the next peer waits"] = function()
		local map = {}
		for i = 1, 20000 do
			map[`k{i}`] = i
		end
		local options = projected(function(n)
			return n.map(n.string, n.f64)
		end)
		local declare = options.declare
		options.declare = function(r)
			local d = declare(r)
			local n = r.n
			d.Second = r.emit.replicatedPerPlayer("Data.Second", nil, n.union({ n["nil"], n.map(n.string, n.f64) }))
			d.Third = r.emit.replicatedPerPlayer("Data.Third", nil, n.union({ n["nil"], n.map(n.string, n.f64) }))
			return d
		end
		local serve = options.serve
		options.serve = function(s, d, w)
			serve(s, d, w)
			s.service("MoreService", function()
				for _, key in { "Second", "Third" } do
					s.entry.publish(d[key], function(player)
						return w.wanted()(player)
					end)
				end
			end)
		end
		local w = World.new(options)
		local bump = w.server.api.state(0)
		want(w, function(player)
			bump()
			return if player.Name == "P1" then table.clone(map) else nil
		end)
		local first = w:join("P1")
		w:join("P2")
		spec.eq(first.api.syncStatus(first.d.View), "failed", "P1's oversized values are failed, not its peer")
		local pulls = table.clone(w.pulls)
		bump:set(1)
		w:heartbeat()
		local spent = w:sync().spent
		assert(spent > 65536, `the first peer spent {spent}, past the start threshold`)
		assert(spent <= 65535 + 262144, "within the Heartbeat bound")
		spec.eq(kicked(w, "P1"), 0, "the first peer completed instead of failing")
		spec.eq(w.pulls.P2, pulls.P2, "the second peer did not start")
		w:finish()
	end,
	["66 70 R2 a peer passing SYNC_PEER_VISITS fails terminally at the snapshot and at a Heartbeat with nothing partial"] = function()
		local map = {}
		for i = 1, 20000 do
			map[`k{i}`] = i
		end
		local w = World.new({
			declare = function(r)
				local n = r.n
				local d = {}
				for i = 1, 11 do
					d[i] = r.emit.replicatedPerPlayer(`Data.M{i}`, nil, n.union({ n["nil"], n.map(n.string, n.f64) }))
				end
				return d
			end,
			serve = function(s, d, world)
				world.big = s.api.state({})
				s.service("MapService", function()
					for i = 1, 11 do
						s.entry.publish(d[i], function(player)
							return if world.big()[player.Name] then map else nil
						end)
					end
				end)
			end,
		})
		w.big:set({ Ann = true })
		local ann = w:join("Ann")
		spec.list(kinds(w, ann), { "failed" }, "the snapshot's examination failed: only the failed frame")
		spec.eq(kicked(w, "Ann"), 1, "Ann was kicked")
		spec.eq(w:logged("examination over SYNC_PEER_VISITS"), 1, "one diagnostic")
		local bea = w:join("Bea")
		spec.list(kinds(w, bea), { "snapshot" }, "Bea's small snapshot")
		w.big:set({ Bea = true })
		w:heartbeat()
		spec.list(kinds(w, bea), { "snapshot", "failed" }, "the Heartbeat examination failed: no partial update")
		spec.eq(kicked(w, "Bea"), 1, "Bea was kicked")
		assert(w:sync().spent <= 65535 + 262144, "the Heartbeat stayed within its bound")
		w:finish()
	end,
	["49 K6 R1 admission stops at its first budget crossing and touches at most one undeclared key"] = function()
		local w = World.new({
			globals = { pairs = countingPairs },
			declare = function(r)
				local n = r.n
				return {
					Board = r.emit.replicated("Job.Board", { title = "" }, n.object({ { "title", n.string } })),
					List = r.emit.replicated("Job.List", {}, n.array(n.f64)),
					Tally = r.emit.replicated("Job.Tally", {}, n.map(n.string, n.f64)),
					Rows = r.emit.replicated("Job.Rows", {}, n.array(n.object(optionalFields(n)))),
				}
			end,
			serve = function(s, d, world)
				s.service("BoardService", function()
					world.board = s.entry.publish(d.Board)
					world.list = s.entry.publish(d.List)
					world.tally = s.entry.publish(d.Tally)
					world.rows = s.entry.publish(d.Rows)
				end)
			end,
		})
		local record = {}
		for i = 1, 20000 do
			record[`k{i}`] = i
		end
		watch(record)
		local ok, err = pcall(function()
			w.tally:set(record)
		end)
		spec.eq(ok, false, "a 20,000-entry record is rejected")
		spec.has(err, "Job.Tally: over SYNC_FRAME_COLLECTION_ENTRIES", "at the entry budget")
		assert(
			#counted[record] <= 12001,
			`the enumeration produced {#counted[record]} keys, at most the budget plus one`
		)
		local wide = { title = "kept" }
		for i = 1, 20000 do
			wide[`extra{i}`] = i
		end
		watch(wide)
		w.board:set(wide)
		local undeclared = 0
		for _, key in counted[wide] do
			if key ~= "title" then
				undeclared += 1
			end
		end
		assert(undeclared <= 1, `touched {undeclared} undeclared keys`)
		spec.eq(w.server.d.Board().title, "kept", "the object is admitted")
		spec.eq(w.server.d.Board().extra1, nil, "without its undeclared keys")
		local long = watch(numbers(12001))
		ok, err = pcall(function()
			w.list:set(long)
		end)
		spec.eq(ok, false, "an array past the entry budget is rejected")
		spec.has(err, "Job.List: over SYNC_FRAME_COLLECTION_ENTRIES", "by its length")
		spec.eq(#counted[long], 0, "before any key is iterated")
		-- Each row costs one codec visit but 102 admission visits: its node, one scan step to its undeclared key, then a
		-- lookup by name for each of its 100 absent declared fields. 1,000 rows pass 96,000 inside every codec budget.
		local rows = table.create(1000)
		for i = 1, 1000 do
			rows[i] = { junk = true }
		end
		ok, err = pcall(function()
			w.rows:set(rows)
		end)
		spec.eq(ok, false, "a value within the codec budgets can still pass the admission budget")
		spec.has(err, "Job.Rows", "naming the declaration")
		spec.has(err, "over SYNC_ADMISSION_VISITS", "at the admission budget")
		w:finish()
	end,
	["67 send false advances no revision and ends the view before networking's deferred cleanup"] = function()
		local w = World.new(projected(function(n)
			return n.f64
		end))
		want(w, function()
			return nil
		end)
		local ann = w:join("Ann")
		local view = w:sync().peers[ann.player]
		w.score:set(5)
		w.link:leave(ann)
		local deferred = #w.link.deferred
		assert(deferred > 0, "networking deferred the ClientReady cleanup")
		w.server.engine.frame(5, 1 / 60)
		spec.eq(view.live, false, "send returned false and the view ended at once")
		spec.eq(view.revision, 1, "no revision advanced")
		spec.eq(w:sync().peers[ann.player], nil, "removed before the deferred cleanup ran")
		spec.eq(#w.link.deferred, deferred, "the cleanup is still pending")
		w.link:flush()
		w:finish()
	end,
	["66 69 encode refused on a preflighted frame fails the peer, keeps its baseline, and kicks"] = function()
		local w = World.new(projected(function(n)
			return n.f64
		end))
		want(w, function()
			return nil
		end)
		local ann = w:join("Ann")
		local view = w:sync().peers[ann.player]
		local declaration = w.server.declarations.declarationOf(w.server.d.Score)
		local version = view.accepted[declaration].version
		w.score:set(5)
		w.refuse = 1
		w:heartbeat()
		spec.eq(w:logged("encode refused a preflighted frame"), 1, "reported")
		spec.eq(view.accepted[declaration].version, version, "nothing staged was committed")
		spec.list(kinds(w, ann), { "snapshot", "failed" }, "only the failed frame followed")
		spec.eq(kicked(w, "Ann"), 1, "kicked")
		w:finish()
	end,
	["59 66 a frame that would pass SYNC_MAX_REVISION fails the peer instead of wrapping"] = function()
		local w = World.new(projected(function(n)
			return n.f64
		end))
		want(w, function()
			return nil
		end)
		local ann = w:join("Ann")
		w:sync().peers[ann.player].revision = 4294967295
		w.score:set(5)
		w:heartbeat()
		spec.eq(w:logged("revision passes SYNC_MAX_REVISION"), 1, "reported")
		spec.list(kinds(w, ann), { "snapshot", "failed" }, "no update")
		spec.eq(kicked(w, "Ann"), 1, "kicked")
		w:finish()
	end,
	["58 70 a Heartbeat with nothing written since the last settled one examines no peer"] = function()
		local w = World.new(projected(function(n)
			return n.f64
		end))
		want(w, function()
			return 1
		end)
		w:join("Ann")
		w:join("Bea")
		w.score:set(2)
		w:heartbeat()
		assert(w:sync().spent > 0, "a Heartbeat after a write examines its peers")
		w:heartbeat()
		spec.eq(w:sync().spent, 0, "the next, with nothing written, examines none")
		w.score:set(3)
		w:heartbeat()
		assert(w:sync().spent > 0, "a later write is examined again")
		w:finish()
	end,
	["69 frameCost counts 130 values with a two-byte list length, as networking's encoder does"] = function()
		local w = World.new({
			declare = function(r)
				local d = {}
				for i = 1, 130 do
					d[i] = r.emit.replicated(string.format("Wide.D%03d", i), 0, r.n.f64)
				end
				return d
			end,
			serve = function(s, d, world)
				s.service("WideService", function()
					world.pubs = {}
					for i, declaration in d do
						world.pubs[i] = s.entry.publish(declaration)
					end
				end)
			end,
		})
		local ann = w:join("Ann")
		for i, publication in w.pubs do
			publication:set(i)
		end
		w:heartbeat()
		local frames = w:frames(ann)
		spec.eq(#frames, 2, "a snapshot and one update")
		spec.eq(#frames[2].frame.values, 130, "the update carries all 130 values")
		spec.eq(ann.d[130](), 130, "and the client applied them")
		spec.eq(w:finish(), 2, "frameCost matched the encoder on both frames")
	end,
}
