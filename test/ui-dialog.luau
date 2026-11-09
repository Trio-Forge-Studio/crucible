local harness = require("./lib/ui-harness")
local function setup()
	local h = harness()
	local module = h.require("dialog")
	h.Dialog, h.AlertDialog = module.Dialog, module.AlertDialog
	return h
end
local function pressB(h)
	h.finish(h.begin(h.enum.KeyCode.ButtonB))
end
return {
	["dialog and alert dialog parts outside their root raise a named error"] = function()
		local h = setup()
		for _, case in
			{
				{ h.Dialog, "Dialog", "Trigger" },
				{ h.Dialog, "Dialog", "Content" },
				{ h.Dialog, "Dialog", "Close" },
				{ h.AlertDialog, "AlertDialog", "Content" },
				{ h.AlertDialog, "AlertDialog", "Action" },
				{ h.AlertDialog, "AlertDialog", "Cancel" },
			}
		do
			local parts, root, part = case[1], case[2], case[3]
			local ok, message = pcall(function()
				h.mount("Base", function()
					return parts[part]({ children = function() end })
				end)
			end)
			local expected = `{root}.{part} must be used inside {root}.Root`
			assert(not ok and string.find(message, expected, 1, true), `{root}.{part}: {message}`)
		end
		h.stop()
	end,
	["the trigger opens a trapped dialog, close input and Close dismiss it, and selection returns to the trigger"] = function()
		local h = setup()
		local reported = {}
		local trigger, close, content, binding
		local mounts = 0
		h.mount("Base", function()
			return h.Dialog.Root({
				onOpenChange = function(open, reason)
					table.insert(reported, `{open}:{reason}`)
				end,
				children = function(dialog)
					binding = dialog
					trigger = h.Dialog.Trigger({ Name = "Open" })
					return {
						trigger,
						h.Dialog.Content({
							Name = "Settings",
							children = function()
								mounts += 1
								local other = h.vide.jsx("textbutton", { action = h.runtime.useSelection({}).action })
								close = h.Dialog.Close({ Name = "Close" })
								content = h.vide.jsx("frame", {}, other, close)
								return content
							end,
						}),
					}
				end,
			})
		end)
		h.flush()
		assert(mounts == 0 and not h.runtime.modalOpen(), "closed content stays unmounted")
		h.select(trigger)
		trigger.Activated:Fire()
		h.flush()
		assert(binding.open() and reported[1] == "true:Trigger" and mounts == 1, "the trigger opens the dialog")
		assert(h.runtime.modalOpen() and not trigger.Selectable, "the content traps selection")
		assert(h.gui.SelectedObject == close, "the Close part is the fallback selection over earlier controls")
		pressB(h)
		h.flush()
		assert(not binding.open() and reported[2] == "false:Dismiss", "close input closes as Dismiss")
		assert(h.gui.SelectedObject == trigger, "selection returns to the trigger")
		trigger.Activated:Fire()
		h.flush()
		close.Activated:Fire()
		h.flush()
		assert(
			not binding.open() and reported[4] == "false:Close" and #reported == 4 and mounts == 2,
			"Close closes as Close and content remounts per open"
		)
		assert(h.gui.SelectedObject == trigger, "Close also restores the trigger")
		h.stop()
	end,
	["dialog buttons pass their press binding to function children, with or without asChild"] = function()
		local h = setup()
		local triggerPress, closePress, actionPress
		local close, action
		h.mount("Base", function()
			return {
				h.Dialog.Root({
					defaultOpen = true,
					children = function()
						return {
							h.Dialog.Trigger({
								children = function(press)
									triggerPress = press
									return h.vide.jsx("textlabel", {})
								end,
							}),
							h.Dialog.Content({
								children = function()
									close = h.Dialog.Close({
										asChild = true,
										children = function(press)
											closePress = press
											return h.vide.jsx("textbutton", { Name = "Close" })
										end,
									})
									return close
								end,
							}),
						}
					end,
				}),
			}
		end)
		h.mount("Base", function()
			return h.AlertDialog.Root({
				defaultOpen = true,
				children = function()
					return h.AlertDialog.Content({
						children = function()
							action = h.AlertDialog.Action({
								children = function(press)
									actionPress = press
									return h.vide.jsx("textlabel", {})
								end,
							})
							return action
						end,
					})
				end,
			})
		end)
		h.flush()
		assert(triggerPress and not triggerPress.highlighted(), "Dialog.Trigger passes its binding")
		assert(close.Name == "Close" and closePress, "an asChild Close passes its binding and uses the child")
		h.select(action)
		assert(actionPress.highlighted() and not closePress.highlighted(), "each binding follows its own host")
		h.select(close)
		assert(closePress.highlighted(), "the asChild binding follows the child")
		local input = { UserInputType = h.enum.UserInputType.MouseButton1 }
		action.InputBegan:Fire(input)
		assert(actionPress.pressed() and not closePress.pressed(), "the binding reports its own press")
		h.stop()
	end,
	["a controlled dialog only reports and an asChild content child becomes the trapped root"] = function()
		local h = setup()
		local open = h.vide.source(true)
		local requests = 0
		local panel
		h.mount("Base", function()
			return h.Dialog.Root({
				open = open,
				onOpenChange = function()
					requests += 1
				end,
				children = function()
					return h.Dialog.Content({
						asChild = true,
						children = function()
							panel = h.vide.jsx("frame", { Name = "Panel" }, h.Dialog.Close({}))
							return { panel, h.vide.jsx("uicorner", {}) }
						end,
					})
				end,
			})
		end)
		h.flush()
		assert(panel.SelectionGroup == true and h.runtime.modalOpen(), "the child is the trapped root")
		pressB(h)
		h.flush()
		assert(requests == 1 and h.runtime.modalOpen(), "a controlled dialog stays open until the owner closes it")
		open(false)
		h.flush()
		assert(not h.runtime.modalOpen(), "the owner closes it")
		h.stop()
	end,
	["an alert dialog selects Action first, or Cancel when destructive, and reports one reason per close"] = function()
		for _, destructive in { false, true } do
			local h = setup()
			local open = h.vide.source(true)
			local keep = true
			local reports, presses = {}, {}
			local action, cancel
			h.mount("Base", function()
				return h.AlertDialog.Root({
					open = open,
					onOpenChange = function(value, reason)
						table.insert(reports, `{value}:{reason}`)
						if not keep then
							open(value)
						end
					end,
					destructive = destructive,
					children = function()
						return h.AlertDialog.Content({
							children = function()
								cancel = h.AlertDialog.Cancel({
									onPress = function()
										table.insert(presses, "Cancel")
									end,
								})
								action = h.AlertDialog.Action({
									onPress = function()
										table.insert(presses, "Action")
										return false
									end,
								})
								return { cancel, action }
							end,
						})
					end,
				})
			end)
			h.flush()
			assert(h.gui.SelectedObject == (if destructive then cancel else action), `destructive={destructive}`)
			action.Activated:Fire()
			assert(
				table.concat(reports, ",") == "false:Action" and open(),
				"an Action press reports Action once and a controlled owner may keep it open"
			)
			assert(h.feedback[#h.feedback] == "Press" and presses[1] == "Action", "onPress runs and never refuses")
			keep = false
			table.clear(reports)
			cancel.Activated:Fire()
			assert(table.concat(reports, ",") == "false:Cancel" and not open(), "Cancel reports Cancel once")
			open(true)
			h.flush()
			table.clear(reports)
			table.clear(presses)
			pressB(h)
			h.flush()
			assert(table.concat(reports, ",") == "false:Dismiss" and not open(), "close input reports Dismiss")
			assert(#presses == 0, "close input runs neither onPress")
			open(true)
			h.flush()
			table.clear(reports)
			action.Activated:Fire()
			h.flush()
			assert(table.concat(reports, ",") == "false:Action", "an Action that closes reports nothing else")
			h.stop()
		end
	end,
}
