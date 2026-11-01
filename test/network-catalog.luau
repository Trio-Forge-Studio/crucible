local spec = require("./lib/load")
local h = require("./lib/network-codec")

return {
	["9 10 catalog seals in byte order and rejects late declarations"] = function()
		local r = h.realm()
		local b = r.catalog.define({ name = "Z.Last", kind = "to-server", input = r.n["nil"] })
		local a = r.catalog.define({ name = "A.First", kind = "to-client", input = r.n.bool })
		spec.eq(r.catalog.seal(), "A.First\nZ.Last", "published Catalog")
		spec.eq(a.index, 0, "first index")
		spec.eq(b.index, 1, "second index")
		spec.eq(r.catalog.definitionAt(0), a, "first lookup")
		spec.eq(r.catalog.definitionAt(1), b, "last lookup")
		h.rejects(function()
			r.catalog.define({ name = "Doors.Toggle", kind = "to-server", input = r.n["nil"] })
		end, "Doors.Toggle was declared after the network catalog sealed; declare messages in modules that boot loads.")
		h.rejects(function()
			r.catalog.seal()
		end, "catalog already sealed")
	end,
	["11 13 catalog count grammar and ascending order are bounded"] = function()
		for _, bad in { "A\nA", "Z\nA", "A\n", "Crucible.Test", "1A", ".A", "A..B", string.rep("a", 65) } do
			h.rejects(function()
				h.realm().catalog.adopt(bad)
			end)
		end
		local names = {}
		for i = 1, 1024 do
			table.insert(names, "M" .. string.format("%063d", i))
		end
		local largest = table.concat(names, "\n")
		spec.eq(#largest, 66559, "1,024 names of 64 bytes")
		local adopted = h.realm()
		adopted.catalog.adopt(largest)
		spec.eq(adopted.catalog.catalogSize(), 1024, "largest catalog adopts")
		h.rejects(function()
			h.realm().catalog.adopt(largest .. "\nN")
		end)
		local short = {}
		for i = 1, 1025 do
			table.insert(short, string.format("A%04d", i))
		end
		h.rejects(function()
			h.realm().catalog.adopt(table.concat(short, "\n"))
		end, "too many messages")
		local r = h.realm()
		for i = 1, 1024 do
			r.catalog.define({ name = `M.N{i}`, kind = "to-server", input = r.n["nil"] })
		end
		r.catalog.seal()
		spec.eq(r.catalog.catalogSize(), 1024, "MAX_MESSAGES seals")
		r = h.realm()
		for i = 1, 1025 do
			r.catalog.define({ name = `M.N{i}`, kind = "to-server", input = r.n["nil"] })
		end
		h.rejects(function()
			r.catalog.seal()
		end, "too many messages")
	end,
	["13 14 15 two realms adopt published indices and leave absent names unpublished"] = function()
		local server, client = h.realm(), h.realm()
		server.catalog.define({ name = "Z", kind = "to-server", input = server.n["nil"] })
		server.catalog.define({ name = "A", kind = "to-server", input = server.n["nil"] })
		client.catalog.adopt(server.catalog.seal())
		local z = client.catalog.define({ name = "Z", kind = "to-server", input = client.n["nil"] })
		local extra = client.catalog.define({ name = "ClientOnly", kind = "to-server", input = client.n["nil"] })
		spec.eq(z.index, server.catalog.definitionAt(1).index, "adopted index")
		spec.eq(extra.index, nil, "unpublished")
		spec.eq(#client.catalog.unpublished(), 1, "only the absent name is unpublished")
		spec.eq(client.catalog.unpublished()[1], extra, "report")
		for _, index in { -1, 0.5, 2, math.huge, 0 / 0 } do
			spec.eq(client.catalog.definitionAt(index), nil, "untrusted index")
		end
		client.catalog.discardIndices()
		spec.eq(z.index, nil, "discard resets index")
		spec.eq(client.catalog.catalogSize(), 0, "discard resets size")
		client.catalog.adopt("Z")
		spec.eq(z.index, 0, "cached declaration adopts again")
	end,
	["3 4 5 runtime declaration validation and token identity"] = function()
		local r = h.realm()
		local token = r.catalog.define({ name = "A", kind = "ask-server", input = r.n["nil"], output = r.n.dynamic })
		spec.eq(token.timeout, 10, "default timeout")
		spec.eq(r.catalog.definitionOf(token), token, "identity")
		spec.eq(r.catalog.definitionOf({ name = "A" }), nil, "forged token")
		h.rejects(function()
			r.catalog.define({ name = "A", kind = "to-server", input = r.n["nil"] })
		end, "duplicate message A")
		for _, bad in { "", "Crucible.Test", "Crucible", "1A", ".A", "A.", "A..B", "A-B", "A B", string.rep("a", 65) } do
			h.rejects(function()
				r.catalog.define({ name = bad, kind = "to-server", input = r.n["nil"] })
			end)
		end
		r.catalog.define({ name = string.rep("a", 64), kind = "to-server", input = r.n["nil"] })
		r.catalog.define({ name = "CrucibleGame.A_1", kind = "to-server", input = r.n["nil"] })
		h.rejects(function()
			r.catalog.define({ name = "Timed", kind = "to-server", input = r.n["nil"], timeout = 5 })
		end, "invalid message options")
		h.rejects(function()
			r.catalog.define({ name = "Bare", kind = "ask-server", input = r.n["nil"] })
		end, "invalid message options")
		local slow = r.catalog.define({
			name = "Slow",
			kind = "ask-server",
			input = r.n["nil"],
			output = r.n["nil"],
			timeout = 120,
		})
		spec.eq(slow.timeout, 120, "MAX_ASK_TIMEOUT_SECONDS admitted")
		local half = {}
		for i = 1, 2047 do
			table.insert(half, { string.format("F%04d", i), r.n.bool })
		end
		local request = r.n.object(half)
		r.catalog.define({ name = "Fits", kind = "ask-server", input = request, output = request })
		local response = r.n.object({ { "a", request }, { "b", r.n.bool } })
		h.rejects(function()
			r.catalog.define({ name = "Over", kind = "ask-server", input = request, output = response })
		end, "schema budget exhausted")
		for _, timeout in { 0, -1, 121, math.huge, 0 / 0 } do
			h.rejects(function()
				r.catalog.define({
					name = "Bad",
					kind = "ask-server",
					input = r.n["nil"],
					output = r.n["nil"],
					timeout = timeout,
				})
			end)
		end
	end,
	["limits parity table is the contract value of every constant"] = function()
		local actual = h.realm().limits
		local expected = {
			PROTOCOL_VERSION = 1,
			REMOTE_NAME = "CrucibleNetwork",
			PROTOCOL_ATTRIBUTE = "Protocol",
			CATALOG_ATTRIBUTE = "Catalog",
			KICK_MESSAGE = "Connection problem, please rejoin.",
			MESSAGE_NAME_MAX_BYTES = 64,
			MAX_MESSAGES = 1024,
			CLIENT_DISCOVERY_TIMEOUT_SECONDS = 30,
			MAX_QUEUED_PACKETS = 256,
			MAX_QUEUED_BYTES = 1048576,
			QUEUED_SLOT_CHARGE_BYTES = 64,
			MAX_PACKET_BYTES = 262144,
			MAX_INSTANCE_SLOTS = 256,
			MAX_DEPTH = 32,
			MAX_VALUE_VISITS = 65536,
			MAX_COLLECTION_ENTRIES = 16384,
			MAX_STRING_BYTES = 65536,
			VARUINT_MAX_BYTES = 5,
			MAX_UNION_BRANCHES = 256,
			MAX_SCHEMA_NODES = 4096,
			CFRAME_TOLERANCE = 0.001,
			MAX_ACTIVE_HANDLERS = 64,
			MAX_PENDING_ASKS = 64,
			HANDLER_LIFETIME_SECONDS = 30,
			DEFAULT_ASK_TIMEOUT_SECONDS = 10,
			MAX_ASK_TIMEOUT_SECONDS = 120,
		}
		for name, value in expected do
			spec.eq(actual[name], value, name)
		end
		for name in actual do
			assert(expected[name] ~= nil, `missing parity row {name}`)
		end
	end,
}
