return function(realm)
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
		failing("fireClient")
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
		return { className = "RemoteEvent", protocol = 1, catalog = "Crucible.State.Fault\nCrucible.State.Frame" }
	end
	function port.disconnect()
		port.listening = false
	end
	function port.fireServer(packet, sidecar)
		failing("fireServer")
		table.insert(port.fired, { packet = packet, sidecar = sidecar })
	end
	function port.delay(seconds, callback)
		local timer = {}
		port.timers[timer] = true
		local thread = if realm
			then realm.delay(seconds, function()
				port.timers[timer] = nil
				callback()
			end)
			else nil
		return function()
			port.timers[timer] = nil
			if thread then
				realm.cancel(thread)
			end
		end
	end
	function port.now()
		return if realm then realm.clock() else 0
	end
	function port.resume(thread, ...)
		assert(coroutine.resume(thread, ...))
	end
	return port
end
