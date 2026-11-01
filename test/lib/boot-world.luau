local world = require("./binding-world")

return function()
	local w = world()
	local function none() end
	local boot = w.graph.module("internal/boot")
	-- Specs written before networking hand ignite a Side without a network; they get an idle one.
	w.bootModule = setmetatable({
		ignite = function(port, side, options)
			if side.network == nil then
				side = table.clone(side)
				side.network = { open = none, loaded = none, commit = none, close = none }
			end
			return boot.ignite(port, side, options)
		end,
	}, { __index = boot })
	w.subsystem = w.graph.module("subsystem").Subsystem
	w.closers = {}
	w.workers = {}
	w.now = 0
	w.engine.port.clock = function()
		return w.now
	end
	w.engine.port.wait = function()
		w.now += 0.5
		for _, thread in table.clone(w.workers) do
			if coroutine.status(thread) == "suspended" then
				assert(coroutine.resume(thread))
			end
		end
	end
	w.engine.port.spawn = function(callback)
		local thread = coroutine.create(callback)
		table.insert(w.workers, thread)
		assert(coroutine.resume(thread))
	end
	w.engine.port.bindToClose = function(callback)
		if w.bindToCloseError then
			error(w.bindToCloseError)
		end
		table.insert(w.closers, callback)
	end
	function w.service(id, methods, options)
		options = options or {}
		local class = setmetatable(methods or {}, { __index = w.subsystem })
		class.__index = class
		function class.new(...)
			local self = setmetatable({}, class)
			w.subsystem.constructor(self)
			if options.init then
				options.init(self, ...)
			end
			return self
		end
		w.emit.defineClass(
			class,
			{ kind = "subsystem", id = id, deps = options.deps or {}, provides = options.provides or {} }
		)
		return class, w.registry.recordOf(class)
	end
	function w.boot(options, client, overrides)
		overrides = overrides or {}
		w.bootModule.ignite(w.engine.port, {
			name = if client then "client" else "server",
			renders = client == true,
			booted = overrides.booted or function()
				table.insert(w.events, "booted")
			end,
			network = { open = none, loaded = none, commit = overrides.commit or none, close = none },
		}, options or {})
	end
	function w.empty()
		for _, record in w.registry.records() do
			if record.state then
				assert(w.life.hasEnded(record.state), record.id .. " still alive")
			end
			if record.bound then
				assert(next(record.bound) == nil, record.id .. " still bound")
			end
		end
		for _, signal in w.engine.frames do
			assert(#signal.list == 0, "frame still connected")
		end
		assert(#w.engine.renderSteps == 0, "render still bound")
		for _, signal in w.engine.tagAdded do
			assert(#signal.list == 0, "tag added still connected")
		end
		for _, signal in w.engine.tagRemoved do
			assert(#signal.list == 0, "tag removed still connected")
		end
		for _, instance in w.engine.instances do
			assert(#instance.ancestry.list == 0, "ancestry still connected")
		end
	end
	return w
end
