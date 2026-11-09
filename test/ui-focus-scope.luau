local harness = require("./lib/ui-harness")
local function button(h, name: string, options: any?)
	local selection = h.runtime.useSelection(options or {})
	return h.vide.jsx("textbutton", { Name = name, action = selection.action })
end
local function scope(h, props)
	return h.require("focus-scope").FocusScope(props)
end
-- Lune yields when it first loads a module, which Vide forbids inside a reactive scope.
local function setup()
	local h = harness()
	h.require("focus-scope")
	h.require("use-press")
	return h
end
return {
	["nested trapped scopes give selection and close input to the topmost and restore the opener"] = function()
		local h = setup()
		local inner = h.vide.source(false)
		local outerButton, innerButton
		local outerClosed, innerClosed = 0, 0
		h.mount("Base", function()
			return scope(h, {
				trapped = true,
				onClose = function()
					outerClosed += 1
				end,
				children = function()
					outerButton = button(h, "Outer", { selectFirst = true })
					return {
						outerButton,
						h.vide.show(inner, function()
							return scope(h, {
								trapped = true,
								onClose = function()
									innerClosed += 1
									inner(false)
								end,
								children = function()
									innerButton = button(h, "Inner", { selectFirst = true })
									return innerButton
								end,
							})
						end),
					}
				end,
			})
		end)
		h.flush()
		assert(h.gui.SelectedObject == outerButton, "the outer trap selects its first control")
		inner(true)
		h.flush()
		assert(h.gui.SelectedObject == innerButton and not outerButton.Selectable, "the inner trap owns selection")
		h.finish(h.begin(h.enum.KeyCode.ButtonB))
		assert(innerClosed == 1 and outerClosed == 0, "close input reaches only the topmost trap")
		h.flush()
		assert(h.gui.SelectedObject == outerButton and outerButton.Selectable, "closing restores the opener")
		h.finish(h.begin(h.enum.KeyCode.ButtonB))
		assert(outerClosed == 1, "the outer trap takes close input again")
		h.stop()
	end,
	["a trapped scope restores the base control that was selected when it opened unless restoreFocus is false"] = function()
		for _, restoreFocus in { true, false } do
			local h = setup()
			local trigger = h.control("Base", "Trigger")
			h.select(trigger)
			local open = h.vide.source(true)
			local inside
			h.mount("Base", function()
				return h.vide.show(open, function()
					return scope(h, {
						trapped = true,
						restoreFocus = restoreFocus,
						children = function()
							inside = button(h, "Inside", { selectFirst = true })
							return inside
						end,
					})
				end)
			end)
			h.flush()
			assert(h.gui.SelectedObject == inside and h.runtime.modalOpen(), "the scope takes selection")
			open(false)
			h.flush()
			assert(not h.runtime.modalOpen(), "closing releases the base")
			assert(h.gui.SelectedObject == (if restoreFocus then trigger else nil), "restoreFocus decides restoration")
			h.stop()
		end
	end,
	["restoration falls back to the parent scope when the opener is disabled or gone"] = function()
		for _, breakOpener in { "disable", "hide", "remove" } do
			local h = setup()
			local inner = h.vide.source(false)
			local disabled = h.vide.source(false)
			local present = h.vide.source(true)
			local opener, fallback
			h.mount("Base", function()
				return scope(h, {
					trapped = true,
					children = function()
						fallback = h.runtime.closeFallbackContext(true, function()
							return button(h, "Close")
						end)
						return {
							fallback,
							h.vide.show(present, function()
								local press = h.require("use-press").usePress({
									onPress = function() end,
									disabled = disabled,
									selectFirst = true,
								})
								opener = h.vide.jsx("textbutton", { Name = "Opener", action = press.action })
								return opener
							end),
							h.vide.show(inner, function()
								return scope(h, {
									trapped = true,
									children = function()
										return button(h, "Inner", { selectFirst = true })
									end,
								})
							end),
						}
					end,
				})
			end)
			h.flush()
			assert(h.gui.SelectedObject == opener, "the opener starts selected")
			inner(true)
			h.flush()
			if breakOpener == "disable" then
				disabled(true)
			elseif breakOpener == "hide" then
				opener.Visible = false
			else
				present(false)
			end
			inner(false)
			h.flush()
			assert(h.gui.SelectedObject == fallback, `a {breakOpener}d opener falls back to the parent's close control`)
			h.stop()
		end
	end,
	["a scope closed out of order hands its restore target to the scope it opened"] = function()
		local h = setup()
		local trigger = h.control("Base", "Trigger")
		h.select(trigger)
		local first = h.root(10)
		local opener = h.control(first, "Opener", { selectFirst = true })
		h.flush()
		assert(h.gui.SelectedObject == opener, "the first scope selects its opener")
		local second = h.root(20)
		h.control(second, "Confirm", { selectFirst = true })
		h.flush()
		h.runtime.closeScope(first)
		h.flush()
		h.runtime.closeScope(second)
		h.flush()
		assert(h.gui.SelectedObject == trigger, "the base trigger is restored through the closed first scope")
		h.stop()
	end,
	["an untrapped scope returns selection to what it held before it mounted"] = function()
		for _, owner in { "Trapped", "Base" } do
			local h = setup()
			local open = h.vide.source(false)
			local before, inside
			local function content()
				before = button(h, "Before", { selectFirst = true })
				return {
					before,
					h.vide.show(open, function()
						return scope(h, {
							children = function()
								inside = button(h, "Inside")
								return inside
							end,
						})
					end),
				}
			end
			h.mount("Base", function()
				if owner == "Trapped" then
					return scope(h, { trapped = true, children = content })
				end
				return content()
			end)
			h.flush()
			h.select(before)
			open(true)
			h.flush()
			h.select(inside)
			open(false)
			h.flush()
			assert(h.gui.SelectedObject == before, `{owner}: unmounting the scope restores the earlier control`)
			h.stop()
		end
	end,
	["an untrapped scope leaves selection alone when it is elsewhere"] = function()
		local h = setup()
		local open = h.vide.source(true)
		local before = h.control("Base", "Before")
		local elsewhere = h.control("Base", "Elsewhere")
		h.select(before)
		h.mount("Base", function()
			return h.vide.show(open, function()
				return scope(h, {
					children = function()
						return button(h, "Inside")
					end,
				})
			end)
		end)
		h.select(elsewhere)
		open(false)
		h.flush()
		assert(h.gui.SelectedObject == elsewhere, "selection outside the scope stays put")
		h.stop()
	end,
	["closing returns selection to the control that opened the scope, never to a top bar icon it did not open"] = function()
		for _, case in
			{
				{ device = "Gamepad", opener = "Portrait", expected = "Portrait" },
				{ device = "Gamepad", opener = "Daily", expected = "Daily" },
				{ device = "Keyboard", opener = "Portrait", expected = nil },
				{ device = "Gamepad", opener = nil, expected = nil },
			}
		do
			local h = setup()
			local open = h.vide.source(false)
			local function opens()
				open(true)
			end
			local controls = {
				Daily = h.press("Topbar", "Daily", { onPress = opens }),
				Portrait = h.press("Base", "Portrait", { onPress = opens }),
			}
			h.mount("Base", function()
				return h.vide.show(open, function()
					return scope(h, {
						trapped = true,
						onClose = function()
							open(false)
						end,
						children = function()
							return button(h, "Invite", { selectFirst = true })
						end,
					})
				end)
			end)
			local label = `{case.device} opened by {case.opener or "the game"}`
			h.select(controls.Daily)
			h.device(case.device)
			h.flush()
			if case.opener then
				controls[case.opener].Activated:Fire()
			else
				opens()
			end
			h.flush()
			assert(h.runtime.modalOpen(), `{label}: the press opens the scope`)
			h.finish(h.begin(h.enum.KeyCode.ButtonB))
			h.flush()
			assert(not h.runtime.modalOpen(), `{label}: close input closes it`)
			local selected = h.gui.SelectedObject
			assert(
				selected == (case.expected and controls[case.expected]),
				`{label}: expected {case.expected}, got {selected and selected.Name}`
			)
			h.stop()
		end
	end,
}
