-- The default fake network port for realms that do not exercise networking. It publishes an empty catalog, so a client
-- realm's discovery returns at once, and records what the session asked of it so specs can assert a closed session.
-- `failNext` makes the next call of that name throw once. Network specs install `network-link.luau` instead.
return function()
	local port = {
		installed = false,
		created = false,
		destroyed = false,
		listening = false,
		published = nil,
		fired = {},
		kicked = {},
		listeners = {},
		timers = {},
		failNext = nil,
	}
	local function failing(name)
		if port.failNext == name then
			port.failNext = nil
			error(`injected {name} failure`)
		end
	end
	local function listen(name, listener)
		port.listeners[name] = listener
		return function()
			port.listeners[name] = nil
		end
	end
	function port.create(receive)
		failing("create")
		port.created = true
		port.listening = true
		port.receive = receive
	end
	function port.publish(protocol, catalog)
		failing("publish")
		port.published = { protocol = protocol, catalog = catalog }
	end
	function port.fireClient(player, packet, sidecar)
		table.insert(port.fired, { player = player, packet = packet, sidecar = sidecar })
	end
	function port.players()
		return {}
	end
	function port.onPlayerAdded(listener)
		return listen("added", listener)
	end
	function port.onPlayerRemoving(listener)
		return listen("removing", listener)
	end
	function port.kick(player, message)
		table.insert(port.kicked, { player = player, message = message })
	end
	function port.destroy()
		port.destroyed = true
		port.listening = false
	end
	function port.discover(_timeout, receive)
		failing("discover")
		port.listening = true
		port.receive = receive
		return { className = "RemoteEvent", protocol = 1, catalog = "" }
	end
	function port.disconnect()
		port.listening = false
	end
	function port.fireServer(packet, sidecar)
		failing("fireServer")
		table.insert(port.fired, { packet = packet, sidecar = sidecar })
	end
	function port.delay(_seconds, _callback)
		local timer = {}
		port.timers[timer] = true
		return function()
			port.timers[timer] = nil
		end
	end
	function port.resume(thread, ...)
		assert(coroutine.resume(thread, ...))
	end
	return port
end
