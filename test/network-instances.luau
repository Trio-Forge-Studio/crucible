local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")

local function declare(r)
	local n = r.n
	return {
		Toggle = r.emit.toServer("Doors.Toggle", n.object({ { "door", n.instance({ "Model" }) }, { "open", n.bool } })),
		Open = r.emit.askServer("Hatches.Open", n.instance({ "Part" }), n.bool, nil),
		Rang = r.emit.toClient(
			"Doorbell.Rang",
			n.object({
				{ "button", n.union({ n["nil"], n.instance({ "Part" }) }) },
				{ "chime", n.vector3 },
				{ "houseId", n.union({ n["nil"], n.string }) },
			})
		),
		LevelUp = r.emit.toClient("Progression.LevelUp", n.object({ { "Player", n.instance({ "Player" }) } })),
		Whose = r.emit.askServer("Players.Whose", n["nil"], n.instance({ "Player" }), nil),
	}
end

local function hex(n)
	return string.format("%02x", n)
end

local function world(link)
	local got = { toggles = {}, opens = {} }
	local m
	link:boot(function(s)
		m = declare(s)
		s.service("Doors", function(self)
			self.toggle = s.emit.receive(self, "toggle", m.Toggle, function(player, value)
				table.insert(got.toggles, `{player.Name} {value.door.Name} {value.open}`)
			end)
			self.open = s.emit.receive(self, "open", m.Open, function(_, part)
				table.insert(got.opens, part.Name)
				return true
			end)
			self.whose = s.emit.receive(self, "whose", m.Whose, function()
				return got.whose
			end)
		end)
	end)
	assert(link.server.booted, tostring(link.server.bootError))
	return m, got
end

local function client(c)
	c.m = declare(c)
	c.rang, c.levels = {}, {}
	c.service("Bell", function(self)
		self.rang = c.emit.receive(self, "rang", c.m.Rang, function(value)
			table.insert(c.rang, `{value.button and value.button.Name or "nil"} {value.chime.X} {value.houseId}`)
		end)
		self.level = c.emit.receive(self, "level", c.m.LevelUp, function(value)
			table.insert(c.levels, value.Player.Name)
		end)
	end)
end

return {
	["67 74 a client event missing a required Instance drops silently and keeps the player's diagnostic"] = function()
		local link = Link.new()
		local m, got = world(link)
		local ann = link:join("Ann", client)
		local local_ = link:instance("Model", "ClientModel")
		link:only(ann, local_)
		ann.Network.send(ann.m.Toggle, { door = local_, open = true })
		local gone = link:instance("Model", "Destroyed")
		ann.Network.send(ann.m.Toggle, { door = gone, open = false })
		link:destroy(gone)
		link:flush()
		local door = link:instance("Model", "Door")
		ann.Network.send(ann.m.Toggle, { door = door, open = true })
		link:flush()
		spec.list(got.toggles, { "Ann Door true" }, "missing Instances dropped, siblings unaffected")
		spec.eq(#link.server.diagnostics(), 0, "a missing Instance is a race, not a rejection")
		link:inject(ann, h.bytes(`00 {hex(m.Toggle.index)} 01 02`), { ["1"] = door })
		link:flush()
		spec.eq(
			link.server.logged("Crucible dropped Doors.Toggle from Ann: payload.open: invalid boolean"),
			1,
			"diagnosed"
		)
		link:inject(ann, h.bytes(`00 {hex(m.Toggle.index)} 01 02`), { ["1"] = door })
		link:flush()
		spec.eq(#link.server.diagnostics(), 1, "once per player per message")
		spec.eq(#link.kicks, 0, "a bad client payload never kicks")
		link:assertClean()
	end,
	["41 67 a request missing a required Instance replies invalid with no diagnostic"] = function()
		local link = Link.new()
		local m, got = world(link)
		local ann = link:join("Ann", client)
		local hatch = link:instance("Part", "Hatch")
		link:only(ann, hatch)
		local asked = link:run(function()
			return ann.Network.ask(ann.m.Open, hatch)
		end)
		link:flush()
		spec.eq(asked.value.reason, "invalid", "ERROR invalid")
		spec.eq(#got.opens, 0, "no callback")
		spec.eq(#link.server.diagnostics(), 0, "not diagnosed")
		local real = link:instance("Part", "Real")
		local ok = link:run(function()
			return ann.Network.ask(ann.m.Open, real)
		end)
		link:flush()
		spec.eq(ok.value.value, true, "a visible Instance passes")
		link:inject(ann, h.bytes(`01 {hex(m.Open.index)} 05 00`), {})
		link:flush()
		spec.eq(
			link.server.logged("Crucible dropped Hatches.Open from Ann: request: slot count exceeded"),
			1,
			"a real decode failure is"
		)
		link:assertClean()
	end,
	["67 a missing optional Instance arrives undefined with its siblings intact"] = function()
		local link = Link.new()
		local m = world(link)
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		local button = link:instance("Part", "Button")
		link:hide(bea, button)
		link.server.Network.broadcast(
			m.Rang,
			{ button = button, chime = h.roblox.Vector3.new(4, 5, 6), houseId = "H-012" }
		)
		link:flush()
		spec.list(ann.rang, { "Button 4 H-012" }, "visible")
		spec.list(bea.rang, { "nil 4 H-012" }, "streamed out: undefined, siblings intact")
		spec.eq(#bea.diagnostics(), 0, "not a failure")
		link:assertClean()
	end,
	["68 43 a client drops an event and settles a reply invalid when a required Player arrived nil"] = function()
		local link = Link.new()
		local m, got = world(link)
		local ann = link:join("Ann", client)
		local bea = link:join("Bea", client)
		link:hide(ann, bea.player)
		link.server.Network.send(ann.player, m.LevelUp, { Player = bea.player })
		link.server.Network.send(ann.player, m.LevelUp, { Player = bea.player })
		link.server.Network.send(ann.player, m.LevelUp, { Player = ann.player })
		link:flush()
		spec.list(ann.levels, { "Ann" }, "dropped, and the session continues")
		spec.eq(ann.logged("Crucible dropped Progression.LevelUp: a Player left"), 1, "once per message per session")
		got.whose = bea.player
		local first = link:run(function()
			return ann.Network.ask(ann.m.Whose)
		end)
		link:flush()
		local second = link:run(function()
			return ann.Network.ask(ann.m.Whose)
		end)
		link:flush()
		spec.eq(first.value.reason, "invalid", "a reply that does not decode settles invalid")
		spec.eq(second.value.reason, "invalid", "again")
		spec.eq(ann.logged("Crucible could not decode a reply to Players.Whose"), 1, "one diagnostic per message")
		spec.eq(#link.kicks, 0, "neither fails the session")
		got.whose = ann.player
		local third = link:run(function()
			return ann.Network.ask(ann.m.Whose)
		end)
		link:flush()
		spec.eq(third.value.value, ann.player, "the session still answers")
		link:assertClean()
	end,
}
