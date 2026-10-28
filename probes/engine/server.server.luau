local RunService = game:GetService("RunService")
local CollectionService = game:GetService("CollectionService")
local ServerStorage = game:GetService("ServerStorage")
local frame = 0
local sequence = 0
local origin = os.clock()
local function log(question, message)
	sequence += 1
	print(string.format("S1|server|%04d|%.6f|f%d|%s|%s", sequence, os.clock() - origin, frame, question, message))
end
RunService.Heartbeat:Connect(function()
	frame += 1
end)
log("metadata", "version=" .. version() .. " behavior=" .. tostring(workspace:GetAttribute("ProbeSignalBehavior")))

local phases = {}
for _, name in { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" } do
	RunService[name]:Connect(function()
		table.insert(phases, name)
		if name == "Heartbeat" then
			if frame <= 35 then
				log("3.server", table.concat(phases, ","))
			end
			table.clear(phases)
		end
	end)
end
local function settle()
	RunService.Heartbeat:Wait()
	RunService.Heartbeat:Wait()
end
local function tags(id, operation)
	local tag = "S1_" .. id
	local part = Instance.new("Part")
	part.Anchored = true
	local added = CollectionService:GetInstanceAddedSignal(tag):Connect(function(item)
		log(id, "added " .. item.Name .. " parent=" .. tostring(item.Parent))
	end)
	local removed = CollectionService:GetInstanceRemovedSignal(tag):Connect(function(item)
		log(id, "removed " .. item.Name .. " parent=" .. tostring(item.Parent))
	end)
	operation(part, tag)
	settle()
	added:Disconnect()
	removed:Disconnect()
	part:Destroy()
end
settle()
tags("1a", function(part, tag)
	part.Parent = workspace
	CollectionService:AddTag(part, tag)
	settle()
	log("1a", "before Parent=nil")
	part.Parent = nil
	log("1a", "after Parent=nil HasTag=" .. tostring(CollectionService:HasTag(part, tag)))
	settle()
	log("1a", "settled HasTag=" .. tostring(CollectionService:HasTag(part, tag)))
	log("1a", "before Parent=workspace")
	part.Parent = workspace
	log("1a", "after Parent=workspace")
end)
tags("1b", function(part, tag)
	part.Parent = workspace
	CollectionService:AddTag(part, tag)
	settle()
	log("1b", "before ServerStorage")
	part.Parent = ServerStorage
	log("1b", "after ServerStorage")
	settle()
	log("1b", "before workspace")
	part.Parent = workspace
	log("1b", "after workspace")
end)
tags("1c", function(part, tag)
	part.Parent = workspace
	CollectionService:AddTag(part, tag)
	settle()
	log("1c", "before Destroy")
	part:Destroy()
	log("1c", "after Destroy HasTag=" .. tostring(CollectionService:HasTag(part, tag)))
end)
tags("1d", function(part, tag)
	part.Parent = workspace
	log("1d", "before AddTag")
	CollectionService:AddTag(part, tag)
	log("1d", "after AddTag before RemoveTag")
	CollectionService:RemoveTag(part, tag)
	log("1d", "after RemoveTag")
end)
tags("1e", function(part, tag)
	part.Name = "CreatedAtNil"
	CollectionService:AddTag(part, tag)
	log("1e", "tagged at nil")
	settle()
	local clone = part:Clone()
	clone.Name = "ClonedAtNil"
	log("1e", "clone HasTag=" .. tostring(CollectionService:HasTag(clone, tag)))
	settle()
	log("1e", "before parent created")
	part.Parent = workspace
	log("1e", "after parent created before parent clone")
	clone.Parent = workspace
	log("1e", "after parent clone")
	settle()
	clone:Destroy()
end)

for _, kind in { "wait", "wait0", "event", "defer-before", "defer-after", "spawn-callback" } do
	local id = "2." .. kind
	local event = Instance.new("BindableEvent")
	local thread
	local function check()
		local status = coroutine.status(thread)
		log(id, "check status=" .. status)
		if status == "suspended" then
			local ok, result = pcall(task.cancel, thread)
			log(id, "cancel ok=" .. tostring(ok) .. " result=" .. tostring(result))
		end
	end
	thread = coroutine.create(function()
		log(id, "running schedules check")
		if kind == "defer-before" then
			task.defer(thread)
		elseif kind == "spawn-callback" then
			task.defer(function()
				log(id, "other callback before task.spawn(T)")
				task.spawn(thread)
				log(id, "other callback after task.spawn(T)")
			end)
		end
		task.defer(check)
		if kind == "defer-after" then
			task.defer(thread)
		elseif kind == "event" then
			task.defer(function()
				log(id, "fire event after yield")
				event:Fire()
			end)
		end
		log(id, "before yield")
		if kind == "wait" then
			task.wait()
		elseif kind == "wait0" then
			task.wait(0)
		elseif kind == "event" then
			event.Event:Wait()
		else
			coroutine.yield()
		end
		log(id, "RESUMED segment after first yield")
		task.wait()
		log(id, "RESUMED segment after second yield")
	end)
	task.spawn(thread)
	settle()
	log(id, "final status=" .. coroutine.status(thread))
	event:Destroy()
end
for _, method in { "cancel", "close" } do
	for _, status in { "running", "normal" } do
		local id = "2.errors." .. method .. "." .. status
		local target
		target = coroutine.create(function()
			local function attempt()
				log(id, "target status=" .. coroutine.status(target))
				local result = table.pack(pcall(if method == "cancel" then task.cancel else coroutine.close, target))
				for index = 1, result.n do
					log(id, "result[" .. index .. "]=" .. tostring(result[index]))
				end
			end
			if status == "normal" then
				local child = coroutine.create(attempt)
				log(id, "child resume=" .. tostring(coroutine.resume(child)))
			else
				attempt()
			end
		end)
		task.spawn(target)
		settle()
	end
end

local part = Instance.new("Part")
part.Anchored = true
part.Parent = workspace
local tag = "S1_signal_order"
local attributeConnection = part:GetAttributeChangedSignal("Value"):Connect(function()
	log("4", "attribute handler start")
	task.defer(function()
		log("4", "defer inside attribute handler")
	end)
	log("4", "attribute handler end")
end)
local tagConnection = CollectionService:GetInstanceAddedSignal(tag):Connect(function()
	log("4", "tag added handler")
end)
log("4", "before attribute write")
part:SetAttribute("Value", 1)
log("4", "after attribute write before AddTag")
CollectionService:AddTag(part, tag)
log("4", "after AddTag")
task.defer(function()
	log("4", "outer defer")
end)
log("4", "end firing resumption")
settle()
attributeConnection:Disconnect()
tagConnection:Disconnect()
part:Destroy()

local long = workspace:GetAttribute("LongClose") == true
for index, duration in (if long then { 1, 3, 40 } else { 1, 3, 5 }) do
	game:BindToClose(function()
		log("5", string.format("callback %d start duration=%d clock=%.6f", index, duration, os.clock()))
		if duration == 40 then
			task.spawn(function()
				for tick = 1, 45 do
					task.wait(1)
					log("5.watch", string.format("tick=%d clock=%.6f", tick, os.clock()))
				end
			end)
		end
		task.wait(duration)
		log("5", string.format("callback %d end duration=%d clock=%.6f", index, duration, os.clock()))
	end)
end
log("ready", "stop play now long=" .. tostring(long))
