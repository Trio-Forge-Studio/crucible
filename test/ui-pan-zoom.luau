local harness = require("./lib/ui-harness")
local roblox: any = require("@lune/roblox")
local Enum, Vector2, Vector3 = roblox.Enum, roblox.Vector2, roblox.Vector3
local function near(actual, expected, label: string)
	assert((actual - expected).Magnitude < 1e-3, `{label}: expected {expected}, got {actual}`)
end
local function close(actual: number, expected: number, label: string)
	assert(math.abs(actual - expected) < 1e-4, `{label}: expected {expected}, got {actual}`)
end
-- The harness has no layout engine, so specs place the viewport as Roblox would: `size` is logical and
-- `scale` is the product of the UIScales above it.
local function layout(viewport, size, position, scale: number?)
	local factor = scale or 1
	viewport:FindFirstChild("ScaleProbe").AbsoluteSize = Vector2.new(100 * factor, 0)
	viewport.AbsolutePosition = position
	viewport.AbsoluteSize = size * factor
end
local function mount(h, given, owner: string?)
	local viewport = h.instance("Frame", owner or "Base")
	viewport.AbsoluteSize = Vector2.zero
	local content = h.instance("Frame", owner or "Base")
	content.Parent = viewport
	local binding, dispose = h.mount(owner or "Base", function()
		local result = h.require("use-pan-zoom").usePanZoom(given)
		result.content(content)
		result.viewport(viewport)
		return result
	end)
	return binding, viewport, content, dispose
end
local function pointer(kind: string, x: number, y: number, z: number?)
	return { UserInputType = Enum.UserInputType[kind], Position = Vector3.new(x, y, z or 0) }
end
local function stick(h, keyCode, x: number, y: number)
	h.input.InputChanged:Fire({
		KeyCode = Enum.KeyCode[keyCode],
		UserInputType = Enum.UserInputType.Gamepad1,
		Position = Vector3.new(x, y, 0),
	})
end
local function fitOne()
	return 1
