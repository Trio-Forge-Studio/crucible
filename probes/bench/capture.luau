local ReplicatedStorage = game:GetService("ReplicatedStorage")

local SLOT = 1
local FRAME_LIMIT = 256
local MEMORY_MB = 256

local LibMP = require(ReplicatedStorage:WaitForChild("LibMP")) :: any

assert(LibMP.Control:IsBackendReady(), "MicroProfiler backend isn't ready")
LibMP.Control:EnableProfiler(true)
LibMP.Control:SetMemTargetMB(MEMORY_MB)
LibMP.Control:SetFrameLimit(FRAME_LIMIT)
LibMP.Control:EnableCapture(true)
-- A paused profiler keeps its old frames, so the window refills before the snapshot.
task.wait(FRAME_LIMIT / 30)
assert(LibMP.Control:CaptureToSlotSync(SLOT), "capture to slot failed")
LibMP.Control:EnableCapture(false)
LibMP.Control:EnableProfiler(false)

local session = LibMP.Session.OpenFromSlot(SLOT)
assert(session and session:IsValid(), "couldn't open the captured slot")
local frames = session:GetFrameIdMax() - session:GetFrameIdMin() + 1
session:Dispose()
return `captured {frames} frames into slot {SLOT}`
