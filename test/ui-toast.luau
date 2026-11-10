local harness = require("./lib/ui-harness")
local roblox: any = require("@lune/roblox")
local Enum, Vector2, Vector3 = roblox.Enum, roblox.Vector2, roblox.Vector3
type View = {
	bindings: { [string]: any },
	roots: { [string]: any },
	buttons: { [string]: any },
	frame: any,
	dispose: () -> (),
}
local function toaster(h, options: { [string]: any }?)
	return h.require("toaster").createToaster(options)
end
local function show(h, toasts, props: { [string]: any }?): View
	local given = props or {}
	local Toast = h.require("toast").Toast
	local usePress = h.require("use-press").usePress
	local view = { bindings = {}, roots = {}, buttons = {} } :: any
	local function render()
		return Toast.Viewport({
			toaster = toasts,
			placement = given.placement,
			layout = given.layout,
			children = function(toast)
				local press = usePress({ onPress = toast.act, selectFirst = given.selectFirst })
				local button = h.vide.jsx("textbutton", { action = press.action })
				local root = Toast.Root({ holdExit = given.holdExit, children = button })
				view.bindings[toast.id] = toast
				view.roots[toast.id] = root
				view.buttons[toast.id] = button
				return root
			end,
		})
	end
	view.frame, view.dispose = h.mount(given.owner or "Base", function()
		return h.runtime.closeFallbackContext(given.fallback == true, render)
	end)
	h.flush()
	return view
end
local function phase(toasts, id: string): string?
	for _, record in toasts.toasts() do
		if record.id == id then
			return record.phase
		end
	end
	return nil
end
local function settle(h)
	for _ = 1, 120 do
		h.tick(1 / 60)
	end
end
local function touch(x: number, y: number)
	return { UserInputType = Enum.UserInputType.Touch, Position = Vector3.new(x, y, 0) }
