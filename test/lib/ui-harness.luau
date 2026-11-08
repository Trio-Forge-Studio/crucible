local fs = require("@lune/fs")
local luau = require("@lune/luau")
local load = require("./ui-load")
-- The engine adapter accepts Roblox instances and signals; these doubles are dynamic.
local roblox: any = require("@lune/roblox")
type Mock = any
-- Vide's reactive core runs as is in Lune, from the copy the runtime builds against. Each harness gets a fresh copy so
-- specs never share a graph.
local function loadVide(): Mock
	local modules: { [string]: Mock } = {}
	local function loadModule(name: string): Mock
		if modules[name] == nil then
			local path = `packages/crucible/node_modules/@rbxts/vide/src/{name}.luau`
			modules[name] = luau.load(fs.readFile(path), {
				debugName = path,
				environment = {
					require = function(target: string): Mock
						if target == "../test/mock" then
							return { typeof = typeof }
						end
						return loadModule((string.match(target, "^%./(.+)$")))
					end,
				},
			})()
		end
		return modules[name]
	end
	-- Studio runs unoptimized Luau, where Vide's strict mode reruns every effect; specs match it.
	loadModule("flags").strict = true
	local vide: Mock = {}
	for _, name in { "source", "derive", "effect", "cleanup", "untrack", "read", "context", "root", "batch", "show" } do
		vide[name] = loadModule(name)
	end
	vide.For = loadModule("components").For
	return vide
end
local function signal(): Mock
	local listeners: { [(...Mock) -> ()]: boolean } = {}
	return {
		Connect = function(_self, run: (...Mock) -> ())
			listeners[run] = true
			return {
				Disconnect = function()
					listeners[run] = nil
				end,
			}
		end,
		Fire = function(_self, ...)
			for run in table.clone(listeners) do
				run(...)
			end
		end,
	}
