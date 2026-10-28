-- A fake engine behind the runtime's EnginePort. State changes apply at once; their signals queue like Roblox's
-- Deferred signal behaviour, which FD's places set, and run with task.defer callbacks when the test calls flush.
local task = require("@lune/task")

local FrameEvent =
	{ PreRender = 0, PreSimulation = 1, PostSimulation = 2, RenderStepped = 3, PreAnimation = 4, Heartbeat = 5 }

local engine = {}
engine.FrameEvent = FrameEvent

local function listeners()
	return { list = {} }
end

local function connect(group, fn)
	local entry = { fn = fn, connected = true }
	table.insert(group.list, entry)
	return function()
		entry.connected = false
		local index = table.find(group.list, entry)
		if index then
			table.remove(group.list, index)
		end
	end
end

local function sortedTags(instance): { string }
	local tags = {}
	for tag in instance.tags do
		table.insert(tags, tag)
	end
	table.sort(tags)
	return tags
end

function engine.new()
	local e = {
		deferred = {},
		logs = {},
		instances = {},
		tagAdded = {},
		tagRemoved = {},
		frames = {
			[0] = listeners(),
			[1] = listeners(),
			[2] = listeners(),
			[3] = listeners(),
			[4] = listeners(),
			[5] = listeners(),
		},
		renderSteps = {},
		bound = 0,
	}

	local function group(map, key)
		map[key] = map[key] or listeners()
		return map[key]
	end

	local function fire(signal, ...)
		local args = table.pack(...)
		for _, entry in table.clone(signal.list) do
			table.insert(e.deferred, function()
				if entry.connected then
					entry.fn(table.unpack(args, 1, args.n))
				end
			end)
		end
	end

	function e.instance(name: string, className: string?, attributes: { [string]: any }?)
		local instance = {
			Name = name,
			ClassName = className or "Model",
			inWorld = true,
			tags = {},
			attributes = attributes or {},
			ancestry = listeners(),
			attributeChanged = {},
		}
		table.insert(e.instances, instance)
		return instance
	end

	-- CollectionService signals only for instances in the DataModel.
	function e.addTag(instance, tag: string)
		if instance.tags[tag] then
			return
		end
		instance.tags[tag] = true
		if instance.inWorld then
			fire(group(e.tagAdded, tag), instance)
		end
	end

	function e.removeTag(instance, tag: string)
		if not instance.tags[tag] then
			return
		end
		instance.tags[tag] = nil
		if instance.inWorld then
			fire(group(e.tagRemoved, tag), instance)
		end
	end

	function e.setAttribute(instance, name: string, value: any)
		instance.attributes[name] = value
		fire(group(instance.attributeChanged, name))
	end

	-- Leaving the DataModel fires each tag's removed signal and returning fires its added signal, as
	-- CollectionService does. C4's Studio probe S1 confirms the removal signal on reparent to nil.
	function e.setInWorld(instance, inWorld: boolean)
		local changed = instance.inWorld ~= inWorld
		instance.inWorld = inWorld
		fire(instance.ancestry)
		if changed then
			for _, tag in sortedTags(instance) do
				fire(group(if inWorld then e.tagAdded else e.tagRemoved, tag), instance)
			end
		end
	end

	-- Runs queued signals and task.defer callbacks, including ones they queue, in order.
	function e.flush()
		while #e.deferred > 0 do
			table.remove(e.deferred, 1)()
		end
	end

	local function frameSignal(event, dt, reverse)
		local entries = table.clone(e.frames[event].list)
		for i = 1, #entries do
			local entry = entries[if reverse then #entries - i + 1 else i]
			if entry.connected then
				entry.fn(dt)
			end
		end
	end

	function e.frame(event: number, dt: number)
		if event ~= FrameEvent.PreRender then
			frameSignal(event, dt, false)
			return
		end
		local steps = table.clone(e.renderSteps)
		table.sort(steps, function(a, b)
			if a.priority ~= b.priority then
				return a.priority < b.priority
			end
			return a.order < b.order
		end)
		for _, step in steps do
			if step.connected then
				step.fn(dt)
			end
		end
		frameSignal(FrameEvent.RenderStepped, dt, false)
		frameSignal(FrameEvent.PreRender, dt, true)
	end

	function e.cycle(dt: number, renders: boolean)
		if renders then
			e.frame(FrameEvent.PreRender, dt)
		end
		for _, event in
			{ FrameEvent.PreAnimation, FrameEvent.PreSimulation, FrameEvent.PostSimulation, FrameEvent.Heartbeat }
		do
			e.frame(event, dt)
		end
	end

	function e.logged(part: string): number
		local n = 0
		for _, entry in e.logs do
			if string.find(entry.message, part, 1, true) then
				n += 1
			end
		end
		return n
	end

	e.port = {
		hasTag = function(instance, tag)
			return instance.tags[tag] == true
		end,
		tagged = function(tag)
			local found = {}
			for _, instance in e.instances do
				if instance.inWorld and instance.tags[tag] then
					table.insert(found, instance)
				end
			end
			return found
		end,
		onTagAdded = function(tag, fn)
			return connect(group(e.tagAdded, tag), fn)
		end,
		onTagRemoved = function(tag, fn)
			return connect(group(e.tagRemoved, tag), fn)
		end,
		inWorld = function(instance)
			return instance.inWorld
		end,
		isA = function(instance, className)
			return className == "Instance" or instance.ClassName == className
		end,
		onAncestryChanged = function(instance, fn)
			return connect(instance.ancestry, fn)
		end,
		getAttribute = function(instance, name)
			return instance.attributes[name]
		end,
		onAttributeChanged = function(instance, name, fn)
			return connect(group(instance.attributeChanged, name), fn)
		end,
		onFrame = function(event, fn)
			return connect(e.frames[event], fn)
		end,
		bindRenderStep = function(name, priority, fn)
			e.bound += 1
			local step = { name = name, priority = priority, fn = fn, order = e.bound, connected = true }
			table.insert(e.renderSteps, step)
			return function()
				for index = #e.renderSteps, 1, -1 do
					local bound = e.renderSteps[index]
					if bound.name == name then
						bound.connected = false
						table.remove(e.renderSteps, index)
					end
				end
			end
		end,
		defer = function(callback)
			table.insert(e.deferred, callback)
		end,
		cancel = function(thread)
			task.cancel(thread)
		end,
		log = function(level, message)
			table.insert(e.logs, { level = level, message = message })
		end,
		describe = function(instance)
			return instance.Name
		end,
	}

	return e
end

return engine