end
return {
	["pan zoom drags with the mouse and with only the touch that started"] = function()
		local h = harness()
		local binding, viewport = mount(h, { contentSize = Vector2.new(1600, 1200), fit = fitOne })
		layout(viewport, Vector2.new(800, 600), Vector2.new(100, 50))
		local mouse = pointer("MouseButton1", 500, 300)
		viewport.InputBegan:Fire(pointer("MouseButton2", 500, 300))
		assert(not binding.dragging(), "the right button does not drag")
		viewport.InputBegan:Fire(mouse)
		assert(binding.dragging() and viewport:GetAttribute("Dragging") == true, "a left press starts a drag")
		h.input.InputChanged:Fire(pointer("MouseMovement", 560, 260))
		near(binding.view().pan, Vector2.new(60, -40), "the pan follows the pointer")
		h.input.InputChanged:Fire(pointer("Touch", 900, 900))
		near(binding.view().pan, Vector2.new(60, -40), "other input does not move a mouse drag")
		h.input.InputEnded:Fire(mouse)
		assert(not binding.dragging() and viewport:GetAttribute("Dragging") == false, "release ends the drag")
		h.input.InputChanged:Fire(pointer("MouseMovement", 700, 400))
		near(binding.view().pan, Vector2.new(60, -40), "moving after release pans nothing")
		local touch = pointer("Touch", 300, 300)
		viewport.InputBegan:Fire(touch)
		h.input.InputChanged:Fire(pointer("Touch", 900, 900))
		near(binding.view().pan, Vector2.new(60, -40), "a second touch does not move the drag")
		touch.Position = Vector3.new(250, 330, 0)
		h.input.InputChanged:Fire(touch)
		near(binding.view().pan, Vector2.new(10, -10), "the dragging touch pans")
		h.input.InputEnded:Fire(touch)
		assert(not binding.dragging(), "lifting the touch ends the drag")
		h.stop()
	end,
	["pan zoom converts screen pixels through any UIScale above it"] = function()
		local h = harness()
		local seen
		local binding, viewport = mount(h, {
			contentSize = Vector2.new(4000, 4000),
			fit = function(size)
				seen = size
				return 0.5
			end,
		})
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0), 0.5)
		near(seen, Vector2.new(800, 600), "fit receives the logical viewport size")
		near(binding.half(), Vector2.new(400, 300), "half is in logical pixels")
		local mouse = pointer("MouseButton1", 200, 150)
		viewport.InputBegan:Fire(mouse)
		h.input.InputChanged:Fire(pointer("MouseMovement", 250, 150))
		near(binding.view().pan, Vector2.new(200, 0), "50 screen pixels are 100 logical pixels at fit 0.5")
		h.input.InputEnded:Fire(mouse)
		viewport:FindFirstChild("ScaleProbe").AbsoluteSize = Vector2.new(25, 0)
		near(seen, Vector2.new(1600, 1200), "a root scale change alone fits again")
		near(binding.half(), Vector2.new(800, 600), "a root scale change alone measures half again")
		h.stop()
	end,
	["the wheel zooms by 1.15 per notch around the cursor where the viewport is uncovered"] = function()
		local h = harness()
		local binding, viewport, content = mount(h, { contentSize = Vector2.new(1600, 1200) })
		layout(viewport, Vector2.new(800, 600), Vector2.new(100, 50))
		assert(binding.fit() == 0.5, "the default fit contains the content")
		local anchor = Vector2.new(200, 100)
		local cursor = Vector2.new(500, 350) + anchor
		local point = anchor / 0.5
		local function wheel(delta: number, at)
			local position = at or cursor
			h.input.InputChanged:Fire(pointer("MouseWheel", position.X, position.Y, delta))
		end
		wheel(1)
		close(binding.view().zoom, 1.15, "one notch zooms in by 1.15")
		near((point + binding.view().pan) * binding.scale(), anchor, "the point under the cursor stays")
		wheel(1, Vector2.new(50, 20))
		close(binding.view().zoom, 1.15, "the wheel outside the viewport does nothing")
		local cover = h.instance("Frame")
		cover.Parent = h.screen(5)
		cover.AbsolutePosition = cursor - Vector2.new(10, 10)
		cover.AbsoluteSize = Vector2.new(20, 20)
		wheel(1)
		close(binding.view().zoom, 1.15, "an opaque frame above the cursor blocks the wheel")
		cover.BackgroundTransparency = 1
		wheel(-1)
		close(binding.view().zoom, 1, "a transparent inactive frame lets the wheel through")
		cover.Active = true
		wheel(1)
		close(binding.view().zoom, 1, "an active frame above the cursor blocks the wheel")
		cover.Parent = nil
		local node = h.instance("TextButton")
		node.Parent = content
		node.Active = true
		node.AbsolutePosition = cursor - Vector2.new(10, 10)
		node.AbsoluteSize = Vector2.new(20, 20)
		wheel(1)
		close(binding.view().zoom, 1.15, "a control inside the content does not block the wheel")
		near((point + binding.view().pan) * binding.scale(), anchor, "the anchor holds across notches")
		node.Parent = nil
		local overlay = h.instance("Frame")
		overlay.Parent = viewport
		overlay.AbsolutePosition = cursor - Vector2.new(10, 10)
		overlay.AbsoluteSize = Vector2.new(20, 20)
		wheel(1)
		close(
			binding.view().zoom,
			1.15,
			"an opaque overlay inside the viewport but outside the content blocks the wheel"
		)
		overlay.Parent = nil
		wheel(10)
		close(binding.view().zoom, 1.6, "zoom stops at the default maximum")
		wheel(-30)
		close(binding.view().zoom, 0.5, "zoom stops at the default minimum")
		near(binding.view().pan, Vector2.zero, "content smaller than the viewport centres")
		h.stop()
	end,
	["a pinch over the viewport zooms around its midpoint and cancels a drag"] = function()
		local h = harness()
		local binding, viewport = mount(h, { contentSize = Vector2.new(1600, 1200) })
		layout(viewport, Vector2.new(800, 600), Vector2.new(100, 50))
		local touch = pointer("Touch", 300, 300)
		viewport.InputBegan:Fire(touch)
		assert(binding.dragging(), "the first finger drags")
		local anchor = Vector2.new(-200, 50)
		local middle = Vector2.new(500, 350) + anchor
		local fingers = { middle - Vector2.new(40, 0), middle + Vector2.new(40, 0) }
		local point = anchor / 0.5
		h.input.TouchPinch:Fire(fingers, 1, 0, Enum.UserInputState.Begin)
		assert(not binding.dragging() and viewport:GetAttribute("Dragging") == false, "a pinch cancels the drag")
		h.input.TouchPinch:Fire(fingers, 1.2, 0, Enum.UserInputState.Change)
		close(binding.view().zoom, 1.2, "the pinch scales the zoom it started from")
		near((point + binding.view().pan) * binding.scale(), anchor, "the point under the midpoint stays")
		local pan = binding.view().pan
		touch.Position = Vector3.new(200, 200, 0)
		h.input.InputChanged:Fire(touch)
		near(binding.view().pan, pan, "the cancelled drag no longer pans")
		h.input.TouchPinch:Fire(fingers, 1.2, 0, Enum.UserInputState.End)
		h.input.TouchPinch:Fire(fingers, 1.5, 0, Enum.UserInputState.Change)
		close(binding.view().zoom, 1.2, "a change after the pinch ends does nothing")
		local away = { Vector2.new(0, 0), Vector2.new(20, 0) }
		h.input.TouchPinch:Fire(away, 1, 0, Enum.UserInputState.Begin)
		h.input.TouchPinch:Fire(away, 1.3, 0, Enum.UserInputState.Change)
		close(binding.view().zoom, 1.2, "a pinch outside the viewport does nothing")
		h.stop()
	end,
	["a hidden viewport takes no wheel or pinch, and hiding it ends a drag"] = function()
		local h = harness()
		local changes = 0
		local binding, viewport = mount(h, {
			contentSize = Vector2.new(1600, 1200),
			onViewChange = function()
				changes += 1
			end,
		})
		layout(viewport, Vector2.new(800, 600), Vector2.zero)
		changes = 0
		viewport.Visible = false
		h.input.InputChanged:Fire(pointer("MouseWheel", 300, 200, 1))
		close(binding.view().zoom, 1, "the wheel over a hidden viewport does nothing")
		local fingers = { Vector2.new(280, 200), Vector2.new(320, 200) }
		h.input.TouchPinch:Fire(fingers, 1, 0, Enum.UserInputState.Begin)
		h.input.TouchPinch:Fire(fingers, 1.4, 0, Enum.UserInputState.Change)
		close(binding.view().zoom, 1, "nor does a pinch")
		assert(changes == 0, "and nothing reports a change")
		viewport.Visible = true
		h.input.InputChanged:Fire(pointer("MouseWheel", 300, 200, 1))
		close(binding.view().zoom, 1.15, "the wheel zooms once the viewport shows again")
		local mouse = pointer("MouseButton1", 400, 300)
		viewport.InputBegan:Fire(mouse)
		assert(binding.dragging(), "a press drags")
		local pan = binding.view().pan
		viewport.Visible = false
		h.input.InputChanged:Fire(pointer("MouseMovement", 500, 300))
		near(binding.view().pan, pan, "a hidden viewport does not pan")
		assert(not binding.dragging(), "and its drag ended")
		viewport.Visible = true
		h.input.TouchPinch:Fire(fingers, 1, 0, Enum.UserInputState.Begin)
		viewport.Visible = false
		h.input.TouchPinch:Fire(fingers, 1.2, 0, Enum.UserInputState.Change)
		viewport.Visible = true
		h.input.TouchPinch:Fire(fingers, 1.3, 0, Enum.UserInputState.Change)
		close(binding.view().zoom, 1.15, "hiding the viewport ended its pinch")
		h.stop()
	end,
	["the default clamp keeps content covering the viewport and clampPan replaces it"] = function()
		local h = harness()
		local binding, viewport = mount(h, { contentSize = Vector2.new(1600, 1200), fit = fitOne })
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		local mouse = pointer("MouseButton1", 400, 300)
		viewport.InputBegan:Fire(mouse)
		h.input.InputChanged:Fire(pointer("MouseMovement", 2400, -1700))
		near(binding.view().pan, Vector2.new(400, -300), "the content edges stop at the viewport edges")
		h.input.InputEnded:Fire(mouse)
		local seen = {}
		local custom, other = mount(h, {
			contentSize = Vector2.new(1600, 1200),
			fit = fitOne,
			minZoom = 0.8,
			maxZoom = 3,
			clampPan = function(pan, scale, half)
				table.insert(seen, { scale = scale, half = half })
				return if pan.Magnitude > 50 then pan.Unit * 50 else pan
			end,
		})
		layout(other, Vector2.new(800, 600), Vector2.new(0, 0))
		local drag = pointer("MouseButton1", 400, 300)
		other.InputBegan:Fire(drag)
		h.input.InputChanged:Fire(pointer("MouseMovement", 700, 300))
		near(custom.view().pan, Vector2.new(50, 0), "the custom clamp decides the pan")
		local last = seen[#seen]
		assert(last.scale == 1 and last.half == Vector2.new(400, 300), "the clamp receives the scale and half")
		h.input.InputEnded:Fire(drag)
		custom.zoomTo(10)
		assert(custom.view().zoom == 3, "zoomTo stops at maxZoom")
		custom.zoomTo(0)
		assert(custom.view().zoom == 0.8, "zoomTo stops at minZoom")
		h.stop()
	end,
	["fit re-measures on resize and the content shows once measured"] = function()
		local h = harness()
		local binding, viewport, content = mount(h, { contentSize = Vector2.new(1600, 1200) })
		assert(content.Visible == false, "the content hides until the viewport has a size")
		assert(content.AnchorPoint == Vector2.new(0.5, 0.5), "the content is centred on its position")
		assert(content.Size == roblox.UDim2.fromOffset(1600, 1200), "the content takes contentSize")
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		local scaler = content:FindFirstChild("UIScale")
		assert(content.Visible and scaler.Scale == 0.5, "measured content shows at the contained fit")
		binding.zoomTo(1.6)
		binding.panTo(Vector2.new(1000, 700))
		near(binding.view().pan, Vector2.new(-200, -100), "panTo centres the content point")
		assert(content.Position == roblox.UDim2.new(0.5, -160, 0.5, -80), "the content moves by pan times scale")
		viewport.AbsoluteSize = Vector2.new(400, 600)
		close(binding.fit(), 0.25, "a resize fits again")
		close(scaler.Scale, 0.4, "the content scales by the new fit times the zoom")
		near(binding.view().pan, Vector2.new(-200, 0), "a resize clamps the pan again")
		h.stop()
	end,
	["follow selection glides a selected node into the safe area"] = function()
		local h = harness()
		local binding, viewport, content = mount(h, { contentSize = Vector2.new(2000, 2000), fit = fitOne })
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		content.AbsolutePosition = Vector2.new(400, 300) - Vector2.new(1000, 1000)
		content.AbsoluteSize = Vector2.new(2000, 2000)
		local function place(object, point)
			object.AbsoluteSize = Vector2.new(100, 60)
			object.AbsolutePosition = content.AbsolutePosition + point - object.AbsoluteSize / 2
		end
		local near_node = h.instance("TextButton")
		near_node.Parent = content
		place(near_node, Vector2.new(1100, 1050))
		h.select(near_node)
		h.tick(1)
		near(binding.view().pan, Vector2.zero, "a node inside the safe area does not move the view")
		local outside = h.instance("TextButton")
		outside.Parent = viewport
		outside.AbsolutePosition = Vector2.new(780, 280)
		h.select(outside)
		h.tick(1)
		near(binding.view().pan, Vector2.zero, "selection outside the content does not move the view")
		local far = h.instance("TextButton")
		far.Parent = content
		place(far, Vector2.new(1850, 1000))
		h.select(far)
		h.tick(1 / 60)
		local x = binding.view().pan.X
		assert(x < 0 and x > -550, "the view glides rather than jumps")
		for _ = 1, 120 do
			h.tick(1 / 60)
		end
		near(binding.view().pan, Vector2.new(-550, 0), "the node settles 100 px inside the right edge")
		local custom, other, box = mount(h, {
			contentSize = Vector2.new(2000, 2000),
			fit = fitOne,
			followSelection = { inset = { topLeft = Vector2.zero, bottomRight = Vector2.new(0, 0) } },
		})
		layout(other, Vector2.new(800, 600), Vector2.new(0, 0))
		box.AbsolutePosition = Vector2.new(-600, -700)
		box.AbsoluteSize = Vector2.new(2000, 2000)
		local edge = h.instance("TextButton")
		edge.Parent = box
		edge.AbsoluteSize = Vector2.new(100, 60)
		edge.AbsolutePosition = Vector2.new(-600, -700) + Vector2.new(1850, 1000) - Vector2.new(50, 30)
		h.select(edge)
		for _ = 1, 120 do
			h.tick(1 / 60)
		end
		near(custom.view().pan, Vector2.new(-450, 0), "a zero inset keeps the node just inside the edge")
		local off, hidden, inner =
			mount(h, { contentSize = Vector2.new(2000, 2000), fit = fitOne, followSelection = false })
		layout(hidden, Vector2.new(800, 600), Vector2.new(0, 0))
		local node = h.instance("TextButton")
		node.Parent = inner
		node.AbsolutePosition = Vector2.new(1200, 270)
		h.select(node)
		h.tick(1)
		near(off.view().pan, Vector2.zero, "followSelection false never moves the view")
		h.stop()
	end,
	["pan zoom stops while its scope is inactive or the Roblox menu is open"] = function()
		local h = harness()
		local binding, viewport =
			mount(h, { contentSize = Vector2.new(4000, 4000), fit = fitOne, gamepad = { pan = true } })
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		local mouse = pointer("MouseButton1", 400, 300)
		viewport.InputBegan:Fire(mouse)
		binding.panTo(Vector2.new(1000, 1000), { glide = true })
		local scope = h.root(1)
		assert(not binding.dragging() and viewport:GetAttribute("Dragging") == false, "a modal ends the drag")
		h.input.InputChanged:Fire(pointer("MouseMovement", 500, 300))
		h.input.InputChanged:Fire(pointer("MouseWheel", 400, 300, 1))
		h.input.TouchPinch:Fire({ Vector2.new(380, 300), Vector2.new(420, 300) }, 1, 0, Enum.UserInputState.Begin)
		h.input.TouchPinch:Fire({ Vector2.new(380, 300), Vector2.new(420, 300) }, 1.4, 0, Enum.UserInputState.Change)
		stick(h, "Thumbstick2", 1, 0)
		h.tick(1)
		viewport.InputBegan:Fire(pointer("MouseButton1", 400, 300))
		assert(not binding.dragging(), "a press under a modal does not drag")
		assert(binding.view().pan == Vector2.zero and binding.view().zoom == 1, "nothing moves under a modal")
		h.runtime.closeScope(scope)
		h.tick(1)
		assert(binding.view().pan == Vector2.zero, "the glide cancelled by the modal stays cancelled")
		h.input.InputChanged:Fire(pointer("MouseWheel", 400, 300, 1))
		close(binding.view().zoom, 1.15, "the wheel works again once the modal closes")
		h.gui.MenuIsOpen = true
		h.input.InputChanged:Fire(pointer("MouseWheel", 400, 300, 1))
		close(binding.view().zoom, 1.15, "the Roblox menu blocks the wheel")
		h.stop()
	end,
	["gamepad pan and zoom run only when enabled and leave navigation input alone"] = function()
		local h = harness()
		local binding, viewport = mount(h, {
			contentSize = Vector2.new(4000, 4000),
			fit = fitOne,
			maxZoom = 4,
			gamepad = { pan = true, zoom = true },
		})
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		stick(h, "Thumbstick1", 1, 0)
		h.tick(0.5)
		h.finish(h.begin(Enum.KeyCode.DPadRight))
		h.tick(0.5)
		stick(h, "Thumbstick2", 0.1, 0.1)
		h.tick(0.5)
		assert(binding.view().pan == Vector2.zero, "the left stick, the D-pad and a resting right stick do nothing")
		stick(h, "Thumbstick2", 1, 0)
		h.tick(0.5)
		near(binding.view().pan, Vector2.new(-450, 0), "full right tilt moves the view 900 px a second")
		stick(h, "Thumbstick2", 0, 1)
		h.tick(0.5)
		near(binding.view().pan, Vector2.new(-450, 450), "pushing up moves the view up")
		h.input.InputEnded:Fire({ KeyCode = Enum.KeyCode.Thumbstick2, UserInputType = Enum.UserInputType.Gamepad1 })
		local zoomIn = h.begin(Enum.KeyCode.ButtonR2)
		h.tick(1)
		close(binding.view().zoom, 2, "R2 doubles the zoom each second")
		near(binding.view().pan, Vector2.new(-450, 450), "trigger zoom keeps the viewport centre")
		h.finish(zoomIn)
		stick(h, "Thumbstick2", 1, 0)
		h.tick(0.5)
		near(binding.view().pan, Vector2.new(-675, 450), "stick speed is in screen pixels at any zoom")
		h.input.InputEnded:Fire({ KeyCode = Enum.KeyCode.Thumbstick2, UserInputType = Enum.UserInputType.Gamepad1 })
		zoomIn = h.begin(Enum.KeyCode.ButtonR2)
		local zoomOut = h.begin(Enum.KeyCode.ButtonL2)
		h.tick(1)
		close(binding.view().zoom, 2, "holding both triggers holds the zoom")
		h.finish(zoomIn)
		h.tick(1)
		close(binding.view().zoom, 1, "L2 halves the zoom each second")
		h.finish(zoomOut)
		zoomIn = h.begin(Enum.KeyCode.ButtonR2)
		h.tick(3)
		close(binding.view().zoom, 4, "trigger zoom stops at maxZoom")
		h.finish(zoomIn)
		viewport.Visible = false
		stick(h, "Thumbstick2", 1, 0)
		h.tick(1)
		near(binding.view().pan, Vector2.new(-675, 450), "a hidden viewport ignores the stick")
		viewport.Visible = true
		h.editing(true)
		stick(h, "Thumbstick2", 1, 0)
		h.tick(1)
		near(binding.view().pan, Vector2.new(-675, 450), "text editing ignores the stick")
		h.editing(false)
		local plain, other = mount(h, { contentSize = Vector2.new(4000, 4000), fit = fitOne })
		layout(other, Vector2.new(800, 600), Vector2.new(0, 0))
		stick(h, "Thumbstick2", 1, 0)
		h.finish(h.begin(Enum.KeyCode.ButtonR2))
		h.tick(1)
		assert(plain.view().pan == Vector2.zero and plain.view().zoom == 1, "gamepad controls are off by default")
		h.stop()
	end,
	["a controlled view reports each change and renders what the game applies"] = function()
		local h = harness()
		local view = h.vide.source({ pan = Vector2.zero, zoom = 1 })
		local reported = {}
		local binding, viewport, content = mount(h, {
			contentSize = Vector2.new(1600, 1200),
			fit = fitOne,
			view = view,
			onViewChange = function(next)
				table.insert(reported, next)
			end,
		})
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		local mouse = pointer("MouseButton1", 400, 300)
		viewport.InputBegan:Fire(mouse)
		h.input.InputChanged:Fire(pointer("MouseMovement", 460, 300))
		near(reported[#reported].pan, Vector2.new(60, 0), "the drag reports the new view")
		assert(binding.view().pan == Vector2.zero, "the view waits for the game")
		assert(content.Position == roblox.UDim2.new(0.5, 0, 0.5, 0), "the content does not move on its own")
		view(reported[#reported])
		assert(content.Position == roblox.UDim2.new(0.5, 60, 0.5, 0), "the applied view moves the content")
		h.stop()
	end,
	["reduced motion turns glides into jumps"] = function()
		local h = harness()
		h.gui.ReducedMotionEnabled = true
		local binding, viewport, content = mount(h, { contentSize = Vector2.new(2000, 2000), fit = fitOne })
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		content.AbsolutePosition = Vector2.new(400, 300) - Vector2.new(1000, 1000)
		content.AbsoluteSize = Vector2.new(2000, 2000)
		local far = h.instance("TextButton")
		far.Parent = content
		far.AbsoluteSize = Vector2.new(100, 60)
		far.AbsolutePosition = content.AbsolutePosition + Vector2.new(1850, 1000) - far.AbsoluteSize / 2
		h.select(far)
		near(binding.view().pan, Vector2.new(-550, 0), "follow selection jumps to the safe area")
		binding.panTo(Vector2.new(1000, 1000), { glide = true })
		near(binding.view().pan, Vector2.zero, "a gliding panTo lands at once")
		h.stop()
	end,
	["panTo can glide and zoomTo keeps a content anchor in place"] = function()
		local h = harness()
		local binding, viewport = mount(h, { contentSize = Vector2.new(1600, 1200), fit = fitOne })
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		binding.panTo(Vector2.new(1000, 700), { glide = true })
		assert(binding.view().pan == Vector2.zero, "a glide starts on the next frame")
		for _ = 1, 120 do
			h.tick(1 / 60)
		end
		near(binding.view().pan, Vector2.new(-200, -100), "the glide centres the point")
		local anchor = Vector2.new(1100, 500)
		local before = (anchor - Vector2.new(800, 600) + binding.view().pan) * binding.scale()
		binding.zoomTo(1.5, anchor)
		close(binding.view().zoom, 1.5, "zoomTo sets the zoom")
		near((anchor - Vector2.new(800, 600) + binding.view().pan) * binding.scale(), before, "the anchor stays put")
		binding.zoomTo(1)
		close(binding.view().zoom, 1, "without an anchor it zooms about the centre")
		h.stop()
	end,
	["unmounting disconnects every pan zoom listener"] = function()
		local h = harness()
		local binding, viewport, content, dispose =
			mount(h, { contentSize = Vector2.new(4000, 4000), fit = fitOne, gamepad = { pan = true } })
		layout(viewport, Vector2.new(800, 600), Vector2.new(0, 0))
		local mouse = pointer("MouseButton1", 400, 300)
		viewport.InputBegan:Fire(mouse)
		binding.panTo(Vector2.new(1500, 1500), { glide = true })
		dispose()
		assert(viewport:FindFirstChild("ScaleProbe") == nil, "the probe leaves the viewport")
		assert(content:FindFirstChild("UIScale") == nil, "the scale leaves the content")
		h.input.InputChanged:Fire(pointer("MouseMovement", 500, 300))
		h.input.InputChanged:Fire(pointer("MouseWheel", 400, 300, 1))
		stick(h, "Thumbstick2", 1, 0)
		h.tick(1)
		viewport.InputBegan:Fire(pointer("MouseButton1", 400, 300))
		assert(binding.view().pan == Vector2.zero and binding.view().zoom == 1, "nothing moves after unmount")
		assert(not binding.dragging(), "unmounting ends the drag")
		h.stop()
	end,
	["pan zoom parts render a viewport and content inside their root"] = function()
		local h = harness()
		local PanZoom = h.require("pan-zoom").PanZoom
		for _, part in { "Viewport", "Content" } do
			local ok, message = pcall(function()
				h.mount("Base", function()
					return PanZoom[part]({})
				end)
			end)
			assert(not ok and string.find(message, `PanZoom.{part} must be used inside PanZoom.Root`, 1, true), message)
		end
		local viewport, content, given
		h.mount("Base", function()
			return PanZoom.Root({
				contentSize = Vector2.new(400, 300),
				children = function(binding)
					given = binding
					content = PanZoom.Content({ Name = "Canvas" })
					viewport = PanZoom.Viewport({ Name = "Map", children = content })
					return viewport
				end,
			})
		end)
		assert(viewport.Name == "Map" and content.Name == "Canvas" and content.Parent == viewport, "parts nest")
		assert(viewport.Active and viewport.BackgroundTransparency == 1, "the viewport sinks input and stays clear")
		assert(viewport.Size == roblox.UDim2.fromScale(1, 1), "the viewport fills its parent")
		assert(content.Size == roblox.UDim2.fromOffset(400, 300), "the content takes contentSize")
		assert(content:FindFirstChild("UIScale").Scale == given.scale(), "the content scales by fit times zoom")
		local image = h.instance("ImageLabel", "Base")
		local slot
		h.mount("Base", function()
			return PanZoom.Root({
				contentSize = Vector2.new(400, 300),
				children = function(binding)
					slot = binding
					return PanZoom.Viewport({ asChild = true, children = image })
				end,
			})
		end)
		image.InputBegan:Fire(pointer("MouseButton1", 10, 10))
		assert(slot.dragging() and image:GetAttribute("Dragging") == true, "an asChild viewport drags")
		assert(image.Active and image:FindFirstChild("ScaleProbe"), "an asChild viewport measures its scale")
		h.stop()
	end,
}
