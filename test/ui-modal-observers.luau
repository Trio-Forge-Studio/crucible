-- The ui entry and a Crucible runtime share one strict Charm, so a Crucible effect observes modalOpen as a game's would.
local fs = require("@lune/fs")
local luau = require("@lune/luau")
local Reactive = require("./lib/reactive-world")
local harness = require("./lib/ui-harness")

-- The game installs vide-charm, so the sample's copy is the one a game reads modalOpen through.
local VIDE_CHARM = fs.readFile("examples/sample-game/node_modules/@rbxts/vide-charm/src/init.luau")

local function setup()
	local r = Reactive(nil, nil, true)
	local h = harness({ charm = r.charm })
	h.require("focus-scope")
	h.require("dialog")
	h.require()
	return r, h
end

-- A Crucible effect that throws whenever modalOpen() equals `failOn`. Returns the effect's scope dispose.
local function observe(r, h, failOn: boolean)
	local modalOpen = h.runtime.modalOpen
	return r.root(function()
		r.api.effect(function()
			if modalOpen() == failOn then
				error(`observer failed on modalOpen {failOn}`)
			end
		end)
	end)
end

local function failed(failOn: boolean, ok: boolean, err: any)
	assert(not ok, "the observer's error reaches the code that opened or closed the scope")
	assert(
		string.find(tostring(err), `observer failed on modalOpen {failOn}`, 1, true),
		`the reported error is the observer's: {err}`
	)
end

return {
	["a modalOpen observer that throws on open leaves no scope behind a dialog that failed to open"] = function()
		local r, h = setup()
		local Dialog = h.require("dialog").Dialog
		local hud = h.control("Base", "Hud")
		local stop = observe(r, h, true)
		local open = h.vide.source(false)
		h.mount("Base", function()
			return Dialog.Root({
				open = open,
				children = function()
					return Dialog.Content({
						children = function()
							return h.vide.jsx("frame", { Name = "Panel" })
						end,
					})
				end,
			})
		end)
		failed(true, pcall(open, true))
		assert(not h.runtime.modalOpen(), "modalOpen() is false after the failed open")
		assert(h.runtime.active("Base") and hud.Selectable, "Base is active again")
		stop()
		open(false)
		open(true)
		h.flush()
		assert(h.runtime.modalOpen() and not h.runtime.active("Base"), "the next open still traps")
		open(false)
		assert(not h.runtime.modalOpen() and h.runtime.active("Base"), "and its close releases Base")
		h.stop()
	end,
	["a modalOpen observer that throws on open closes the scope when its owner unmounts"] = function()
		local r, h = setup()
		local useFocusScope = h.require("focus-scope").useFocusScope
		local hud = h.control("Base", "Hud")
		local stop = observe(r, h, true)
		local trigger = h.vide.source(false)
		local _, unmount = h.mount("Base", function()
			-- A plain effect keeps a failed run's cleanups until it reruns or its owner ends.
			h.vide.effect(function()
				if trigger() then
					h.vide.untrack(function()
						useFocusScope({ trapped = true })
					end)
				end
			end)
		end)
		failed(true, pcall(trigger, true))
		assert(h.runtime.modalOpen() and not h.runtime.active("Base"), "the open scope lasts while its owner lives")
		unmount()
		assert(not h.runtime.modalOpen(), "modalOpen() is false once the owner unmounts")
		assert(h.runtime.active("Base") and hud.Selectable, "Base is active again")
		stop()
		h.stop()
	end,
	["a modalOpen observer that throws on close still restores focus"] = function()
		local r, h = setup()
		local useFocusScope = h.require("focus-scope").useFocusScope
		local hud = h.control("Base", "Hud")
		h.select(hud)
		local _, unmount = h.mount("Base", function()
			useFocusScope({ trapped = true })
		end)
		h.flush()
		assert(h.runtime.modalOpen() and h.gui.SelectedObject == nil, "the trap takes selection off the HUD")
		local stop = observe(r, h, false)
		failed(false, pcall(unmount))
		h.flush()
		assert(not h.runtime.modalOpen() and h.runtime.active("Base"), "the scope is gone")
		assert(h.gui.SelectedObject == hud, "the close still restores the HUD selection")
		stop()
		h.stop()
	end,
	["modalOpen ignores an argument, so its export cannot write the modal atom"] = function()
		local _, h = setup()
		local modalOpen = h.require().modalOpen
		-- TypeScript accepts `const w: Atom<boolean> = modalOpen; w(false)`, so the export must drop the argument.
		local write: any = modalOpen
		local hud = h.control("Base", "Hud")
		write(true)
		assert(not modalOpen() and h.runtime.active("Base"), "a write with no scope opens nothing")
		local id = h.root(10)
		write(false)
		assert(modalOpen(), "a write while a scope is open leaves modalOpen() true")
		assert(not h.runtime.active("Base") and h.runtime.active(id), "and leaves the Vide state trapped")
		h.runtime.closeScope(id)
		assert(not modalOpen() and hud.Selectable, "closing the scope still releases it")
		h.stop()
	end,
	["modalOpen tracks in a Charm computed, a Crucible effect and vide-charm's useAtom"] = function()
		local r, h = setup()
		local VideCharm = luau.load(VIDE_CHARM, {
			debugName = "vide-charm",
			environment = {
				require = function(name: string)
					return if name == "./Charm" then r.charm else h.vide
				end,
			},
		})()
		local modalOpen = h.require().modalOpen
		local closed = r.charm.computed(function()
			return not modalOpen()
		end)
		local seen = {}
		local stop = r.root(function()
			r.api.effect(function()
				table.insert(seen, tostring(closed()))
			end)
		end)
		local state = h.mount("Base", function()
			return VideCharm.useAtom(modalOpen)
		end)
		local id = h.root(10)
		assert(state() == true, "useAtom follows the open")
		h.runtime.closeScope(id)
		assert(state() == false, "useAtom follows the close")
		assert(table.concat(seen, " ") == "true false true", `the effect saw each change: {table.concat(seen, " ")}`)
		stop()
		h.stop()
	end,
}
