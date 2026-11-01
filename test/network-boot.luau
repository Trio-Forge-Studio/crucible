local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")
local world = require("./lib/boot-world")

local function declare(r)
	return {
		Note = r.emit.toClient("Boot.Note", r.n.string),
		Ping = r.emit.toServer("Boot.Ping", r.n.string),
	}
end

-- A server with one Subsystem holding a ClientReady and a Ping receiver; `extra(s, m)` adds to its setup.
local function server(link, record, extra)
	link:boot(function(s)
		local m = declare(s)
		record.m = m
		s.service("Owner", function(self)
			record.state = s.construction.readClaim()
			self.ping = s.emit.receive(self, "ping", m.Ping, function() end)
			self.ready = s.emit.receive(self, "ready", s.ClientReady, function(player)
				table.insert(record.ready, player.Name)
			end)
		end, {
			onShutdown = function()
				table.insert(record.shutdown, "Owner")
			end,
		})
		if extra then
			extra(s, m)
		end
	end)
end

-- After a failed server boot: no remote, listener or timer, no peer for a joiner and no ClientReady.
local function closedServer(link, record)
	spec.eq(link.server.booted, false, "boot failed")
	spec.eq(link.server.remote, nil, "no remote")
	spec.eq(link.server.added, nil, "no PlayerAdded listener")
	spec.eq(link.server.removing, nil, "no PlayerRemoving listener")
	spec.eq(link:pending(), 0, "no timer")
	local joined = link:join("Late" .. #link.clients, nil, { boot = false })
	link:inject(joined, h.bytes("04"), {})
	link:flush()
	spec.eq(#record.ready, 0, "no ClientReady")
	spec.eq(#link:sent(function(e)
		return e.to == joined.name
	end), 0, "nothing reaches the joiner")
end

local function record()
	return { ready = {}, shutdown = {} }
end

-- Rule 79 for a failure after the registry seal: every generation ended, claims and indices discarded, definitions
-- kept, and a second boot gets past its own catalog step to the registry's one-boot rule.
local function rolledBack(realm, rec, again)
	assert(realm.life.hasEnded(rec.state), "the boot generation ended")
	spec.eq(realm.claims.receiverOf(rec.m.Ping), nil, "claims discarded")
	spec.eq(#realm.claims.readyClaims(), 0, "ClientReady claims discarded")
	spec.eq(rec.m.Ping.index, nil, "indices discarded")
	spec.eq(realm.catalog.catalogSize(), 0, "catalog empty")
	spec.eq(realm.catalog.definitionOf(rec.m.Ping), rec.m.Ping, "definitions stay")
	local ok, err = pcall(again)
	spec.eq(ok, false, "a second boot")
	spec.has(err, "boot already ran on this side", "stops only at the registry, after its own catalog step")
end

return {
	["79 a module require throwing closes the session; a player joining gets no peer"] = function()
		local link = Link.new()
		local rec = record()
		link:boot(function()
			error("module threw")
		end)
		spec.has(link.server.bootError, "module threw", "rethrown")
		closedServer(link, rec)
		link:assertClean()
	end,
	["11 79 MAX_MESSAGES + 1 declarations fail the seal"] = function()
		local link = Link.new()
		local rec = record()
		link:boot(function(s)
			for i = 1, 1025 do
				s.emit.toServer(`Many.N{i}`, s.n["nil"])
			end
		end)
		spec.has(link.server.bootError, "too many messages", "rule 11")
		closedServer(link, rec)
		link:assertClean()
	end,
	["79 an unresolved constructor dependency fails the registry seal and closes the session"] = function()
		local link = Link.new()
		local rec = record()
		server(link, rec, function(s)
			s.service("Needy", nil, nil, { deps = { "Missing" } })
		end)
		spec.has(link.server.bootError, "Crucible could not boot", "registry seal")
		closedServer(link, rec)
		link:assertClean()
	end,
	["79 R1 BindToClose throwing on the server rolls the boot back"] = function()
		local link = Link.new()
		local rec = record()
		link.server.bindToCloseError = "bind failed"
		server(link, rec)
		spec.has(link.server.bootError, "boot finalization failed", "finalization")
		spec.has(link.server.bootError, "bind failed", "cause")
		closedServer(link, rec)
		rolledBack(link.server, rec, function()
			link.server.entry.boot({ load = { "crucible-path:ServerScriptService/Game" } })
		end)
		link:assertClean()
	end,
	["79 80 R1 the network commit throwing after BindToClose and booted rolls back, and close does nothing"] = function()
		local link = Link.new()
		local rec = record()
		link:failNext("publish")
		server(link, rec)
		spec.has(link.server.bootError, "injected publish failure", "publication threw")
		closedServer(link, rec)
		spec.eq(#link.server.closers, 1, "BindToClose ran before the failure")
		link.server.closers[1]()
		spec.eq(#rec.shutdown, 0, "a failed boot runs no onShutdown at close")
		rolledBack(link.server, rec, function()
			link.server.entry.boot({ load = { "crucible-path:ServerScriptService/Game" } })
		end)
		link:assertClean()
	end,
	["79 R1 booted throwing on the server rolls the boot back and closes the real session"] = function()
		local link = Link.new()
		local rec = record()
		local s = link.server
		s.setup = function()
			local m = declare(s)
			rec.m = m
			s.service("Owner", function(self)
				rec.state = s.construction.readClaim()
				self.ping = s.emit.receive(self, "ping", m.Ping, function() end)
			end)
		end
		local session = s.graph.module("internal/network/server-session").ServerSession.new(s.port)
		local boot = s.graph.module("internal/boot").boot
		local side = {
			name = "server",
			renders = false,
			booted = function()
				error("booted threw")
			end,
			network = session,
		}
		local ok, err = pcall(boot, side, { load = { "crucible-path:ServerScriptService/Game" } })
		spec.eq(ok, false, "failed")
		spec.has(err, "booted threw", "cause")
		s.booted = false
		closedServer(link, rec)
		rolledBack(s, rec, function()
			boot(side, { load = { "crucible-path:ServerScriptService/Game" } })
		end)
		link:assertClean()
	end,
	["13 79 client discovery timing out or finding a wrong class Protocol or Catalog fails before load"] = function()
		local function loaded(c)
			return c.required ~= nil
		end
		local quiet = Link.new()
		local waiting = quiet:join("Ann", declare)
		quiet:advance(29.9)
		spec.eq(waiting.booted, nil, "still waiting within 30 seconds")
		quiet:advance(0.1)
		spec.eq(waiting.booted, false, "discovery timed out")
		spec.has(waiting.bootError, "Crucible found no ReplicatedStorage.CrucibleNetwork within 30 seconds", "timeout")
		spec.eq(loaded(waiting), false, "no module loaded")
		spec.eq(waiting.attributes.CrucibleBooted, nil, "never booted")
		for _, case in
			{
				{ "className", "Folder", "Crucible found ReplicatedStorage.CrucibleNetwork as a Folder" },
				{ "protocol", 2, "Crucible ReplicatedStorage.CrucibleNetwork has Protocol 2; this client needs 1" },
				{ "catalog", "B\nA", "Crucible ReplicatedStorage.CrucibleNetwork has an invalid Catalog" },
				{ "catalog", nil, "Crucible ReplicatedStorage.CrucibleNetwork has an invalid Catalog" },
			}
		do
			local link = Link.new()
			server(link, record())
			local c = link:join("Ann", declare, {
				tamper = function(found)
					found[case[1]] = case[2]
				end,
			})
			spec.eq(c.booted, false, `{case[1]} fails`)
			spec.has(c.bootError, case[3], "one diagnostic naming the problem")
			spec.eq(loaded(c), false, "before any module loads")
			spec.eq(c.receive, nil, "listener disconnected")
			spec.eq(#link:sent(function(e)
				return e.from == "Ann"
			end), 0, "no READY")
			c.tamper = nil
			link:start(c)
			assert(c.booted, tostring(c.bootError))
			spec.eq(c.m == nil and c.attributes.CrucibleBooted, true, "a second boot adopts and commits")
			link:assertClean()
		end
	end,
	["79 80 R1 booted throwing or the network commit failing leaves the client's CrucibleBooted unset"] = function()
		for _, failure in { "booted", "commit" } do
			local link = Link.new()
			server(link, record())
			local rec = record()
			local c = link:join("Ann", function(realm)
				local m = declare(realm)
				rec.m = m
				realm.service("Owner", function(self)
					rec.state = realm.construction.readClaim()
					self.note = realm.emit.receive(self, "note", m.Note, function() end)
				end)
				realm.behaviour("Lamp", nil, {
					onDestroy = function()
						table.insert(rec.shutdown, tostring(realm.attributes.CrucibleBooted))
					end,
				})
				realm.engine.addTag(realm.engine.instance("lamp", "Model"), "Lamp")
			end, { boot = false })
			if failure == "booted" then
				c.attributes = setmetatable({}, {
					__newindex = function()
						error("SetAttribute threw")
					end,
				})
			else
				link:failNext("fireServer")
			end
			link:start(c)
			spec.eq(c.booted, false, `{failure} failed the boot`)
			spec.eq(rawget(c.attributes, "CrucibleBooted"), nil, "CrucibleBooted unset")
			if failure == "commit" then
				spec.list(rec.shutdown, { "nil" }, "already cleared when the rollback's first onDestroy ran")
			end
			rolledBack(c, rec, function()
				c.entry.boot({ load = { "crucible-path:ServerScriptService/Game" } })
			end)
			spec.eq(#link:sent(function(e)
				return e.from == "Ann"
			end), 0, "a failure before READY sends nothing")
			link:assertClean()
		end
	end,
	["38 76 79 a client boot failing at its network commit settles a pending ask disconnected"] = function()
		local link = Link.new()
		server(link, record(), function(s, m)
			m.Echo = s.emit.askServer("Boot.Echo", s.n.string, s.n.string, nil)
		end)
		local asked
		local c = link:join("Ann", function(realm)
			local m = declare(realm)
			m.Echo = realm.emit.askServer("Boot.Echo", realm.n.string, realm.n.string, nil)
			realm.service("Asker", nil, {
				start = function()
					asked = link:run(function()
						return realm.Network.ask(m.Echo, "x")
					end)
				end,
			})
		end, { boot = false })
		link:failNext("fireServer")
		link:start(c)
		spec.eq(c.booted, false, "READY failing fails the boot")
		spec.eq(asked.done, true, "the ask settled when the session closed")
		spec.eq(asked.value.reason, "disconnected", "as disconnected, not timeout")
		spec.eq(link:pending(), 0, "its deadline was cancelled")
		link:assertClean()
	end,
	["79 the client flush throwing after READY closes with FAIL closing"] = function()
		local link = Link.new()
		local rec = record()
		server(link, rec)
		local c = link:join("Ann", function(realm)
			local m = declare(realm)
			realm.service("Sender", nil, {
				start = function()
					realm.Network.send(m.Ping, "queued")
				end,
			})
		end, { boot = false })
		link:failNext("fireServer", 1)
		link:start(c)
		spec.eq(c.booted, false, "the flush failed the boot")
		spec.eq(c.attributes.CrucibleBooted, nil, "unset")
		local sent = {}
		for _, e in
			link:sent(function(entry)
				return entry.from == "Ann"
			end)
		do
			table.insert(sent, h.hex(e.packet))
		end
		spec.list(sent, { "04", "05 02" }, "READY, then FAIL closing from close")
		link:flush()
		spec.list(rec.ready, { "Ann" }, "the server saw READY")
		spec.eq(link.kicks[1].message, "Connection problem, please rejoin.", "and kicks on the FAIL")
		link:assertClean()
	end,
	["17 sends throw before boot loaded its modules and fail softly after a failed boot"] = function()
		local link = Link.new()
		local N = link.server.graph.module("server").Network
		local m = declare(link.server)
		local ok, err = pcall(N.send, nil, m.Note, "x")
		spec.eq(ok, false, "before any boot")
		spec.has(err, "Crucible networking is not ready", "throws")
		link:boot(function(s)
			s.Network.send(nil, m.Note, "during load")
		end)
		spec.has(link.server.bootError, "Crucible networking is not ready", "a send during load throws")
		local ann = link:join("Ann", nil, { boot = false })
		spec.eq(N.send(ann.player, m.Note, "after"), false, "a failed boot's session sends nothing")
		local other = Link.new()
		local c = other:join("Bea")
		other:advance(30)
		spec.eq(c.booted, false, "client boot failed")
		c.m = declare(c)
		c.m.Echo = c.emit.askServer("Boot.Echo", c.n.string, c.n.string, nil)
		local asked = other:run(function()
			return c.Network.ask(c.m.Echo, "x")
		end)
		spec.eq(asked.value.reason, "disconnected", "ask after a failed boot")
		spec.eq(c.Network.send(c.m.Ping, "x"), false, "send after a failed boot")
		link:assertClean()
		other:assertClean()
	end,
	["12 the Roblox port sets Protocol and Catalog before parenting and discovery reads them"] = function()
		local events = {}
		local storage = { Name = "ReplicatedStorage" }
		local remote
		local graph = spec.runtime({
			Instance = {
				new = function(className)
					remote = {
						ClassName = className,
						attributes = {},
						OnServerEvent = {
							Connect = function()
								table.insert(events, "connect server")
								return {
									Disconnect = function()
										table.insert(events, "disconnect")
									end,
								}
							end,
						},
					}
					function remote:SetAttribute(key, value)
						table.insert(events, `attribute {key} {value}`)
						self.attributes[key] = value
					end
					function remote:Destroy()
						table.insert(events, "destroy")
					end
					return setmetatable(remote, {
						__newindex = function(self, key, value)
							table.insert(events, `{key} {value == storage and "storage" or tostring(value)}`)
							rawset(self, key, value)
						end,
					})
				end,
			},
			game = {
				GetService = function(_, name)
					if name == "ReplicatedStorage" then
						return storage
					end
					return {}
				end,
			},
		})
		local port = graph.module("internal/network/roblox-network-port").robloxNetworkPort()
		port.create(function() end)
		port.publish(1, "A.B\nC.D")
		spec.list(events, {
			"Name CrucibleNetwork",
			"connect server",
			"attribute Protocol 1",
			"attribute Catalog A.B\nC.D",
			"Parent storage",
		}, "attributes before the parent, as the last act")
		port.destroy()
		spec.eq(events[#events], "destroy", "destroyed")
		function storage:WaitForChild(name, timeout)
			table.insert(events, `wait {name} {timeout}`)
			return {
				ClassName = "RemoteEvent",
				IsA = function(_, class)
					return class == "RemoteEvent"
				end,
				GetAttribute = function(_, key)
					return ({ Protocol = 1, Catalog = "A.B" })[key]
				end,
				OnClientEvent = {
					Connect = function()
						table.insert(events, "connect client")
						return { Disconnect = function() end }
					end,
				},
			}
		end
		local found = port.discover(30, function() end)
		spec.eq(found.className, "RemoteEvent", "class")
		spec.eq(found.protocol, 1, "Protocol")
		spec.eq(found.catalog, "A.B", "Catalog")
		spec.list(
			{ events[#events - 1], events[#events] },
			{ "wait CrucibleNetwork 30", "connect client" },
			"connects before returning"
		)
	end,
	["79 world finalization undoes booted before rollback and leaves the network uncommitted"] = function()
		local w = world()
		local order = {}
		w.service("Store")
		w.define("Lamp", {
			onDestroy = function()
				table.insert(order, `onDestroy {tostring(w.flag)}`)
			end,
		})
		w.tagged("L1", "Lamp")
		local ok, err = pcall(w.boot, nil, true, {
			booted = function()
				w.flag = true
				table.insert(order, "booted")
				return function()
					w.flag = nil
					table.insert(order, "undo")
				end
			end,
			commit = function()
				table.insert(order, "commit")
				error("commit threw")
			end,
		})
		spec.eq(ok, false, "rejected")
		spec.has(err, "commit threw", "cause")
		spec.list(order, { "booted", "commit", "undo", "onDestroy nil" }, "undo first, then the rollback")
		local w2 = world()
		w2.bindToCloseError = "bind threw"
		local commits = 0
		local ok2 = pcall(w2.boot, nil, false, {
			commit = function()
				commits += 1
			end,
		})
		spec.eq(ok2, false, "BindToClose failure rejects")
		spec.eq(commits, 0, "the commit never ran")
		spec.eq(#w2.events, 0, "nor did booted")
	end,
}
