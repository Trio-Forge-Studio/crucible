local M = require("./lib/ui-load")()("press-state")
return {
	["hold fires once after a second and requires fresh begin"] = function()
		local mode = { kind = "Hold", seconds = 1 }
		local s = M.transition(M.idlePress(), mode, { kind = "Begin", source = "a" }).state
		local d = M.transition(s, mode, { kind = "Tick", delta = 0.9 })
		assert(not d.fire and d.state.progress == 0.9, "Expected not d.fire and d.state.progress == 0.9")
		d = M.transition(d.state, mode, { kind = "Tick", delta = 0.1 })
		assert(d.fire, "Expected d.fire")
		d = M.transition(d.state, mode, { kind = "Tick", delta = 2 })
		assert(not d.fire, "Expected not d.fire")
		assert(
			not M.transition(d.state, mode, { kind = "End", source = "a" }).fire,
			'Expected not M.transition(d.state, mode, { kind = "End", source = "a" }).fire'
		)
	end,
	["press has one source and cancellation drains without activation"] = function()
		local mode = { kind = "Hold", seconds = 1 }
		local s = M.transition(M.idlePress(), mode, { kind = "Begin", source = "touch" }).state
		assert(
			M.transition(s, mode, { kind = "Begin", source = "a" }).state == s,
			'Expected M.transition(s, mode, { kind = "Begin", source = "a" }).state == s'
		)
		assert(
			not M.transition(s, mode, { kind = "End", source = "a" }).ended,
			'Expected not M.transition(s, mode, { kind = "End", source = "a" }).ended'
		)
		s = M.transition(s, mode, { kind = "Tick", delta = 0.5 }).state
		local d = M.transition(s, mode, { kind = "Cancel" })
		assert(d.ended and not d.fire, "Expected d.ended and not d.fire")
		d = M.transition(d.state, mode, { kind = "Tick", delta = 0.25 })
		assert(d.state.progress == 0 and not d.fire, "Expected d.state.progress == 0 and not d.fire")
	end,
	["repeat steps immediately delays accelerates and bounds slow frame catchup"] = function()
		local mode = { kind = "Repeat" }
		local d = M.transition(M.idlePress(), mode, { kind = "Begin", source = 1 })
		assert(d.fire, "Expected d.fire")
		d = M.transition(d.state, mode, { kind = "Tick", delta = 0.39 })
		assert(not d.fire, "Expected not d.fire")
		d = M.transition(d.state, mode, { kind = "Tick", delta = 0.01 })
		assert(d.fire and d.state.repeatInterval < 0.15, "Expected d.fire and d.state.repeatInterval < 0.15")
		for _ = 1, 30 do
			d = M.transition(d.state, mode, { kind = "Tick", delta = 0.2 })
		end
		assert(d.state.repeatInterval == 0.04, "Expected d.state.repeatInterval == 0.04")
		d = M.transition(d.state, mode, { kind = "Tick", delta = 10 })
		assert(
			d.fire and d.state.nextRepeatAt > d.state.elapsed,
			"Expected d.fire and d.state.nextRepeatAt > d.state.elapsed"
		)
		assert(
			not M.transition(d.state, mode, { kind = "End", source = 1 }).fire,
			'Expected not M.transition(d.state, mode, { kind = "End", source = 1 }).fire'
		)
	end,
}
