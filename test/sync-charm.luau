local World = require("./lib/sync-world")
local spec = require("./lib/load")

local function newWorld()
	return World.new({
		declare = function(r)
			return {
				Shared = r.emit.replicated("Trial.Shared", {}, r.n.map(r.n.string, r.n.f64)),
				Private = r.emit.replicatedPerPlayer("Trial.Private", 0, r.n.f64),
				Strings = r.emit.replicated("Trial.Strings", {}, r.n.map(r.n.string, r.n.string)),
			}
		end,
		serve = function(s, d, w)
			w.version = s.charm.atom(0)
			w.pulls = {}
			s.service("Trial", function()
				w.shared = s.entry.publish(d.Shared)
				w.strings = s.entry.publish(d.Strings)
				s.entry.publish(d.Private, function(player)
					w.pulls[player.Name] = (w.pulls[player.Name] or 0) + 1
					return player.UserId + w.version()
				end)
			end)
		end,
	})
end

return {
	["S6 deleting a key at the admitted depth limit fits the codec depth limit"] = function()
		local function value(leaf)
			for _ = 1, 28 do
				leaf = { child = leaf }
			end
			return leaf
		end
		local w = World.new({
			declare = function(r)
				local node = r.n.union({ r.n["nil"], r.n.f64 })
				for _ = 1, 28 do
					node = r.n.object({ { "child", node } })
				end
				return r.emit.replicated("Trial.Deep", value(1), node)
			end,
			serve = function(s, d, world)
				s.service("Deep", function()
					world.write = s.entry.publish(d)
				end)
			end,
		})
		local ann = w:join("Ann", { userId = 100 })
		w.write(value(nil))
		w:heartbeat()
		local leaf = ann.d()
		for _ = 1, 28 do
			leaf = leaf.child
		end
		spec.eq(leaf, nil, "the deepest key was deleted")
		spec.eq(ann.api.syncStatus(ann.d), "ready", "the client remains ready")
		w:finish()
	end,
	["S6 unchanged subtrees with escaped keys retain client identity"] = function()
		local w = World.new({
			declare = function(r)
				return r.emit.replicated(
					"Trial.Nested",
					{ keep = {}, count = 0 },
					r.n.object({
						{ "count", r.n.f64 },
						{ "keep", r.n.map(r.n.string, r.n.string) },
					})
				)
			end,
			serve = function(s, d, world)
				s.service("Nested", function()
					world.write = s.entry.publish(d)
				end)
			end,
		})
		local keep = { __none = "__none" }
		w.write({ keep = keep, count = 0 })
		local ann = w:join("Ann", { userId = 100 })
		local previous = ann.d().keep
		w.write({ keep = keep, count = 1 })
		w:heartbeat()
		spec.eq(ann.d().keep, previous, "an unchanged escaped subtree keeps its identity")
		w:finish()
	end,
	["S6 a mid-session join receives every subscribed value in the first payload, with private values isolated"] = function()
		local w = newWorld()
		local ann = w:join("Ann", { userId = 100 })
		w.shared({ score = 42 })
		w.strings({ phase = "Running" })
		w:heartbeat()
		local bea = w:join("Bea", { userId = 200 })
		local first = w:frames(bea)
		spec.eq(#first, 1, "one initial payload")
		spec.list(World.keys(first[1]), { "Trial.Private", "Trial.Shared", "Trial.Strings" }, "every subscription")
		spec.eq(bea.d.Shared().score, 42, "current shared value")
		spec.eq(bea.d.Strings().phase, "Running", "current string value")
		for _, client in { ann, bea } do
			for _, frame in w:frames(client) do
				local value, present = World.value(frame, "Trial.Private")
				if present then
					spec.eq(value, client.player.UserId, "only the recipient's private value reaches the wire")
				end
			end
		end
		w:finish()
	end,
	["S6 deleting a key yields undefined and 1000 writes coalesce into one frame within the byte budget"] = function()
		local w = newWorld()
		w.shared({ removed = 7 })
		local ann = w:join("Ann", { userId = 100 })
		for i = 1, 1000 do
			w.shared({ score = i })
		end
		w:heartbeat()
		spec.eq(ann.d.Shared().removed, nil, "removed server key is undefined")
		spec.eq(ann.d.Shared().score, 1000, "the final write")
		local frames = w:frames(ann)
		spec.eq(#frames, 2, "snapshot and one update")
		assert(frames[2].bytes <= w.server.reactiveLimits.SYNC_FRAME_BYTES, "1000 writes stay within the frame budget")
		w:finish()
	end,
	["S6 leaving removes the subscription and stops every private projection"] = function()
		local w = newWorld()
		local ann = w:join("Ann", { userId = 100 })
		local bea = w:join("Bea", { userId = 200 })
		local view = w:sync().peers[ann.player]
		w.link:leave(ann)
		w.link:flush()
		spec.eq(w:sync().peers[ann.player], nil, "no server subscription")
		spec.eq(next(view.projections), nil, "all projections removed")
		local pulls = w.pulls.Ann
		w.version(1)
		w:heartbeat()
		spec.eq(w.pulls.Ann, pulls, "the departed player's projection does not run")
		spec.eq(bea.d.Private(), bea.player.UserId + 1, "the remaining player keeps syncing")
		w:finish()
	end,
	["S6 a malformed patch sent by a client is ignored"] = function()
		local w = newWorld()
		w.shared({ score = 42 })
		local ann = w:join("Ann", { userId = 100 })
		local message = w.wire.frame.SyncFrameMessage
		local body = w.wire.envelope.encodeBody(message.name, message.input, "payload", {
			failed = {},
			kind = "update",
			revision = 2,
			values = { { key = "Trial.Shared", mode = "patch", value = "malformed patch" } },
		})
		local packet = w.wire.envelope.frame(body, 0, { message.index })
		w.link:inject(ann, packet.bytes, packet.sidecar)
		w.link:flush()
		spec.eq(w.server.d.Shared().score, 42, "server value unchanged")
		spec.eq(ann.d.Shared().score, 42, "client value unchanged")
		w.shared({ score = 43 })
		w:heartbeat()
		spec.eq(ann.d.Shared().score, 43, "legitimate server patches still arrive")
		w:finish()
	end,
	["S6 marker keys and escape-prefixed keys remain ordinary game data through updates and deletions"] = function()
		local w = newWorld()
		w.strings({ __none = "__none", __array = "data", ["\0__none"] = "escaped", ["\0\0__none"] = "twice" })
		local ann = w:join("Ann", { userId = 100 })
		spec.eq(ann.d.Strings().__none, "__none", "the deletion-marker shape is data")
		spec.eq(ann.d.Strings()["\0__none"], "escaped", "a colliding escaped key is data")
		spec.eq(ann.d.Strings()["\0\0__none"], "twice", "a second escaped key is data")
		w.strings({ __none = "__none", __array = "updated", ["\0__none"] = "changed" })
		w:heartbeat()
		spec.eq(ann.d.Strings().__none, "__none", "a patch retains the marker key")
		spec.eq(ann.d.Strings().__array, "updated", "the array marker key is data")
		spec.eq(ann.d.Strings()["\0__none"], "changed", "the escaped key updates")
		spec.eq(ann.d.Strings()["\0\0__none"], nil, "the other escaped key is removed")
		w:finish()
	end,
}
