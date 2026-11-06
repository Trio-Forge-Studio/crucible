local HttpService = game:GetService("HttpService")
local ReplicatedStorage = game:GetService("ReplicatedStorage")

local SLOT = 1
local PREFIX = "bench:"

local LibMP = require(ReplicatedStorage:WaitForChild("LibMP")) :: any

local session = LibMP.Session.OpenFromSlot(SLOT)
assert(session and session:IsValid(), "slot 1 is empty: run capture.luau first")
local tickToMs = session:FetchGlobalDesc().TickToMsCpu
local frameMin, frameMax = session:GetFrameIdMin(), session:GetFrameIdMax()
assert(frameMax > frameMin, "the capture holds no frames")

local names: { [number]: string | false } = {}
local function benchName(timerId: number): string | false
	local name = names[timerId]
	if name == nil then
		local timerName = session:FetchTimerDesc(timerId).TimerName
		name = if string.sub(timerName, 1, #PREFIX) == PREFIX then string.sub(timerName, #PREFIX + 1) else false
		names[timerId] = name
	end
	return name :: string | false
end

-- The first frame only warms up the scope stacks.
local firstFrame = frameMin + 1
local durations: { [string]: { number } } = {}
local threads: { [string]: string } = {}
local iter = session:CreateLogIterator()
iter:Configure({ SkipGpuThreads = true, SkipEvents = true })
local state = iter:GetState()
iter:RewindTo(frameMin, frameMax)
-- Roblox stops a script that runs about 10 seconds without yielding. The slot doesn't change between steps.
local resumed = os.clock()
while iter:Step() do
	if os.clock() - resumed > 0.05 then
		task.wait()
		resumed = os.clock()
	end
	if
		state:IsExit()
		and state:FrameId() >= firstFrame
		and not state:ThreadStackIsUnderflowed()
		and not state:ThreadStackWasOverflowed()
	then
		local name = benchName(state:TimerId())
		if name then
			local element = iter:GetCurrentThreadStackElement(state:ThreadStackDepth())
			local list = durations[name]
			if not list then
				list = {}
				durations[name] = list
				threads[name] = session:FetchThreadDesc(state:ThreadId()).ThreadName
			end
			table.insert(list, (state:Timestamp() - element:EnterTimestamp()) * tickToMs * 1000)
		end
	end
end
iter:Dispose()

local workloads = {}
for name, list in durations do
	table.sort(list)
	local total = 0
	for _, value in list do
		total += value
	end
	local function at(fraction: number): number
		return list[math.clamp(math.ceil(#list * fraction), 1, #list)]
	end
	workloads[name] = {
		thread = threads[name],
		micros = { n = #list, median = at(0.5), p10 = at(0.1), p90 = at(0.9), mean = total / #list },
	}
end

local result = {
	libmp = `{bit32.rshift(LibMP.Versions.Library, 16)}.{bit32.band(LibMP.Versions.Library, 0xFFFF)}`,
	frames = frameMax - firstFrame + 1,
	workloads = workloads,
}
session:Dispose()
return HttpService:JSONEncode(result)
