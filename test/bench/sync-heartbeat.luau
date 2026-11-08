-- Time per quiet Heartbeat with 64 peers, each holding 256 per-player projections. Prints one JSON line.
-- `lune run test/bench/sync-heartbeat.luau` measures the compiled runtime in packages/crucible/out. The peers join
-- through ServerSync's snapshot with a sender that takes every frame, so no client realm is built.
local serde = require("@lune/serde")
local SyncWorld = require("../lib/sync-world")

local PEERS = 64
local DECLARATIONS = 256
local HEARTBEATS = 600
local WARMUP = 60

local w = SyncWorld.new({
	declare = function(r)
		local declared = {}
		for i = 1, DECLARATIONS do
			declared[i] = r.emit.replicatedPerPlayer(`Bench.Value{i}`, 0, r.n.f64)
		end
		return declared
	end,
	serve = function(s, d, world)
		world.x = s.charm.atom(0)
		s.service("Publisher", function()
			for i = 1, DECLARATIONS do
				s.entry.publish(d[i], function()
					return world.x() + i
				end)
			end
		end)
	end,
})
w.server.graph.module("internal/sync/server-sync").installServerSync(function()
	return true
end)
local sync = w:sync()
for i = 1, PEERS do
	sync:snapshot({ Name = `Peer{i}`, UserId = i })
end
for _ = 1, WARMUP do
	w:heartbeat()
end
assert(#sync.order == PEERS, `every peer joined: {#sync.order}`)
for _, view in sync.order do
	local watched = 0
	for _, projection in view.projections do
		if projection.node ~= nil and not projection.dirty then
			watched += 1
		end
	end
	assert(watched == DECLARATIONS, `every projection was pulled and is watched: {watched}`)
end
local started = os.clock()
for _ = 1, HEARTBEATS do
	w:heartbeat()
end
local elapsed = os.clock() - started

print(serde.encode("json", {
	peers = PEERS,
	projections = DECLARATIONS,
	usPerHeartbeat = elapsed / HEARTBEATS * 1e6,
	spent = sync.spent,
}))
