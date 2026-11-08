local M = require("./lib/ui-load")()("toast-state")
local closing = { limit = 2, overflow = "Close" }
local queue = { limit = 2, overflow = "Queue" }
local held = {
	timeout = 1,
	held = function()
		return true
	end,
}
local released = {
	timeout = 1,
	held = function()
		return false
	end,
}
local function push(state, id: string, policy, now: number, duration: number?, extra: { [string]: any }?)
	local input = { id = id, data = id, duration = duration or 4 }
	for key, value in extra or {} do
		input[key] = value
	end
	return M.add(state, input, policy, now)
end
local function phase(state, id: string): string?
	local entry = M.findToast(state, id)
	return entry and entry.phase.kind
end
local function order(state): string
	local ids = {}
	for _, entry in state.entries do
		table.insert(ids, entry.id)
	end
	return table.concat(ids, ",")
end
return {
	["a toast times out after its duration"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0).state
		assert(phase(M.tick(state, 3.9, closing, held).state, "a") == "Open", "open before its duration")
		local step = M.tick(state, 4, closing, held)
		assert(phase(step.state, "a") == "Closing", "closing at its duration")
		assert(#step.closed == 1 and step.closed[1].reason == "Timeout", "reported once as a timeout")
	end,
	["pausing keeps the remaining time across overlapping holds"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0).state
		state = M.setPaused(state, true, 1)
		state = M.setPaused(state, true, 2)
		assert(phase(M.tick(state, 100, closing, held).state, "a") == "Open", "a paused toast never times out")
		state = M.setPaused(state, false, 10)
		assert(phase(M.tick(state, 12.9, closing, held).state, "a") == "Open", "3 seconds were left at the first pause")
		assert(phase(M.tick(state, 13, closing, held).state, "a") == "Closing", "the 3 seconds run out after resuming")
	end,
	["a sticky toast never times out and needs no frames"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0, math.huge).state
		assert(phase(M.tick(state, 1e9, closing, held).state, "a") == "Open", "sticky stays open")
		assert(not M.needsTick(state), "only a sticky toast needs no ticks")
		assert(M.needsTick(push(state, "b", closing, 0).state), "a timed toast needs ticks")
	end,
	["adding an existing id replaces it in place, keeps the order and revives a closing toast"] = function()
		local limit = { limit = 3, overflow = "Close" }
		local state = push(M.emptyToasts(), "a", limit, 0).state
		state = push(state, "b", limit, 0).state
		state = push(state, "c", limit, 0).state
		state = M.close(state, "b", "Dismiss", limit, 1).state
		local step = push(state, "b", limit, 2, 4, { data = "again" })
		assert(order(step.state) == "a,b,c", "the replacement keeps its place")
		assert(phase(step.state, "b") == "Open" and #step.closed == 0, "a closing toast comes back")
		assert(M.findToast(step.state, "b").data == "again", "the data is replaced")
		assert(
			phase(M.tick(step.state, 5.9, limit, held).state, "b") == "Open",
			"the revived toast has its full duration"
		)
		local restarted = push(step.state, "a", limit, 3).state
		assert(phase(M.tick(restarted, 6.9, limit, held).state, "a") == "Open", "replacing an open toast restarts it")
		assert(phase(M.tick(restarted, 7, limit, held).state, "a") == "Closing", "its new duration runs out")
	end,
	["reviving past the limit closes another toast, never the revived one"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0).state
		state = push(state, "b", closing, 1).state
		state = M.close(state, "a", "Dismiss", closing, 2).state
		state = push(state, "c", closing, 3).state
		local step = push(state, "a", closing, 4)
		assert(phase(step.state, "a") == "Open", "the oldest toast is revived")
		assert(phase(step.state, "b") == "Closing" and step.closed[1].reason == "Overflow", "the next oldest leaves")
	end,
	["update changes only the data"] = function()
		local action = function() end
		local state = push(M.emptyToasts(), "a", closing, 0, 4, { action = action }).state
		state = push(state, "b", closing, 0).state
		local updated = M.update(state, "a", "changed")
		local entry = M.findToast(updated, "a")
		assert(entry.data == "changed" and entry.action == action, "data changes and the action stays")
		assert(order(updated) == "a,b", "the order stays")
		assert(phase(M.tick(updated, 4, closing, held).state, "a") == "Closing", "the timer is not restarted")
		assert(M.update(updated, "missing", "x") == nil, "a missing toast is not updated")
		local leaving = M.close(updated, "a", "Close", closing, 1).state
		assert(M.update(leaving, "a", "x") == nil, "a closing toast is not updated")
	end,
	["overflow closes the oldest timed toast and spares sticky ones"] = function()
		local state = push(M.emptyToasts(), "sticky", closing, 0, math.huge).state
		state = push(state, "old", closing, 1).state
		local step = push(state, "new", closing, 2)
		assert(#step.closed == 1 and step.closed[1].id == "old", "the timed toast makes room")
		assert(step.closed[1].reason == "Overflow", "it closes as overflow")
		assert(phase(step.state, "sticky") == "Open" and phase(step.state, "new") == "Open", "sticky and new stay")
		local stickies = push(M.emptyToasts(), "first", closing, 0, math.huge).state
		stickies = push(stickies, "second", closing, 1, math.huge).state
		local full = push(stickies, "timed", closing, 2)
		assert(full.closed[1].id == "first", "with only sticky toasts open, the oldest one makes room")
	end,
	["closing toasts do not count toward the limit"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0).state
		state = push(state, "b", closing, 0).state
		state = M.close(state, "a", "Dismiss", closing, 1).state
		local step = push(state, "c", closing, 2)
		assert(#step.closed == 0, "nothing overflows while a toast is leaving")
		assert(phase(step.state, "b") == "Open" and phase(step.state, "c") == "Open", "both open toasts stay")
	end,
	["a queue admits waiting toasts in arrival order and starts their timers on admission"] = function()
		local state = push(M.emptyToasts(), "a", queue, 0, math.huge).state
		state = push(state, "b", queue, 0, math.huge).state
		state = push(state, "c", queue, 1).state
		state = push(state, "d", queue, 2).state
		assert(phase(state, "c") == "Queued" and phase(state, "d") == "Queued", "toasts past the limit wait")
		assert(phase(M.tick(state, 100, queue, held).state, "c") == "Queued", "a waiting toast never times out")
		state = M.close(state, "a", "Close", queue, 3).state
		assert(phase(state, "c") == "Open" and phase(state, "d") == "Queued", "the oldest waiting toast opens")
		assert(phase(M.tick(state, 6.9, queue, held).state, "c") == "Open", "its timer started when it opened")
		state = M.tick(state, 7, queue, held).state
		assert(phase(state, "c") == "Closing" and phase(state, "d") == "Open", "the next one opens when it leaves")
		assert(phase(M.tick(state, 10.9, queue, held).state, "d") == "Open", "its timer also started on admission")
	end,
	["closing a queued toast removes it and still reports it"] = function()
		local single = { limit = 1, overflow = "Queue" }
		local state = push(M.emptyToasts(), "a", single, 0).state
		state = push(state, "b", single, 0).state
		local step = M.close(state, "b", "Close", single, 1)
		assert(M.findToast(step.state, "b") == nil, "a queued toast never shown is removed")
		assert(#step.closed == 1 and step.closed[1].id == "b" and step.closed[1].reason == "Close", "it is reported")
		assert(phase(step.state, "a") == "Open", "the open toast is untouched")
	end,
	["closing twice is a no-op that keeps the first reason"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0).state
		state = M.close(state, "a", "Dismiss", closing, 1).state
		local again = M.close(state, "a", "Close", closing, 2)
		assert(again.state == state and #again.closed == 0, "the second close changes nothing")
		assert(M.findToast(again.state, "a").phase.reason == "Dismiss", "the first reason stays")
		assert(#M.close(state, "missing", "Close", closing, 2).closed == 0, "closing a missing toast reports nothing")
	end,
	["clear closes open and queued toasts without promoting any"] = function()
		local state = push(M.emptyToasts(), "a", queue, 0).state
		state = push(state, "b", queue, 0).state
		state = push(state, "c", queue, 0).state
		local step = M.closeAll(state, "Close", 1)
		assert(#step.closed == 3, "every toast is reported")
		assert(phase(step.state, "c") == nil, "the waiting toast is removed, not shown")
		assert(phase(step.state, "a") == "Closing" and phase(step.state, "b") == "Closing", "open toasts leave")
	end,
	["a closing toast leaves when its exit is released or times out"] = function()
		local state = push(M.emptyToasts(), "a", closing, 0).state
		state = M.close(state, "a", "Dismiss", closing, 1).state
		assert(phase(M.tick(state, 1.5, closing, held).state, "a") == "Closing", "a held exit stays")
		assert(phase(M.tick(state, 1.5, closing, released).state, "a") == nil, "a released exit leaves")
		assert(phase(M.tick(state, 1.99, closing, held).state, "a") == "Closing", "held inside the exit timeout")
		assert(phase(M.tick(state, 2, closing, held).state, "a") == nil, "the exit timeout bounds a held exit")
		local timedOut = M.tick(push(M.emptyToasts(), "b", closing, 0).state, 4, closing, released).state
		assert(phase(timedOut, "b") == "Closing", "a toast that times out keeps one frame of its exit")
	end,
}
