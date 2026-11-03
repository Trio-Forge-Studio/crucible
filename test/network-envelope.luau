local Link = require("./lib/network-link")
local h = require("./lib/network-codec")
local spec = require("./lib/load")

local CHIME = "00 00 00 00 00 00 10 40 00 00 00 00 00 00 14 40 00 00 00 00 00 00 18 40"

local function declare(r)
	local n = r.n
	return {
		First = r.emit.toServer("A.First", n["nil"]),
		Tree = r.emit.askServer("Admin.CommandTree", n["nil"], n.dynamic, nil),
		Rang = r.emit.toClient(
			"Doorbell.Rang",
			n.object({
				{ "button", n.union({ n["nil"], n.instance({ "BasePart" }) }) },
				{ "chime", n.vector3 },
				{ "houseId", n.union({ n["nil"], n.string }) },
			})
		),
		Toggle = r.emit.toServer("Doors.Toggle", n.object({ { "door", n.instance({ "Model" }) }, { "open", n.bool } })),
		Keys = r.emit.toClient("Keys.Map", n.map(n.union({ n.literal("a"), n.literal("b") }), n.u8)),
		Gone = r.emit.askServer("Z.Gone", n["nil"], n.dynamic, nil),
		Strict = r.emit.askServer("Z.Strict", n.u8, n.dynamic, nil),
		Words = r.emit.toServer("Words.List", n.array(n.string)),
	}
end

local function ix(definition)
	return string.format("%02x", definition.index)
end

-- The packets one realm fired, as hex, in order.
local function fired(link, from)
	local out = {}
	for _, e in
		link:sent(function(entry)
			return entry.from == from and not entry.injected
		end)
	do
		table.insert(out, h.hex(e.packet))
	end
	return out
end

return {
	["12 wire worked bytes from the real sessions carry each realm's own sealed or adopted index"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
			s.service("Admin", function(self)
				self.tree = s.emit.receive(self, "tree", m.Tree, function() end)
				self.strict = s.emit.receive(self, "strict", m.Strict, function() end)
				self.toggle = s.emit.receive(self, "toggle", m.Toggle, function() end)
			end)
		end)
		spec.eq(
			link.server.remote.attributes.Catalog,
			"A.First\nAdmin.CommandTree\nCrucible.State.Fault\nCrucible.State.Frame\nDoorbell.Rang\nDoors.Toggle\nKeys.Map\nWords.List\nZ.Gone\nZ.Strict",
			"sealed names in byte order"
		)
		local ann = link:join("Ann", function(c)
			c.m = declare(c)
			c.service("Sink", function(self)
				self.rang = c.emit.receive(self, "rang", c.m.Rang, function() end)
				self.keys = c.emit.receive(self, "keys", c.m.Keys, function() end)
			end)
		end)
		for name, definition in m do
			spec.eq(ann.m[name].index, definition.index, `{name} adopted at the server's index`)
		end
		local door = link:instance("Model", "Door")
		ann.Network.send(ann.m.Toggle, { door = door, open = true })
		local tree = link:run(function()
			return ann.Network.ask(ann.m.Tree)
		end)
		local gone = link:run(function()
			return ann.Network.ask(ann.m.Gone)
		end)
		link:flush()
		link:inject(ann, h.bytes(`01 {ix(m.Strict)} 03 00`), {})
		link:flush()
		local button = link:instance("Part", "Button")
		local chime = h.roblox.Vector3.new(4, 5, 6)
		link.server.Network.broadcast(m.Rang, { button = button, chime = chime, houseId = "H-012" })
		link.server.Network.broadcast(m.Rang, { chime = chime, houseId = "H-012" })
		link.server.Network.send(ann.player, m.Keys, { b = 2, a = 1 })
		link:flush()
		local client = ann.m
		spec.list(fired(link, "Ann"), {
			"04",
			`00 {ix(client.Toggle)} 01 01`,
			`01 {ix(client.Tree)} 01 00`,
			`01 {ix(client.Gone)} 02 00`,
		}, "READY, then EVENT with one slot, then REQUESTs with nonces 1 and 2, headers from the adopted catalog")
		spec.eq(
			link:sent(function(e)
				return e.from == "Ann" and e.opcode == 0
			end)[1].slots,
			1,
			"the door rides slot 1"
		)
		spec.list(fired(link, "Server"), {
			`02 {ix(m.Tree)} 01 00 00`,
			`03 {ix(m.Gone)} 02 00`,
			`03 {ix(m.Strict)} 03 03`,
			`00 {ix(m.Rang)} 01 01 {CHIME} 01 05 48 2d 30 31 32`,
			`00 {ix(m.Rang)} 00 00 {CHIME} 01 05 48 2d 30 31 32`,
			`00 {ix(m.Keys)} 00 02 00 01 01 02`,
		}, "OK with dynamic nil, ERROR unavailable and invalid, the doorbell rows and the literal-key map")
		spec.eq(tree.value.ok, true, "OK settles")
		spec.eq(gone.value.reason, "unavailable", "ERROR settles")
		link:assertClean()
	end,
	["16 19 an invalid payload or a wrong-direction token throws at the call site naming the message, the property path and the reason"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", function(c)
			c.m = declare(c)
		end)
		local before = #link:sent()
		local part = link:instance("Part", "NotAModel")
		for _, case in
			{
				{
					function()
						link.server.Network.send(ann.player, m.Rang, { chime = "loud" })
					end,
					"Doorbell.Rang payload.chime: expected datatype",
				},
				{
					function()
						link.server.Network.broadcast(m.Keys, { a = 300 })
					end,
					"Keys.Map payload[]: integer out of range",
				},
				{
					function()
						ann.Network.send(ann.m.Toggle, { door = part, open = true })
					end,
					"Doors.Toggle payload.door: expected declared Instance class",
				},
				{
					function()
						ann.Network.ask(ann.m.Strict, -1)
					end,
					"Z.Strict request: integer out of range",
				},
				{
					-- A 262,142-byte payload fits on its own; the 3-byte header takes the packet past MAX_PACKET_BYTES.
					function()
						local big = string.rep("w", 65536)
						ann.Network.send(ann.m.Words, { big, big, big, string.rep("w", 65521) })
					end,
					"Words.List payload: packet too large",
				},
				{
					function()
						link.server.Network.send(ann.player, m.Toggle, { door = part, open = true })
					end,
					"Network.send needs a message declared with toClient",
				},
				{
					function()
						link.server.Network.broadcast(m.Tree)
					end,
					"Network.broadcast needs a message declared with toClient",
				},
				{
					function()
						ann.Network.send(ann.m.Keys, {})
					end,
					"Network.send needs a message declared with toServer",
				},
				{
					function()
						ann.Network.ask(ann.m.Toggle, { door = part, open = true })
					end,
					"Network.ask needs a message declared with askServer",
				},
			}
		do
			local call = link:run(function()
				return table.pack(pcall(case[1]))
			end)
			spec.eq(call.done, true, "a rejected call never yields")
			spec.eq(call.value[1], false, "throws")
			spec.eq(call.value[2], case[2], "message, path and reason")
		end
		spec.eq(#link:sent(), before, "nothing was sent or queued")
		link:assertClean()
	end,
	["50 77 FAIL overflow is the two bytes 05 00"] = function()
		local link = Link.new()
		local m
		link:boot(function(s)
			m = declare(s)
		end)
		local ann = link:join("Ann", nil, { boot = false })
		for _ = 1, 257 do
			link.server.Network.send(ann.player, m.Keys, {})
		end
		spec.list(fired(link, "Server"), { "05 00" }, "FAIL overflow")
		link:assertClean()
	end,
}
