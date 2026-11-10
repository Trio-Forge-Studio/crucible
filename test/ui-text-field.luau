local harness = require("./lib/ui-harness")
return {
	["uncontrolled text field owns its value from defaultValue and still reports"] = function()
		local h = harness()
		local changes, commits = {}, {}
		local box = h.instance("TextBox", "Base")
		local binding = h.mount("Base", function()
			local created = h.require("use-text-field").useTextField({
				defaultValue = "hi",
				onValueChange = function(value)
					table.insert(changes, value)
				end,
				onValueCommit = function(value)
					table.insert(commits, value)
				end,
			})
			created.box(box)
			return created
		end)
		assert(box.Text == "hi" and binding.value() == "hi", "defaultValue seeds the box")
		box:CaptureFocus()
		box.Text = "hey"
		assert(binding.value() == "hey" and changes[1] == "hey", "typing updates the owned value")
		box:ReleaseFocus(false)
		h.flush()
		assert(#changes == 1 and commits[1] == "hey", "blur commits without repeating the change")
		h.stop()
	end,
	["text field does not echo controlled writes when Text signals are deferred"] = function()
		local h = harness({ deferredText = true })
		local text = h.vide.source("a")
		local changes = {}
		local box = h.instance("TextBox", "Base")
		h.mount("Base", function()
			local binding = h.require("use-text-field").useTextField({
				value = text,
				maxLength = 3,
				onValueChange = function(value)
					table.insert(changes, value)
				end,
			})
			binding.box(box)
		end)
		text("longer")
		h.flush()
		assert(box.Text == "longer" and #changes == 0, "controlled text stays authoritative without echo")
		box.Text = "abcd"
		h.flush()
		assert(box.Text == "abc" and #changes == 1 and changes[1] == "abc", "deferred clamp reports one change")
		box.Text = "abcx"
		h.flush()
		assert(#changes == 1, "deferred repeated clamp is silent")
		h.stop()
	end,
	["text field does not repeat a clamped change or echo it on blur while controlled updates are pending"] = function()
		local h = harness()
		local changes, commits = {}, {}
		local box = h.instance("TextBox", "Base")
		h.mount("Base", function()
			local binding = h.require("use-text-field").useTextField({
				value = "a",
				maxLength = 3,
				onValueChange = function(text)
					table.insert(changes, text)
				end,
				onValueCommit = function(text)
					table.insert(commits, text)
				end,
			})
			binding.box(box)
		end)
		box:CaptureFocus()
		box.Text = "abcd"
		box.Text = "abcx"
		box:ReleaseFocus(false)
		h.flush()
		assert(#changes == 1 and changes[1] == "abc", "same clamped value and blur do not repeat onChange")
		assert(#commits == 1 and commits[1] == "abc", "blur still commits current input")
		h.stop()
	end,
	["text field binds controlled text limits changes and commits completion on Enter"] = function()
		local h = harness()
		local owner = h.root(10)
		local text = h.vide.source("he")
		local completion = h.vide.source("hello")
		local disabled = h.vide.source(false)
		local invalid = h.vide.source(false)
		local changes, submissions, commits, events = {}, {}, {}, {}
		local field = h.instance("TextButton", owner)
		local box = h.instance("TextBox", owner)
		local binding = h.mount(owner, function()
			local created = h.require("use-text-field").useTextField({
				value = text,
				completion = completion,
				disabled = disabled,
				invalid = invalid,
				maxLength = 4,
				selectFirst = true,
				onValueChange = function(value)
					table.insert(changes, value)
					table.insert(events, "change")
					text(value)
				end,
				onSubmit = function(value)
					table.insert(submissions, value)
					table.insert(events, "submit")
				end,
				onValueCommit = function(value)
					table.insert(commits, value)
					table.insert(events, "commit")
				end,
			})
			created.control(field)
			created.box(box)
			return created
		end)
		h.flush()
		local globalEditing = h.require("focus-scope").useTextEditing()
		assert(h.gui.SelectedObject == field and not box.Selectable, "only field receives gamepad selection")
		assert(not box.ClearTextOnFocus and box.TextEditable and box.Text == "he", "box properties bind")
		assert(
			field:GetAttribute("Editing") == false and field:GetAttribute("Invalid") == false,
			"field attributes start false"
		)
		assert(binding.completionText() == "hello", "completion is readable")
		field.Activated:Fire()
		assert(
			binding.editing() and globalEditing() and field:GetAttribute("Editing") == true,
			"focus uses shared editing runtime"
		)
		assert(not h.contexts.Menu, "text editing releases menu input")
		box.Text = "hello!"
		assert(box.Text == "hell" and text() == "hell" and #changes == 1, "clamp reports one change")
		box.Text = "hellx"
		assert(#changes == 1 and box.Text == "hell", "clamping to unchanged value does not notify")
		text("ha")
		assert(box.Text == "ha" and #changes == 1, "controlled updates do not echo")
		assert(binding.completionText() == "hallo", "completion keeps typed prefix")
		box:ReleaseFocus(true)
		h.flush()
		assert(text() == "hell" and box.Text == "hell" and changes[2] == "hell", "Enter accepts limited completion")
		assert(submissions[1] == "hell" and commits[1] == "hell", "Enter submits then commits")
		assert(events[2] == "change" and events[3] == "submit" and events[4] == "commit", "completion callback order")
		assert(not binding.editing() and not globalEditing() and h.contexts.Menu, "blur restores menu input")
		completion(nil)
		assert(binding.completionText() == nil, "completion can disappear")
		field.Activated:Fire()
		box:ReleaseFocus(false)
		h.flush()
		assert(#submissions == 1 and #commits == 2 and commits[2] == "hell", "ordinary blur always commits")
		invalid(true)
		disabled(true)
		assert(
			field:GetAttribute("Invalid") == true and field:GetAttribute("Disabled") == true,
			"validation and disabled styling react"
		)
		assert(not box.TextEditable, "disabled text is not editable")
		field.Activated:Fire()
		assert(not box:IsFocused(), "disabled field cannot focus")
		box.Text = "oops"
		assert(box.Text == "hell" and #changes == 2, "disabled edits restore controlled text")
		h.stop()
	end,
	["a text field that a modal covers while editing gives up focus and takes no text"] = function()
		local h = harness()
		local field = h.instance("TextButton", "Base")
		local box = h.instance("TextBox", "Base")
		local changes, commits = 0, {}
		local binding = h.mount("Base", function()
			local created = h.require("use-text-field").useTextField({
				onValueChange = function()
					changes += 1
				end,
				onValueCommit = function(value)
					table.insert(commits, value)
				end,
			})
			created.control(field)
			created.box(box)
			return created
		end)
		field.Activated:Fire()
		h.flush()
		assert(box:IsFocused() and h.runtime.textEditing(), "a press focuses the box")
		box.Text = "draft"
		assert(binding.value() == "draft" and changes == 1, "typing while active changes the value")
		local closes = 0
		local id, root = h.root(10, "Native", function()
			closes += 1
		end)
		h.control(id, "Close", nil, true)
		h.flush()
		assert(not box:IsFocused() and not h.runtime.textEditing(), "the modal takes native focus from the field")
		assert(not binding.editing() and commits[1] == "draft", "losing focus ends editing and commits the draft")
		assert(box.TextEditable == false, "the covered box is not editable")
		box.Text = "typed behind the modal"
		assert(binding.value() == "draft" and changes == 1, "text behind the modal changes nothing")
		assert(box.Text == "draft", "and the box shows the value again")
		h.finish(h.begin(h.enum.KeyCode.ButtonB))
		assert(closes == 1, "the modal receives its close input")
		h.runtime.closeScope(id)
		root.Parent = nil
		h.flush()
		assert(box.TextEditable == true, "the field is editable once the modal closes")
		field.Activated:Fire()
		h.flush()
		assert(box:IsFocused(), "and focuses again on a press")
		h.stop()
	end,
	["text field defers autofocus and cancels it after disposal or disabling"] = function()
		local h = harness()
		local disabled = h.vide.source(false)
		local box = h.instance("TextBox", "Base")
		local _, dispose = h.mount("Base", function()
			local binding = h.require("use-text-field").useTextField({ autoFocus = true })
			binding.box(box)
		end)
		assert(not box:IsFocused(), "autofocus is deferred")
		dispose()
		h.flush()
		assert(not box:IsFocused(), "disposed field never captures focus")
		local _, remove = h.mount("Base", function()
			local binding = h.require("use-text-field").useTextField({
				autoFocus = true,
				disabled = disabled,
			})
			binding.box(box)
		end)
		disabled(true)
		h.flush()
		assert(not box:IsFocused(), "disabled field never captures deferred focus")
		remove()
		disabled(false)
		local _, release = h.mount("Base", function()
			local binding = h.require("use-text-field").useTextField({ autoFocus = true })
			binding.box(box)
		end)
		h.flush()
		assert(box:IsFocused(), "live enabled field captures focus")
		release()
		h.flush()
		assert(
			not box:IsFocused() and not h.require("focus-scope").useTextEditing()(),
			"disposal releases shared focus"
		)
		h.stop()
	end,
}