end
local guiObjects = { "Frame", "TextButton", "TextBox", "TextLabel", "ImageLabel", "ImageButton", "CanvasGroup" }
local buttons = { "TextButton", "ImageButton" }
local classes = {
	frame = "Frame",
	canvasgroup = "CanvasGroup",
	textbutton = "TextButton",
	imagebutton = "ImageButton",
	textbox = "TextBox",
	uiscale = "UIScale",
	uisizeconstraint = "UISizeConstraint",
	sound = "Sound",
	textlabel = "TextLabel",
	imagelabel = "ImageLabel",
	screengui = "ScreenGui",
	uicorner = "UICorner",
	uigradient = "UIGradient",
	uilistlayout = "UIListLayout",
	uipadding = "UIPadding",
	uistroke = "UIStroke",
}
local function harness(options: { deferredText: boolean }?): Mock
	local deferred: { () -> () } = {}
	local objects: { Mock } = {}
	local focused: Mock = nil
	local input: Mock
	local vide = loadVide()
	local function scoped(run: () -> Mock): (Mock, () -> ())
		local destroy, result = vide.root(run)
		local disposed = false
		return result, function()
			if disposed then
				return
			end
			disposed = true
			destroy()
		end
	end
	local function descendant(object: Mock, ancestor: Mock): boolean
		local parent = object.Parent
		while parent do
			if parent == ancestor then
				return true
			end
			parent = parent.Parent
		end
		return false
	end
	local function instance(class: string, parent: Mock?): Mock
		local properties: Mock = {
			Name = class,
			ClassName = class,
			Parent = parent,
			Visible = true,
			Enabled = true,
			Selectable = false,
			Text = "",
			TextEditable = true,
			ClipsDescendants = false,
			Active = false,
			BackgroundTransparency = 0,
			AbsoluteSize = roblox.Vector2.new(100, 60),
			AbsolutePosition = roblox.Vector2.zero,
		}
		local signals: Mock = {}
		local methods: Mock = {}
		local attributes: Mock = {}
		function methods.SetAttribute(_self, name, value)
			attributes[name] = value
		end
		function methods.GetAttribute(_self, name)
			return attributes[name]
		end
		function methods.GetPropertyChangedSignal(_self, name)
			if not signals[name] then
				signals[name] = signal()
			end
			return signals[name]
		end
		function methods.IsA(_self, name)
			return name == class
				or (name == "GuiObject" and table.find(guiObjects, class) ~= nil)
				or (name == "GuiButton" and table.find(buttons, class) ~= nil)
				or (name == "UIComponent" and string.sub(class, 1, 2) == "UI")
				or (name == "LayerCollector" and class == "ScreenGui")
		end
		function methods.FindFirstAncestorOfClass(self: Mock, name: string): Mock
			local at = self.Parent
			while at do
				if at:IsA(name) then
					return at
				end
				at = at.Parent
			end
			return nil
		end
		methods.FindFirstAncestorWhichIsA = methods.FindFirstAncestorOfClass
		function methods.IsDescendantOf(self: Mock, ancestor: Mock): boolean
			return descendant(self, ancestor)
		end
		function methods.FindFirstChild(self: Mock, name: string): Mock
			for _, child in objects do
				if child.Parent == self and child.Name == name then
					return child
				end
			end
			return nil
		end
		for _, name in
			{ "AncestryChanged", "InputBegan", "Activated", "MouseEnter", "MouseLeave", "Focused", "FocusLost" }
		do
			properties[name] = signal()
		end
		function methods.Destroy(self)
			self.Parent = nil
		end
		function methods.IsFocused(self)
			return focused == self
		end
		function methods.ReleaseFocus(self, enterPressed)
			if focused ~= self then
				return
			end
			focused = nil
			self.FocusLost:Fire(enterPressed or false)
			input.TextBoxFocusReleased:Fire(self)
		end
		function methods.CaptureFocus(self)
			if focused == self then
				return
			end
			if focused then
				focused:ReleaseFocus()
			end
			focused = self
			self.Focused:Fire()
			input.TextBoxFocused:Fire(self)
		end
		-- Userdata like a real Instance, so Vide sources skip writes of the same object.
		local object: Mock = newproxy(true)
		local meta = getmetatable(object)
		meta.__index = function(_self, name)
			return methods[name] or properties[name]
		end
		meta.__newindex = function(self, name, value)
			if properties[name] == value then
				return
			end
			properties[name] = value
			if signals[name] then
				if name == "Text" and options and options.deferredText then
					table.insert(deferred, function()
						signals[name]:Fire()
					end)
				else
					signals[name]:Fire()
				end
			end
			if name == "Parent" then
				for _, child in objects do
					if child == self or descendant(child, self) then
						child.AncestryChanged:Fire()
					end
				end
			end
		end
		table.insert(objects, object)
		return object
	end
	local gui = instance("GuiService")
	gui.MenuIsOpen = false
	gui.PreferredTransparency = 1
	gui.TopbarInset = { Height = 58 }
	gui.GetGuiInset = function()
		return roblox.Vector2.new(0, 58), roblox.Vector2.zero
	end
	gui.IsTenFootInterface = function()
		return false
	end
	local playerGui = instance("PlayerGui")
	local function shown(object: Mock): boolean
		local at = object
		while at do
			if at.Visible == false or (at:IsA("LayerCollector") and not at.Enabled) then
				return false
			end
			at = at.Parent
		end
		return true
	end
	local function depth(object: Mock): number
		local count, at = 0, object.Parent
		while at do
			count += 1
			at = at.Parent
		end
		return count
	end
	-- Orders hits topmost first by DisplayOrder, then descendants above ancestors, then later siblings.
	function playerGui.GetGuiObjectsAtPosition(_self, x: number, y: number): { Mock }
		local hits = {}
		for index, object in objects do
			if not object:IsA("GuiObject") or not object:IsDescendantOf(playerGui) or not shown(object) then
				continue
			end
			local position, size = object.AbsolutePosition, object.AbsoluteSize
			if x >= position.X and y >= position.Y and x <= position.X + size.X and y <= position.Y + size.Y then
				local layer = object:FindFirstAncestorOfClass("ScreenGui")
				table.insert(hits, {
					object = object,
					order = if layer then layer.DisplayOrder else 0,
					depth = depth(object),
					index = index,
				})
			end
		end
		table.sort(hits, function(a, b)
			if a.order ~= b.order then
				return a.order > b.order
			elseif a.depth ~= b.depth then
				return a.depth > b.depth
			end
			return a.index > b.index
		end)
		local found = {}
		for _, hit in hits do
			table.insert(found, hit.object)
		end
		return found
	end
	local screens = {}
	local function screen(order: number): Mock
		if not screens[order] then
			screens[order] = instance("ScreenGui", playerGui)
			screens[order].DisplayOrder = order
		end
		return screens[order]
	end
	vide.jsx = function(tag, given, ...)
		local props = given or {}
		if type(tag) == "function" then
			local count = select("#", ...)
			if count > 1 then
				props.children = { ... }
			elseif count == 1 then
				props.children = ...
			end
			return vide.untrack(function()
				return tag(props)
			end)
		end
		local class = classes[tag]
		assert(class, "harness renders " .. tag)
		local object = instance(class, if class == "ScreenGui" then playerGui else screen(0))
		for name, value in props do
			if name == "action" then
				continue
			end
			if type(value) == "function" then
				vide.effect(function()
					object[name] = value()
				end)
			else
				object[name] = value
			end
		end
		local function attach(child)
			if type(child) == "userdata" then
				child.Parent = object
			elseif type(child) == "table" then
				for _, nested in child do
					attach(nested)
				end
			elseif type(child) == "function" then
				-- Roblox defers AncestryChanged, so reparenting a reactive child never re-enters Vide's graph.
				vide.effect(function()
					local current = child()
					table.insert(deferred, function()
						attach(current)
					end)
				end)
			end
		end
		for _, child in { ... } do
			attach(child)
		end
		if props.action then
			props.action(object)
		end
		return object
	end
	function gui.Select(_self, root)
		for _, object in objects do
			if object.Selectable and descendant(object, root) then
				gui.SelectedObject = object
				return
			end
		end
	end
	local b = roblox.Enum.KeyCode.ButtonB
	local gamepad = roblox.Enum.UserInputType.Gamepad1
	local held: Mock = {}
	input = {
		InputBegan = signal(),
		InputEnded = signal(),
		InputChanged = signal(),
		WindowFocusReleased = signal(),
		WindowFocused = signal(),
		GamepadDisconnected = signal(),
		TouchPinch = signal(),
		TextBoxFocused = signal(),
		TextBoxFocusReleased = signal(),
		GetConnectedGamepads = function()
			return { gamepad }
		end,
		GetGamepadState = function()
			return {}
		end,
		IsGamepadButtonDown = function(_self, _pad, key)
			return held[key] == true
		end,
		GetFocusedTextBox = function()
			return focused
		end,
	}
	local close = { Pressed = signal(), Released = signal() }
	local heartbeat = signal()
	local clock = 0
	local device = vide.source("Gamepad")
	local contexts = {}
	local feedback: { string } = {}
	local warnings: { string } = {}
	local text = {
		width = function(params): number
			return #params.Text * 8
		end,
	}
	local require = load({
		packages = { ["@rbxts/vide"] = vide },
		services = {
			GuiService = gui,
			UserInputService = input,
			TextService = {
				GetTextBoundsAsync = function(_self, params)
					return roblox.Vector2.new(text.width(params), params.Size)
				end,
			},
			RunService = {
				Heartbeat = heartbeat,
				PreRender = signal(),
				IsStudio = function()
					return true
				end,
			},
		},
		globals = {
			Enum = roblox.Enum,
			UDim = roblox.UDim,
			UDim2 = roblox.UDim2,
			Vector2 = roblox.Vector2,
			Color3 = roblox.Color3,
			ColorSequence = roblox.ColorSequence,
			Font = roblox.Font,
			Instance = { new = instance },
			os = setmetatable({
				clock = function()
					return clock
				end,
			}, { __index = os }),
			task = {
				defer = function(run)
					table.insert(deferred, run)
				end,
			},
			warn = function(message)
				table.insert(warnings, message)
			end,
		},
	})
	require("adapter").installAnvil({
		device = device,
		close = {
			held = function()
				return held[b] == true
			end,
			onPressed = function(listener)
				local connection = close.Pressed:Connect(listener)
				return function()
					connection:Disconnect()
				end
			end,
			onReleased = function(listener)
				local physical = input.InputEnded:Connect(function(event)
					if event.KeyCode == b then
						listener()
					end
				end)
				local action = close.Released:Connect(listener)
				return function()
					physical:Disconnect()
					action:Disconnect()
				end
			end,
		},
		claimMenuInput = function()
			contexts.Menu = true
			return function()
				contexts.Menu = false
			end
		end,
		feedback = function(kind)
			table.insert(feedback, kind)
		end,
	})
	local runtime = require("runtime")
	local Press = require("use-press")
	local stop = runtime.startMenuRuntime()
	local roots = {}
	local disposals: { () -> () } = {}
	local h: Mock = {
		require = require,
		runtime = runtime,
		gui = gui,
		input = input,
		close = close,
		device = device,
		contexts = contexts,
		feedback = feedback,
		warnings = warnings,
		text = text,
		enum = roblox.Enum,
		vide = vide,
		screen = screen,
	}
	function h.mount(owner, run)
		local result, dispose = scoped(function()
			return runtime.ownerContext(owner, run)
		end)
		table.insert(disposals, dispose)
		return result, dispose
	end
	local function parent(owner)
		return if owner == "Base" then screen(0) elseif owner == "Topbar" then screen(12) else roots[owner]
	end
	function h.instance(class, owner)
		return instance(class, parent(owner))
	end
	function h.flush()
		local count = 0
		while #deferred > 0 do
			local run = table.remove(deferred, 1)
			assert(run, "queued readiness callback exists")
			run()
			count += 1
			assert(count < 100, "readiness must not poll or loop")
		end
	end
	function h.root(order, mode, onClose, pending)
		local id = runtime.openScope(mode or "Native", onClose)
		local root = instance("Frame", if pending then nil else screen(order))
		roots[id] = root
		local detach: () -> () = runtime.attachScope(id, root)
		table.insert(disposals, detach)
		return id, root
	end
	function h.attach(root, order)
		root.Parent = screen(order)
	end
	function h.control(owner, name, given, fallback, press)
		local object = instance("TextButton", parent(owner))
		object.Name = name
		local binding, dispose = scoped(function()
			return runtime.ownerContext(owner, function()
				return runtime.closeFallbackContext(fallback or false, function()
					local result = if press then Press.usePress(given) else runtime.useSelection(given or {})
					result.action(object)
					return result
				end)
			end)
		end)
		table.insert(disposals, dispose)
		return object, dispose, binding
	end
	function h.press(owner, name, given)
		return h.control(owner, name, given, false, true)
	end
	function h.select(object)
		gui.SelectedObject = object
	end
	function h.begin(key)
		held[key] = true
		local object = { KeyCode = key, UserInputType = gamepad }
		input.InputBegan:Fire(object)
		if key == b then
			close.Pressed:Fire()
		end
		return object
	end
	function h.finish(object)
		held[object.KeyCode] = false
		input.InputEnded:Fire(object)
		if object.KeyCode == b then
			close.Released:Fire()
		end
	end
	function h.holdB(down)
		held[b] = down
	end
	function h.editing(value)
		focused = if value then {} else nil
		if value then
			input.TextBoxFocused:Fire(focused)
		else
			input.TextBoxFocusReleased:Fire()
		end
		h.flush()
	end
	-- The clock only moves with frames, so timers advance exactly by the deltas a spec ticks.
	function h.tick(delta)
		clock += delta
		heartbeat:Fire(delta)
	end
	function h.stop()
		for _, dispose in disposals do
			dispose()
		end
		for id in roots do
			runtime.closeScope(id)
		end
		stop()
	end
	return h
end
return harness