end
return {
	["a toaster works with no Vide root and closes at once without a viewport"] = function()
		local h = harness()
		local toasts = toaster(h, { limit = 2 })
		local reasons = {}
		local id = toasts.add({
			data = "Saved",
			onClose = function(reason)
				table.insert(reasons, reason)
			end,
		})
		assert(phase(toasts, id) == "Open", "the toast opens outside any root")
		assert(
			toasts.update(id, function(previous)
				return previous .. "!"
			end),
			"update accepts a function of the previous data"
		)
		assert(toasts.toasts()[1].data == "Saved!", "the data changes")
		local seen = 0
		local stop = h.vide.root(function()
			h.vide.effect(function()
				seen = #toasts.toasts()
			end)
		end)
		toasts.add({ id = "second", data = "Again" })
		assert(seen == 2, "toasts() is reactive")
		toasts.close(id)
		assert(phase(toasts, id) == nil and reasons[1] == "Close", "with no viewport a closed toast leaves at once")
		assert(not toasts.update(id, "x"), "a removed toast can't be updated")
		stop()
		h.stop()
	end,
	["timers count only while a viewport shows the toasts"] = function()
		local h = harness()
		local toasts = toaster(h)
		toasts.add({ id = "a", data = "a", duration = 1 })
		h.tick(5)
		assert(phase(toasts, "a") == "Open", "no viewport, no countdown")
		local view = show(h, toasts)
		h.tick(0.5)
		assert(phase(toasts, "a") == "Open", "half the duration has run")
		h.tick(0.6)
		assert(phase(toasts, "a") == "Closing", "the duration runs out while shown")
		h.tick(0.01)
		assert(phase(toasts, "a") == nil, "without holdExit the toast leaves on the next frame")
		toasts.add({ id = "b", data = "b", duration = 1 })
		h.tick(0.5)
		view.dispose()
		h.tick(5)
		assert(phase(toasts, "b") == "Open", "unmounting the viewport stops the countdown")
		show(h, toasts)
		h.tick(0.4)
		assert(phase(toasts, "b") == "Open", "the remaining half second is kept")
		h.tick(0.2)
		assert(phase(toasts, "b") == "Closing", "and runs out once shown again")
		h.stop()
	end,
	["pause holds overlap and the viewport pauses on hover and focus loss"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		toasts.add({ id = "a", data = "a", duration = 1 })
		local first, second = toasts.pause(), toasts.pause()
		first()
		first()
		h.tick(5)
		assert(phase(toasts, "a") == "Open", "one released hold of two keeps the pause")
		second()
		h.tick(0.5)
		local catcher = view.frame:FindFirstChild("HoverCatcher")
		catcher.MouseEnter:Fire()
		h.tick(5)
		assert(phase(toasts, "a") == "Open", "hovering the stack pauses")
		catcher.MouseLeave:Fire()
		h.input.WindowFocusReleased:Fire()
		h.tick(5)
		assert(phase(toasts, "a") == "Open", "losing window focus pauses")
		h.input.WindowFocused:Fire()
		h.tick(0.4)
		assert(phase(toasts, "a") == "Open", "pauses keep the remaining time")
		h.tick(0.2)
		assert(phase(toasts, "a") == "Closing", "the remaining time runs out")
		h.stop()
	end,
	["act refuses on false, keeps the toast and closes once on success"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		local refuse, calls, reasons = true, 0, {}
		toasts.add({
			id = "a",
			data = "a",
			action = function()
				calls += 1
				if refuse then
					return false
				end
				return nil
			end,
			onClose = function(reason)
				table.insert(reasons, reason)
			end,
		})
		local toast, button = view.bindings.a, view.buttons.a
		assert(toast.hasAction(), "the binding reports the action")
		assert(toast.act() == false and phase(toasts, "a") == "Open", "a refused action keeps the toast")
		button.Activated:Fire()
		assert(h.feedback[#h.feedback] == "Refused", "a press that is refused plays Refused")
		refuse = false
		button.Activated:Fire()
		assert(h.feedback[#h.feedback] == "Press", "a press that succeeds plays Press")
		assert(phase(toasts, "a") == "Closing" and #reasons == 1 and reasons[1] == "Action", "success closes once")
		local count = #h.feedback
		button.Activated:Fire()
		assert(toast.act() == false and calls == 3, "a closing toast never runs its action again")
		assert(#h.feedback == count, "a press on a closing toast plays nothing")
		h.stop()
	end,
	["an action that replaces its own toast is not consumed"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		local reasons = {}
		toasts.add({
			id = "a",
			data = "Invite",
			action = function()
				toasts.add({ id = "a", data = "Joining" })
			end,
			onClose = function(reason)
				table.insert(reasons, reason)
			end,
		})
		assert(view.bindings.a.act() == false, "act reports the toast was not consumed")
		assert(phase(toasts, "a") == "Open" and view.bindings.a.data() == "Joining", "the replacement stays")
		assert(#reasons == 0, "nothing closed")
		h.stop()
	end,
	["a yielded action is not consumed when a replacement arrives meanwhile"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		toasts.add({
			id = "a",
			data = "Invite",
			duration = 1,
			action = function()
				coroutine.yield()
			end,
		})
		local result
		local thread = coroutine.create(function()
			result = view.bindings.a.act()
		end)
		coroutine.resume(thread)
		assert(result == nil, "the action is still running")
		local again = coroutine.wrap(function()
			return view.bindings.a.act()
		end)()
		assert(again == false, "a second press while it runs is refused")
		h.tick(5)
		assert(phase(toasts, "a") == "Open", "timers pause while an action runs")
		toasts.add({ id = "a", data = "Updated", duration = 1 })
		coroutine.resume(thread)
		assert(result == false, "the action finished without consuming the replacement")
		assert(phase(toasts, "a") == "Open" and view.bindings.a.data() == "Updated", "the replacement stays")
		h.stop()
	end,
	["holdExit keeps a closing toast mounted until it releases or the exit times out"] = function()
		local h = harness()
		local holding = true
		local toasts = toaster(h, { exitTimeout = 0.5 })
		local view = show(h, toasts, {
			holdExit = function()
				return holding
			end,
		})
		toasts.add({ id = "a", data = "a" })
		toasts.close("a")
		h.tick(0.1)
		assert(phase(toasts, "a") == "Closing", "a held exit stays mounted")
		assert(view.roots.a:GetAttribute("Closing") == true, "the root writes Closing")
		holding = false
		h.tick(0.1)
		assert(phase(toasts, "a") == nil, "releasing the hold removes the toast")
		holding = true
		toasts.add({ id = "b", data = "b" })
		toasts.close("b")
		h.tick(0.3)
		assert(phase(toasts, "b") == "Closing", "inside the exit timeout")
		h.tick(0.3)
		assert(phase(toasts, "b") == nil, "the exit timeout ends a hold that never releases")
		toasts.add({ id = "c", data = "c" })
		toasts.close("c")
		view.dispose()
		assert(phase(toasts, "c") == nil, "unmounting the viewport drops a held exit")
		h.stop()
	end,
	["the root writes Depth, Expanded, Highlighted and Closing"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		toasts.add({ id = "a", data = "a" })
		toasts.add({ id = "b", data = "b" })
		local a, b = view.roots.a, view.roots.b
		assert(b:GetAttribute("Depth") == 0 and a:GetAttribute("Depth") == 1, "the newest toast is in front")
		assert(a:GetAttribute("Expanded") == true and a:GetAttribute("Closing") == false, "a list is expanded")
		assert(b:GetAttribute("Highlighted") == false, "nothing is highlighted at rest")
		b.MouseEnter:Fire()
		assert(b:GetAttribute("Highlighted") == true, "the pointer highlights its toast")
		b.MouseLeave:Fire()
		h.select(view.buttons.a)
		assert(a:GetAttribute("Highlighted") == true, "selection inside a toast highlights it")
		h.select(nil)
		toasts.close("b")
		assert(b:GetAttribute("Closing") == true and b.Interactable == false, "a closing toast takes no input")
		assert(a:GetAttribute("Depth") == 0, "the next toast moves to the front")
		h.stop()
	end,
	["a collapsed deck lets only its front card take input until it expands"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts, { layout = "Deck" })
		local calls = 0
		local function action()
			calls += 1
		end
		toasts.add({ id = "a", data = "a", action = action })
		toasts.add({ id = "b", data = "b" })
		local a, b = view.roots.a, view.roots.b
		assert(a:GetAttribute("Expanded") == false, "a deck starts collapsed")
		assert(a.Interactable == false and b.Interactable == true, "only the front card takes input")
		assert(view.buttons.a.Selectable == false and view.buttons.b.Selectable, "cards behind are not selectable")
		view.bindings.a.close()
		assert(view.bindings.a.act() == false and calls == 0, "a card behind the front can't act")
		assert(phase(toasts, "a") == "Open", "or be dismissed")
		assert(view.bindings.a.collapsedHeight() == 60, "a card behind draws at the front card's height")
		view.frame:FindFirstChild("HoverCatcher").MouseEnter:Fire()
		assert(a:GetAttribute("Expanded") == true and a.Interactable == true, "hovering the stack expands it")
		assert(view.buttons.a.Selectable and view.bindings.a.collapsedHeight() == nil, "every card is reachable")
		h.stop()
	end,
	["the viewport stacks roots by measured height through any UIScale"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		toasts.add({ id = "a", data = "a", duration = math.huge })
		toasts.add({ id = "b", data = "b", duration = math.huge })
		local a, b = view.roots.a, view.roots.b
		settle(h)
		assert(a.AnchorPoint == Vector2.new(1, 0) and a.Position.X.Scale == 1, "roots anchor at the top right")
		assert(b.Position.Y.Offset == 0 and a.Position.Y.Offset == 70, "the older toast sits below the gap")
		b.AbsoluteSize = Vector2.new(100, 30)
		h.tick(1 / 60)
		local moving = a.Position.Y.Offset
		assert(moving < 70 and moving > 40, "the older toast glides toward its new place")
		settle(h)
		assert(a.Position.Y.Offset == 40, "and arrives")
		h.gui.ReducedMotionEnabled = true
		view.frame:FindFirstChild("ScaleProbe").AbsoluteSize = Vector2.new(200, 0)
		assert(a.Position.Y.Offset == 25, "screen pixels are halved under a UIScale of 2, and reduced motion jumps")
		h.stop()
	end,
	["a touch swipe toward the edge dismisses without pressing the toast's button"] = function()
		local h = harness()
		h.device("Touch")
		local toasts = toaster(h)
		local view = show(h, toasts, {
			holdExit = function()
				return true
			end,
		})
		local calls, reasons = 0, {}
		local function add(id: string)
			toasts.add({
				id = id,
				data = id,
				action = function()
					calls += 1
				end,
				onClose = function(reason)
					table.insert(reasons, reason)
				end,
			})
		end
		add("a")
		local finger = touch(50, 30)
		h.input.InputBegan:Fire(finger)
		finger.Position = Vector3.new(70, 30, 0)
		h.input.InputChanged:Fire(finger)
		assert(view.roots.a:GetAttribute("Swiping") == true, "a drag along the swipe axis claims the swipe")
		assert(view.roots.a.Position.X.Offset == 20, "the claimed toast follows the finger")
		h.input.InputEnded:Fire(finger)
		assert(phase(toasts, "a") == "Open" and view.bindings.a.swipe() == 0, "a short slow swipe does not dismiss")
		view.buttons.a.Activated:Fire()
		assert(calls == 0 and #h.feedback == 0, "the release of a swipe never presses")
		assert(view.roots.a.Position.X.Offset == 20, "it glides back rather than jumping")
		settle(h)
		assert(view.roots.a.Position.X.Offset == 0, "and comes to rest at its place")
		finger = touch(50, 30)
		h.input.InputBegan:Fire(finger)
		finger.Position = Vector3.new(120, 30, 0)
		h.input.InputChanged:Fire(finger)
		h.input.InputEnded:Fire(finger)
		view.buttons.a.Activated:Fire()
		assert(phase(toasts, "a") == "Closing" and reasons[1] == "Swipe", "a long swipe dismisses with Swipe")
		h.tick(1 / 60)
		assert(view.roots.a.Position.X.Offset > 70, "the dismissed toast flings on past the edge")
		assert(calls == 0, "the dismissing release never presses either")
		h.tick(1 / 60)
		add("b")
		finger = touch(50, 30)
		h.input.InputBegan:Fire(finger)
		finger.Position = Vector3.new(50, 90, 0)
		h.input.InputChanged:Fire(finger)
		assert(view.roots.b:GetAttribute("Swiping") == false, "a drag across the swipe axis is not a swipe")
		h.stop()
	end,
	["a swipe dismisses only a toast it reaches, never one under a modal, hidden or covered"] = function()
		local h = harness()
		h.device("Touch")
		local toasts = toaster(h, { duration = math.huge })
		local view = show(h, toasts)
		local reasons = {}
		toasts.add({
			id = "a",
			data = "a",
			onClose = function(reason)
				table.insert(reasons, reason)
			end,
		})
		h.flush()
		local function swipe()
			local finger = touch(50, 30)
			h.input.InputBegan:Fire(finger)
			finger.Position = Vector3.new(120, 30, 0)
			h.input.InputChanged:Fire(finger)
			h.input.InputEnded:Fire(finger)
			h.tick(1 / 60)
		end
		local modal, modalRoot = h.root(10)
		local cover = h.instance("Frame", modal)
		cover.AbsoluteSize = Vector2.new(1000, 1000)
		cover.Active = true
		h.flush()
		swipe()
		assert(phase(toasts, "a") == "Open" and #reasons == 0, "a swipe on a modal over the toast dismisses nothing")
		h.runtime.closeScope(modal)
		modalRoot.Parent = nil
		h.flush()
		view.roots.a.Visible = false
		swipe()
		assert(phase(toasts, "a") == "Open", "a hidden toast is not swiped")
		view.roots.a.Visible = true
		local other = h.instance("Frame")
		other.Parent = h.screen(5)
		other.AbsoluteSize = Vector2.new(200, 200)
		swipe()
		assert(phase(toasts, "a") == "Open", "a swipe on another GUI drawn over the toast misses it")
		other.Parent = nil
		local finger = touch(50, 30)
		h.input.InputBegan:Fire(finger)
		finger.Position = Vector3.new(70, 30, 0)
		h.input.InputChanged:Fire(finger)
		assert(view.roots.a:GetAttribute("Swiping") == true, "an uncovered swipe is claimed")
		local late, lateRoot = h.root(10)
		h.flush()
		assert(view.roots.a:GetAttribute("Swiping") == false, "a modal opening mid-swipe cancels it")
		assert(view.bindings.a.swipe() == 0, "and puts the toast back")
		finger.Position = Vector3.new(120, 30, 0)
		h.input.InputChanged:Fire(finger)
		h.input.InputEnded:Fire(finger)
		assert(phase(toasts, "a") == "Open" and #reasons == 0, "the cancelled swipe's release dismisses nothing")
		h.runtime.closeScope(late)
		lateRoot.Parent = nil
		h.flush()
		h.tick(1 / 60)
		swipe()
		assert(phase(toasts, "a") ~= "Open" and #reasons == 1 and reasons[1] == "Swipe", "an uncovered swipe dismisses")
		h.stop()
	end,
	["a control inside a toast is never chosen by restore or fallback selection"] = function()
		local h = harness()
		local toasts = toaster(h)
		local view = show(h, toasts)
		local scope
		toasts.add({
			id = "a",
			data = "a",
			action = function()
				scope = h.root(10)
				return false
			end,
		})
		h.flush()
		local button = view.buttons.a
		assert(button.Selectable, "native navigation can reach a toast control")
		h.select(button)
		button.Activated:Fire()
		h.flush()
		h.runtime.closeScope(scope)
		h.flush()
		assert(h.gui.SelectedObject == nil, "closing a screen a toast opened does not select the toast")

		local id, root = h.root(20)
		local close = h.control(id, "Close", nil, true)
		local scoped = toaster(h)
		scoped.add({ id = "b", data = "b" })
		local inside = show(h, scoped, { owner = id, selectFirst = true, fallback = true })
		inside.frame.Parent = root
		h.flush()
		assert(h.gui.SelectedObject == close, "a scope's fallback skips a toast control asking to be selected first")
		h.stop()
	end,
}
