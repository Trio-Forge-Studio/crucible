local roblox: any = require("@lune/roblox")
local Vector2 = roblox.Vector2
local M = require("./lib/ui-load")({ globals = { Vector2 = Vector2 } })("toast-layout")
local function near(actual, expected, label: string)
	assert((actual - expected).Magnitude < 1e-6, `{label}: expected {expected}, got {actual}`)
end
local function options(name: string, layout: string, expanded: boolean)
	return { layout = layout, expanded = expanded, gap = 10, peek = 12, placement = M.placementOf(name) }
end
local cards = { Vector2.new(300, 40), Vector2.new(300, 80), Vector2.new(300, 60) }
return {
	["each placement anchors to its edge, stacks away from it and swipes toward it"] = function()
		local expected = {
			TopLeft = { Vector2.new(0, 0), 1, Vector2.new(-1, 0) },
			TopCenter = { Vector2.new(0.5, 0), 1, Vector2.new(0, -1) },
			TopRight = { Vector2.new(1, 0), 1, Vector2.new(1, 0) },
			BottomLeft = { Vector2.new(0, 1), -1, Vector2.new(-1, 0) },
			BottomCenter = { Vector2.new(0.5, 1), -1, Vector2.new(0, 1) },
			BottomRight = { Vector2.new(1, 1), -1, Vector2.new(1, 0) },
		}
		for name, values in expected do
			local placement = M.placementOf(name)
			assert(placement.anchor == values[1], `{name} anchors at {values[1]}`)
			assert(placement.stack == values[2], `{name} stacks {values[2]}`)
			assert(placement.swipe == values[3], `{name} swipes toward {values[3]}`)
		end
	end,
	["a list stacks toasts of mixed heights with the gap between them"] = function()
		local top = M.stackTargets(cards, options("TopRight", "List", true))
		near(top[1].position, Vector2.new(0, 0), "the newest toast sits at the edge")
		near(top[2].position, Vector2.new(0, 50), "the next one starts after 40 pixels and the gap")
		near(top[3].position, Vector2.new(0, 140), "the third starts after 40 + 10 + 80 + 10")
		local bottom = M.stackTargets(cards, options("BottomLeft", "List", false))
		near(bottom[3].position, Vector2.new(0, -140), "bottom placements stack upward")
		for depth, target in top do
			assert(
				target.scale == 1 and target.depth == depth - 1,
				"list toasts are full size, numbered from the front"
			)
			assert(target.collapsedHeight == nil, "list toasts draw at their own height")
		end
		local open = M.stackTargets(cards, options("TopRight", "Deck", true))
		near(open[3].position, top[3].position, "an expanded deck is a list")
	end,
	["a collapsed deck peeks each card past the front and centres the shrunk cards"] = function()
		local right = M.stackTargets(cards, options("TopRight", "Deck", false))
		near(right[1].position, Vector2.new(0, 0), "the front card sits at the edge")
		assert(right[1].scale == 1 and right[1].collapsedHeight == nil, "the front card is full size")
		assert(math.abs(right[2].scale - 0.95) < 1e-9 and math.abs(right[3].scale - 0.9) < 1e-9, "cards shrink")
		assert(
			right[2].collapsedHeight == 40 and right[3].collapsedHeight == 40,
			"cards behind draw at the front height"
		)
		local farEdge = right[2].position.Y + 40 * right[2].scale
		assert(math.abs(farEdge - 52) < 1e-9, "the second card's far edge peeks 12 pixels past the front")
		assert(math.abs(right[3].position.Y + 40 * right[3].scale - 64) < 1e-9, "the third peeks 24 pixels")
		assert(right[2].position.X < 0, "a shrunk right-anchored card shifts left")
		assert(math.abs(right[2].position.X + 7.5) < 1e-9, "by half its shrink")
		local left = M.stackTargets(cards, options("TopLeft", "Deck", false))
		assert(math.abs(left[2].position.X - 7.5) < 1e-9, "a shrunk left-anchored card shifts right")
		local centre = M.stackTargets(cards, options("BottomCenter", "Deck", false))
		assert(centre[2].position.X == 0, "a centred card does not shift")
		assert(math.abs(centre[2].position.Y + 14) < 1e-9, "a bottom deck peeks upward")
	end,
	["the stack bounds cover every card and the gaps between them"] = function()
		local placement = M.placementOf("TopRight")
		local targets = M.stackTargets(cards, options("TopRight", "List", true))
		local bounds = M.stackBounds(cards, targets, placement)
		near(bounds.min, Vector2.new(-300, 0), "the stack grows left from a right anchor")
		near(bounds.max, Vector2.new(0, 200), "and down past every card and gap")
		assert(M.stackBounds({}, {}, placement) == nil, "an empty stack has no bounds")
	end,
	["a swipe dismisses past its distance or when flung"] = function()
		assert(M.swipeDismisses(48, 0), "far enough dismisses")
		assert(not M.swipeDismisses(30, 100), "short and slow glides back")
		assert(M.swipeDismisses(20, 800), "a fling past the slop dismisses")
		assert(not M.swipeDismisses(4, 2000), "a fling inside the slop does not")
		assert(M.swipeOffset(30) == 30 and M.swipeOffset(-1000) > -12, "back toward the screen is damped")
	end,
}
