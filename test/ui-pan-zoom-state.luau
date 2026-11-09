local roblox: any = require("@lune/roblox")
local Vector2 = roblox.Vector2
local M = require("./lib/ui-load")({ globals = { Vector2 = Vector2 } })("pan-zoom-state")
local function near(actual, expected, label: string)
	assert((actual - expected).Magnitude < 1e-3, `{label}: expected {expected}, got {actual}`)
end
return {
	["contain fit scales the content to fit its tighter axis"] = function()
		assert(M.containFit(Vector2.new(800, 600), Vector2.new(1600, 1000)) == 0.5, "width limits a wide map")
		assert(M.containFit(Vector2.new(800, 600), Vector2.new(1000, 2000)) == 0.3, "height limits a tall map")
	end,
	["cover clamp stops a larger axis at the viewport edge and centres a smaller one"] = function()
		local content, half = Vector2.new(2000, 400), Vector2.new(400, 300)
		local pan = M.coverPan(Vector2.new(900, 50), content, 1, half)
		assert((pan.X - content.X / 2) * 1 == -half.X, "the left content edge meets the left viewport edge")
		assert(pan.Y == 0, "content shorter than the viewport stays centred")
		local back = M.coverPan(Vector2.new(-700, -10), content, 1, half)
		assert((back.X + content.X / 2) == half.X, "the right content edge meets the right viewport edge")
		near(M.coverPan(Vector2.new(100, 0), content, 1, half), Vector2.new(100, 0), "inside the limit is kept")
		local zoomed = M.coverPan(Vector2.new(5000, 5000), content, 2, half)
		near(zoomed, Vector2.new(800, 50), "at twice the scale the content covers more and pans further")
	end,
	["zooming around an anchor keeps the content point under it"] = function()
		local view = { pan = Vector2.new(30, -20), zoom = 1 }
		local fit, anchor = 0.5, Vector2.new(120, -60)
		local point = anchor / (fit * view.zoom) - view.pan
		local zoomed = M.zoomAround(view, 1.5, anchor, fit)
		assert(zoomed.zoom == 1.5, "takes the requested zoom")
		near((point + zoomed.pan) * fit * zoomed.zoom, anchor, "the anchored point stays under the anchor")
		near(M.zoomAround(view, 1.5, Vector2.zero, fit).pan, view.pan, "a centre anchor keeps the pan")
	end,
	["reveal moves a point back inside the safe area and leaves a safe point alone"] = function()
		local half = Vector2.new(400, 300)
		local inset = { topLeft = Vector2.new(100, 100), bottomRight = Vector2.new(100, 50) }
		assert(M.revealPan(Vector2.zero, Vector2.new(100, 100), 0.5, half, inset) == nil, "a safe point stays")
		local right = M.revealPan(Vector2.zero, Vector2.new(350, 0), 0.5, half, inset)
		near((right + Vector2.new(700, 0)) * 0.5, Vector2.new(300, 0), "the point lands on the right safe edge")
		local corner = M.revealPan(Vector2.new(10, 10), Vector2.new(-500, 400), 0.5, half, inset)
		near(corner, Vector2.new(410, -290), "a far corner point lands on both safe edges")
		local tight = { topLeft = Vector2.new(140, 140), bottomRight = Vector2.new(140, 140) }
		local squeezed = M.revealPan(Vector2.zero, Vector2.new(50, -20), 1, Vector2.new(100, 100), tight)
		near(squeezed, Vector2.new(-50, 20), "a viewport smaller than its insets centres the point")
	end,
	["glide eases exponentially at rate 9 and snaps within half a unit"] = function()
		local step = M.glideStep(Vector2.zero, Vector2.new(100, 0), 0.1)
		near(step, Vector2.new(100 * (1 - math.exp(-0.9)), 0), "one step covers 1 - e^(-9 dt) of the way")
		local target = Vector2.new(100, 0)
		assert(M.glideStep(Vector2.new(99.6, 0), target, 0.001) == target, "within half a unit it snaps")
		local pan = Vector2.zero
		for _ = 1, 120 do
			pan = M.glideStep(pan, target, 1 / 60)
		end
		assert(pan == target, "two seconds reach the target exactly")
	end,
	["stick input ignores the deadzone and rescales the rest"] = function()
		assert(M.stickVector(Vector2.new(0.1, 0.1), 0.2) == Vector2.zero, "a resting stick moves nothing")
		near(M.stickVector(Vector2.new(1, 0), 0.2), Vector2.new(1, 0), "full tilt is full speed")
		near(M.stickVector(Vector2.new(0.6, 0), 0.2), Vector2.new(0.5, 0), "the range past the deadzone rescales")
		near(M.stickVector(Vector2.new(0, -1.2), 0.2), Vector2.new(0, -1), "overshoot caps at full speed")
	end,
}
