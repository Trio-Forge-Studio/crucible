local M = require("./lib/ui-load")()("modal-state")
return {
	["modal order follows display order then original serial"] = function()
		local state = { entries = {} }
		state = M.enter(state, { id = "admin", order = 30, serial = 1 })
		state = M.enter(state, { id = "screen", order = 10, serial = 2 })
		assert(M.top(state).id == "admin", 'Expected M.top(state).id == "admin"')
		state = M.enter(state, { id = "nested", order = 30, serial = 3 })
		assert(M.top(state).id == "nested", 'Expected M.top(state).id == "nested"')
		state = M.enter(state, { id = "admin", order = 30, serial = 9 })
		assert(
			#state.entries == 3 and M.top(state).id == "nested",
			'Expected #state.entries == 3 and M.top(state).id == "nested"'
		)
		state = M.leave(state, "screen")
		assert(M.top(state).id == "nested", 'Expected M.top(state).id == "nested"')
		assert(M.leave(state, "missing") == state, 'Expected M.leave(state, "missing") == state')
	end,
	["pending modal blocks play and every top until attachment or disposal"] = function()
		local state = M.enter({ entries = {} }, { id = 1, order = 10, serial = 1 })
		state = M.enter(state, { id = 2, serial = 2 })
		assert(M.hasModal(state) and M.top(state) == nil, "Expected M.hasModal(state) and M.top(state) == nil")
		state = M.enter(state, { id = 2, order = 5, serial = 99 })
		assert(
			M.top(state).id == 1 and state.entries[2].serial == 2,
			"Expected M.top(state).id == 1 and state.entries[2].serial == 2"
		)
		state = M.leave(M.leave(state, 1), 2)
		assert(not M.hasModal(state) and M.top(state) == nil, "Expected not M.hasModal(state) and M.top(state) == nil")
	end,
}
