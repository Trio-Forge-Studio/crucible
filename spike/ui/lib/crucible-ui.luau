-- The Luau a Crucible JSX lowering would emit, written by hand, plus the `Show` and keyed `For` it would call.
-- `variant` picks how a reactive prop is lowered:
--   "A": an initial read, then one general `effect` per prop.
--   "B": one `bind` node per prop (lib/binding.luau), on a patched trunk flush.
--   "H": one binding node per prop from the runtime's own `createBinding`, with one shared sink per property name,
--        as a compiler would emit it. The caller reads the initial value and writes it, then binds.
-- Fixed props are assigned at creation. Components are plain functions that run once in their caller's owner. The
-- root Instance of a mount, a `Show` branch or a `For` row is owned with `own`; Instances under it go with it.
local fake = require("./instance")
local Binding = require("./binding")

local Instance = fake.Instance
local FRAME_OWNING = 3
local ROLE_SCOPE = 3
local CALL_OK = 0
local SIZE = table.freeze({ 1, 0, 0, 24 })

return function(r, variant: string)
	local api = r.api
	local ownership = r.ownership
	local effect, own, scope = api.effect, api.own, api.scope
	local newLifetime, adopt, endLifetime, runBody, resolveOwner =
		ownership.newLifetime, ownership.adopt, ownership.endLifetime, ownership.runBody, ownership.resolveOwner

	local bind
	if variant == "H" then
		local createBinding = r.flushModule.createBinding
		local sinks = {}
		function bind(instance, property: string, source)
			local value = source()
			instance[property] = value
			local sink = sinks[property]
			if sink == nil then
				sink = function(target, next)
					target[property] = next
				end
				sinks[property] = sink
			end
			createBinding(source, value, sink, instance)
		end
	elseif variant == "A" then
		function bind(instance, property: string, read)
			instance[property] = read()
			effect(function()
				instance[property] = read()
			end)
		end
	else
		bind = Binding(r).bind
	end

	-- `scope(setup)` with an explicit parent, so a row made during the driver's run outlives that run.
	local function scopeIn(parent, setup: () -> ())
		local lifetime = newLifetime(ROLE_SCOPE, parent, parent.owner)
		adopt(parent, lifetime)
		local failure
		runBody({ kind = FRAME_OWNING, lifetime = lifetime }, setup, nil, function(completion)
			if completion.result ~= CALL_OK then
				failure = completion.value
				endLifetime(lifetime)
			end
		end)
		if failure ~= nil then
			error(failure, 0)
		end
		return lifetime
	end

	-- <Show when={when}>{render()}</Show>
	local function Show(when, render: () -> any, parent)
		local owner = resolveOwner()
		local branch
		effect(function()
			local on = when()
			if on and branch == nil then
				branch = scopeIn(owner, function()
					own(render()).Parent = parent
				end)
			elseif not on and branch ~= nil then
				endLifetime(branch)
				branch = nil
			end
		end)
	end

	-- <For each={each}>{(item, index) => render(item, index)}</For>, keyed by item identity. `index` is a state.
	local function For(each, render: (any, any) -> any, parent)
		local owner = resolveOwner()
		local rows = {}
		local pass = 0
		effect(function()
			local list = each()
			pass += 1
			for i, item in list do
				local row = rows[item]
				if row == nil then
					local index = api.state(i)
					row = { pass = pass, index = index }
					row.lifetime = scopeIn(owner, function()
						own(render(item, index)).Parent = parent
					end)
					rows[item] = row
				else
					row.pass = pass
					row.index:set(i)
				end
			end
			for item, row in rows do
				if row.pass ~= pass then
					endLifetime(row.lifetime)
					rows[item] = nil
				end
			end
		end)
	end

	-- function Item({ text, color }) {
	--   return <textlabel Name="Item" Size={SIZE} BackgroundTransparency={1} Text={text()} TextColor3={color()} />
	-- }
	local function Item(text, color)
		local e = Instance.new("TextLabel")
		e.Name = "Item"
		e.Size = SIZE
		e.BackgroundTransparency = 1
		bind(e, "Text", text)
		bind(e, "TextColor3", color)
		return e
	end

	-- function Row({ item, index }) {
	--   return <textlabel Name="Row" Size={SIZE} BackgroundTransparency={1} Text={item.label()} LayoutOrder={index()} />
	-- }
	local function Row(item, index)
		local e = Instance.new("TextLabel")
		e.Name = "Row"
		e.Size = SIZE
		e.BackgroundTransparency = 1
		bind(e, "Text", item.label)
		bind(e, "LayoutOrder", index)
		return e
	end

	local ui = {}

	-- mount(<frame Name="Root">{texts.map((t, i) => <Item text={t} color={colors[i]} />)}</frame>)
	function ui.mountItems(texts, colors)
		local root
		local dispose = scope(function()
			root = own(Instance.new("Frame"))
			root.Name = "Root"
			for i = 1, #texts do
				Item(texts[i], colors[i]).Parent = root
			end
		end)
		r.flush()
		return root, dispose
	end

	-- mount(<frame Name="List"><For each={list}>{(item, index) => <Row item={item} index={index} />}</For></frame>)
	function ui.mountList(list)
		local root
		local dispose = scope(function()
			root = own(Instance.new("Frame"))
			root.Name = "List"
			For(list, Row, root)
		end)
		r.flush()
		return root, dispose
	end

	-- mount(<frame Name="Panel"><Show when={open}><Item text={text} color={color} /></Show></frame>)
	function ui.mountShow(open, text, color)
		local root
		local dispose = scope(function()
			root = own(Instance.new("Frame"))
			root.Name = "Panel"
			Show(open, function()
				return Item(text, color)
			end, root)
		end)
		r.flush()
		return root, dispose
	end

	function ui.state(value)
		return api.state(value)
	end

	function ui.set(s, value)
		s:set(value)
	end

	ui.flush = r.flush
	return ui
end
