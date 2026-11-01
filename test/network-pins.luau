local Link = require("./lib/network-link")
local networkPort = require("./lib/network-port")
local spec = require("./lib/load")

local function declare(r)
	return { Ping = r.emit.toServer("Pins.Ping", r.n.string) }
end

return {
	["79 A17 a throwing undo of booted is reported once and the failed boot still rolls back"] = function()
		local link = Link.new()
		assert(link:boot(function(s)
			declare(s)
		end))
		local owner
		local c = link:join("Ann", function(realm)
			declare(realm)
			realm.service("Owner", function()
				owner = realm.construction.readClaim()
			end)
		end, { boot = false })
		local store = {}
		c.attributes = setmetatable({}, {
			__index = store,
			__newindex = function(_, key, value)
				if value == nil then
					error("undo threw")
				end
				store[key] = value
			end,
		})
		link:failNext("fireServer")
		link:start(c)
		spec.eq(c.booted, false, "the commit failed")
		spec.has(c.bootError, "boot finalization failed: ", "the finalization failure, not the undo's")
		spec.eq(c.logged("Crucible: undoing booted failed: "), 1, "one diagnostic for the undo")
		spec.eq(c.life.hasEnded(owner), true, "the boot generation still rolled back")
	end,
	["78 A17 closing a committed server session twice keeps its claims and sealed indices"] = function()
		local link = Link.new()
		local m
		assert(link:boot(function(s)
			m = declare(s)
			s.service("Owner", function(self)
				self.ping = s.emit.receive(self, "ping", m.Ping, function() end)
			end)
		end))
		local s = link.server
		local index = m.Ping.index
		s.closers[1]()
		s.closers[1]()
		spec.eq(m.Ping.index, index, "the index outlives a second close")
		spec.eq(s.catalog.catalogSize(), 3, "so does the sealed catalog")
		spec.eq(s.claims.receiverOf(m.Ping) ~= nil, true, "and the receiver claim")
		link:assertClean()
	end,
	["8 A17 the loader marks the network port installed and keeps the config slot apart from the network slot"] = function()
		local network = networkPort()
		local config = { name = "config port" }
		local g = spec.runtime(nil, nil, config, network)
		spec.eq(network.installed, false, "nothing is installed before the port module loads")
		local port = g.module("internal/network/port")
		spec.eq(network.installed, true, "loading the network port module installs and marks it")
		spec.eq(port.networkPort(), network, "the fourth slot is the network port")
		spec.eq(g.module("internal/configs/port").configPort(), config, "the third slot is the config port")
	end,
}
