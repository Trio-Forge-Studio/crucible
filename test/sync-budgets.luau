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
			w.wanted = s.charm.atom(function()
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
	w.wanted(function()
		return project
	end)
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

-- Rule 70's byte bound: eight peers each get one 131,072-byte update frame, so the Heartbeat's frames total exactly
-- SYNC_HEARTBEAT_BYTES (1,048,576). `extra` lengthens the last peer's frame by that many bytes.
local PEER_FRAME = 131072
local FRAME_TAIL = 32713
local function byteLimitWorld(extra: number)
	local long = string.rep("a", 49152)
	local w = World.new({
		declare = function(r)
			local n = r.n
			local record = n.object({ { "a", n.string }, { "b", n.string }, { "c", n.string }, { "count", n.f64 } })
			return { View = r.emit.replicatedPerPlayer("Data.View", nil, n.union({ n["nil"], record })) }
		end,
		serve = function(s, d, world)
			world.count = s.charm.atom(0)
			s.service("ViewService", function()
				s.entry.publish(d.View, function(player)
					local tail = FRAME_TAIL + (if player.Name == "P8" then extra else 0)
					return { a = long, b = long, c = string.rep("c", tail), count = world.count() }
				end)
			end)
		end,
	})
	local clients = {}
	for i = 1, 8 do
		clients[i] = w:join(`P{i}`)
	end
	return w, clients
end

-- Rule 49 counts one admission visit per value node and per table key touched. A row `{ junk = true }` of an object with
-- `absent` optional fields costs 1 for its node, 1 for the scan step that stops at its undeclared key and 1 lookup for
-- each declared field: 2 + absent. An array of `rows` such rows costs 1 for its node, 1 per key scanned and the rows.
local function rowDescriptor(n, absent: number)
	local fields = {}
	for i = 1, absent do
		table.insert(fields, { string.format("f%02d", i), n.union({ n["nil"], n.f64 }) })
	end
	return n.object(fields)
end

local function junkRows(rows: number)
	local out = table.create(rows)
	for i = 1, rows do
		out[i] = { junk = true }
	end
	return out
end

-- A world with one published array of rows under rule 49's count; returns whether `set` took `rows` rows.
local function admitRows(rows: number, absent: number)
	local w = World.new({
		declare = function(r)
			local n = r.n
			return { Rows = r.emit.replicated("Job.Rows", {}, n.array(rowDescriptor(n, absent))) }
		end,
		serve = function(s, d, world)
			s.service("RowsService", function()
				world.rows = s.entry.publish(d.Rows)
			end)
		end,
	})
	local ok, err = pcall(function()
		w.rows(junkRows(rows))
	end)
	return w, ok, err
end

-- Rule 70's per-peer count for a rebuilt, equal value: one visit for a declaration considered, then rule 49's count
-- for the value, then rule 61's: one for the array's node pair and two per row (the index and the row's node pair).
-- Each of three per-player views holds `rows` rows of `absent` optional fields behind a nil union (one more node).
local function peerWorld(rows: number, absent: number, publics: number)
	return World.new({
		declare = function(r)
			local n = r.n
			local view = n.union({ n["nil"], n.array(rowDescriptor(n, absent)) })
			local d = {}
			for i = 1, 3 do
				d[i] = r.emit.replicatedPerPlayer(`Data.V{i}`, nil, view)
			end
			for i = 1, publics do
				d[3 + i] = r.emit.replicated(`Job.P{i}`, 0, n.f64)
			end
			return d
		end,
		serve = function(s, d, world)
			world.bump = s.charm.atom(0)
			s.service("ViewsService", function()
				for i = 1, 3 do
					s.entry.publish(d[i], function()
						world.bump()
						return junkRows(rows)
					end)
				end
				for i = 1, publics do
					s.entry.publish(d[3 + i])
				end
			end)
		end,
	})
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
		local count = w.server.charm.atom(0)
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
		w.score(1)
		count(1)
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
		count(2)
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
		local bump = w.server.charm.atom(0)
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
		bump(1)
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
		local bump = w.server.charm.atom(0)
		want(w, function()
			bump()
			return numbers(9999)
		end)
		local clients = {}
		for i = 1, 3 do
			clients[i] = w:join(`P{i}`)
		end
		local pulls = table.clone(w.pulls)
		bump(1)
		w:heartbeat()
		-- Rule 70's count per peer: two declarations considered; admission of the union node, the array node, 9,999
		-- keys scanned and 9,999 element nodes; comparison of one node pair, then an index and a node pair per element.
		local peer = 2 + (1 + 1 + 9999 + 9999) + (1 + 2 * 9999)
		spec.eq(w:sync().spent, 2 * peer, "the Heartbeat spent both peers' examinations")
		spec.eq(w.pulls.P3, pulls.P3, "80,002 is not below 65,536, so the third did not start")
		pulls = table.clone(w.pulls)
		bump(2)
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
		local bump = w.server.charm.atom(0)
		want(w, function(player)
			bump()
			return if player.Name == "P1" then table.clone(map) else nil
		end)
		local first = w:join("P1")
		w:join("P2")
		spec.eq(first.api.syncStatus(first.d.View), "failed", "P1's oversized values are failed, not its peer")
		local pulls = table.clone(w.pulls)
		bump(1)
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
				world.big = s.charm.atom({})
				s.service("MapService", function()
					for i = 1, 11 do
						s.entry.publish(d[i], function(player)
							return if world.big()[player.Name] then map else nil
						end)
					end
				end)
			end,
		})
		w.big({ Ann = true })
		local ann = w:join("Ann")
		spec.list(kinds(w, ann), { "failed" }, "the snapshot's examination failed: only the failed frame")
		spec.eq(kicked(w, "Ann"), 1, "Ann was kicked")
		spec.eq(w:logged("examination over SYNC_PEER_VISITS"), 1, "one diagnostic")
		local bea = w:join("Bea")
		spec.list(kinds(w, bea), { "snapshot" }, "Bea's small snapshot")
		w.big({ Bea = true })
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
			w.tally(record)
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
		w.board(wide)
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
			w.list(long)
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
			w.rows(rows)
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
		w.score(5)
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
		w.score(5)
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
		w.score(5)
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
		w.score(2)
		w:heartbeat()
		assert(w:sync().examined > 0, "a Heartbeat after a write examines its peers")
		w:heartbeat()
		spec.eq(w:sync().examined, 0, "the next, with nothing written, examines none")
		w.score(3)
		w:heartbeat()
		assert(w:sync().examined > 0, "a later write is examined again")
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
			publication(i)
		end
		w:heartbeat()
		local frames = w:frames(ann)
		spec.eq(#frames, 2, "a snapshot and one update")
		spec.eq(#frames[2].frame.values, 130, "the update carries all 130 values")
		spec.eq(ann.d[130](), 130, "and the client applied them")
		spec.eq(w:finish(), 2, "frameCost matched the encoder on both frames")
	end,
	["69 frameCost counts 127 and 128 values at the one-byte and two-byte list-length boundary, as networking's encoder does"] = function()
		local w = World.new({
			declare = function(r)
				local d = {}
				for i = 1, 128 do
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
		for i = 2, 128 do
			w.pubs[i](i)
		end
		w:heartbeat()
		for i = 1, 128 do
			w.pubs[i](i + 1000)
		end
		w:heartbeat()
		local frames = w:frames(ann)
		spec.eq(#frames, 3, "a snapshot and two updates")
		spec.eq(#frames[1].frame.values, 128, "the snapshot carries all 128 values")
		spec.eq(#frames[2].frame.values, 127, "the first update changed 127 of them")
		spec.eq(#frames[3].frame.values, 128, "the second changed all 128")
		-- Every entry is a 10-byte key and a 9-byte number, so one more entry adds 19 bytes, and the list length 128
		-- needs a second varuint byte where 127 needs one.
		spec.eq(frames[3].bytes - frames[2].bytes, 19 + 1, "the 128th entry and the second length byte")
		spec.eq(w:finish(), 3, "frameCost matched the encoder on all three frames")
	end,
	["58 70 a Heartbeat that stopped for the byte budget completes the next one, and the one after examines no peer"] = function()
		local w, clients = byteLimitWorld(1)
		w.count(1)
		w:heartbeat()
		spec.eq(#w:frames(clients[8]), 1, "the eighth peer's frame waited for the byte budget")
		w:heartbeat()
		spec.eq(#w:frames(clients[8]), 2, "the next Heartbeat sent it")
		assert(w:sync().examined > 0, "and examined the peers")
		w:heartbeat()
		spec.eq(w:sync().examined, 0, "a Heartbeat that completed every peer settles, so the next examines none")
		w:finish()
	end,
	["70 a Heartbeat whose frames total exactly SYNC_HEARTBEAT_BYTES sends every one, and one byte more defers the last"] = function()
		local w, clients = byteLimitWorld(0)
		w.count(1)
		w:heartbeat()
		for i = 1, 8 do
			local frames = w:frames(clients[i])
			spec.eq(#frames, 2, `P{i} got its update`)
			spec.eq(frames[2].bytes, PEER_FRAME, `P{i}'s update is 131,072 bytes`)
		end
		w:finish()
		local over, others = byteLimitWorld(1)
		over.count(1)
		over:heartbeat()
		for i = 1, 7 do
			spec.eq(#over:frames(others[i]), 2, `P{i} got its update`)
		end
		spec.eq(#over:frames(others[8]), 1, "1,048,577 bytes pass the budget, so the eighth waits")
		over:heartbeat()
		local frames = over:frames(others[8])
		spec.eq(#frames, 2, "and goes next Heartbeat")
		spec.eq(frames[2].bytes, PEER_FRAME + 1, "as a frame one byte longer")
		over:finish()
	end,
	["49 R1 an array whose admission visits total exactly SYNC_ADMISSION_VISITS is admitted, and one more visit refuses it"] = function()
		-- 1 + 5647 * (3 + 14) = 96,000 and 1 + 6000 * (3 + 13) = 96,001, both inside every codec budget.
		local w, ok = admitRows(5647, 14)
		spec.eq(ok, true, "96,000 admission visits are admitted")
		spec.eq(#w.server.d.Rows(), 5647, "the server holds the value")
		local ann = w:join("Ann")
		spec.eq(#ann.d.Rows(), 5647, "and the snapshot delivered it")
		w:finish()
		local over, refused, err = admitRows(6000, 13)
		spec.eq(refused, false, "96,001 admission visits are refused")
		spec.has(err, "Job.Rows", "naming the declaration")
		spec.has(err, "over SYNC_ADMISSION_VISITS", "at the admission budget")
		spec.eq(#over.server.d.Rows(), 0, "the server keeps its last value")
		over:finish()
	end,
	["70 a peer examination spending exactly SYNC_PEER_VISITS completes, and one more visit fails the peer"] = function()
		-- Per view: 1 considered + (1 union + 1 array + 3799 keys + 3799 * (2 + 18)) admission + (1 + 2 * 3799) comparison,
		-- which is 87,381; three views and one public declaration considered make 262,144.
		local w = peerWorld(3799, 18, 1)
		local ann = w:join("Ann")
		w.bump(1)
		w:heartbeat()
		spec.eq(w:sync().spent, 262144, "the Heartbeat spent exactly SYNC_PEER_VISITS")
		spec.eq(w:logged("examination over SYNC_PEER_VISITS"), 0, "and the peer was not failed")
		spec.eq(ann.api.syncStatus(ann.d[1]), "ready", "its views stay ready")
		spec.eq(#w:frames(ann), 1, "nothing changed, so no update")
		w:finish()
		local over = peerWorld(3799, 18, 2)
		local bea = over:join("Bea")
		over.bump(1)
		over:heartbeat()
		spec.eq(over:logged("examination over SYNC_PEER_VISITS after 262144 visits"), 1, "262,145 visits fail the peer")
		spec.list(kinds(over, bea), { "snapshot", "failed" }, "with only the failed frame after the snapshot")
		spec.eq(kicked(over, "Bea"), 1, "and kick it")
		over:finish()
	end,
}
