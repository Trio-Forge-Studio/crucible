-- Two-realm sync specs over the fake link. One server realm boots with the declarations and publishers a spec gives;
-- each joined client evaluates the same declarations. Every frame ServerSync hands to the server session is recorded
-- with the packet the session fired, so `finish` can pin frameCost against networking's own encoder and decoder on
-- every frame a spec sent (rule 69).
local Link = require("./network-link")
local spec = require("./load")

local HEARTBEAT = 5
local EVENT = 0

local world = {}
world.__index = world

local function decoder(realm)
	return {
		envelope = realm.graph.module("internal/network/envelope"),
		codec = realm.graph.module("internal/network/codec"),
		frame = realm.graph.module("internal/sync/frame"),
	}
end

-- options.declare(realm) returns the realm's declarations; options.serve(server, declarations, world) defines the
-- server's publishers; options.patch(world) runs before boot; options.globals reaches every realm's module env.
function world.new(options)
	local link = Link.new({ globals = options.globals })
	local self = setmetatable({ link = link, options = options, sent = {}, refuse = 0 }, world)
	local server = link.server
	self.server = server
	server.api = server.graph.module("")
	server.declarations = server.graph.module("internal/sync/declarations")
	server.values = server.graph.module("internal/sync/values")
	server.reactiveLimits = server.graph.module("internal/reactive/limits")
	self.wire = decoder(server)
	local frameModule = self.wire.frame
	local Session = server.graph.module("internal/network/server-session").ServerSession
	local send = Session.send
	Session.send = function(session, player, token, value)
		local before = #link.log
		local ok, result
		if token == frameModule.SyncFrameMessage and self.refuse > 0 then
			self.refuse -= 1
			ok, result = false, "injected encode refusal"
		else
			ok, result = pcall(send, session, player, token, value)
		end
		if token == frameModule.SyncFrameMessage then
			table.insert(self.sent, {
				player = player.Name,
				frame = value,
				ok = ok,
				result = result,
				entry = if #link.log > before then link.log[before + 1] else nil,
			})
		end
		if not ok then
			error(result, 0)
		end
		return result
	end
	if options.patch then
		options.patch(self)
	end
	local ok, err = link:boot(function(s)
		s.d = options.declare(s)
		if options.serve then
			options.serve(s, s.d, self)
		end
	end)
	self.booted, self.bootError = ok, err
	if options.mustBoot ~= false then
		assert(ok, tostring(err))
	end
	return self
end

-- A client realm for a new player. options.declare overrides the realm's declarations; options.patch(client) runs
-- before the client entry loads, so it can replace an export a sync module imports.
function world:join(name: string, options)
	options = options or {}
	local declare = options.declare or self.options.declare
	local client = self.link:join(name, function(c)
		c.d = declare(c)
	end, { boot = false })
	client.api = client.graph.module("")
	if options.patch then
		options.patch(client)
	end
	self.link:start(client)
	assert(client.booted, tostring(client.bootError))
	return client
end

function world:heartbeat()
	self.server.engine.frame(HEARTBEAT, 1 / 60)
	self.link:flush()
end

-- The framework's ServerSync object, for specs that read its accounting.
function world:sync()
	for _, record in self.server.registry.records() do
		if record.id == "Crucible.ServerSync" then
			return record.state.object
		end
	end
	error("no ServerSync")
end

local function decode(wire, packet: buffer)
	local header = wire.envelope.readHeader(2, packet, {})
	if header == nil or header.opcode ~= EVENT or header.index ~= wire.frame.SyncFrameMessage.index then
		return nil
	end
	local reader = wire.codec.Reader.new(packet, header.payloadAt, {}, 0)
	local frame = wire.frame.SyncFrameMessage.input.decode(reader, "payload")
	return { frame = frame, visits = reader.budget.visits, entries = reader.budget.entries, bytes = buffer.len(packet) }
end

-- Every sync frame on the wire to `client`, decoded by networking's codec, in order.
function world:frames(client)
	local out = {}
	for _, entry in self.link.log do
		if entry.to == client.name and type(entry.packet) == "buffer" then
			local decoded = decode(self.wire, entry.packet)
			if decoded then
				decoded.injected = entry.injected
				table.insert(out, decoded)
			end
		end
	end
	return out
end

-- The keys of a decoded frame's values, in order.
function world.keys(decoded): { string }
	local out = {}
	for _, entry in decoded.frame.values do
		table.insert(out, entry.key)
	end
	return out
end

function world.value(decoded, key: string)
	for _, entry in decoded.frame.values do
		if entry.key == key then
			return entry.value, true
		end
	end
	return nil, false
end

-- Encodes `frame` with the client's own codec and delivers it as if the server sent it.
function world:inject(client, frame)
	local wire = decoder(client)
	local message = wire.frame.SyncFrameMessage
	local body = wire.envelope.encodeBody(message.name, message.input, "payload", frame)
	local packet = wire.envelope.frame(body, EVENT, { message.index })
	self.link:injectToClient(client, packet.bytes, packet.sidecar)
	self.link:flush()
end

-- Rule 69's parity on every frame the server session fired, then the link's own error check.
function world:finish()
	local index = self.wire.frame.SyncFrameMessage.index
	local checked = 0
	for i, sent in self.sent do
		if sent.ok and sent.result and sent.entry then
			local cost = self.wire.frame.frameCost(sent.frame, index)
			local decoded = decode(self.wire, sent.entry.packet)
			assert(decoded, `frame {i} decodes`)
			spec.eq(cost.bytes, decoded.bytes, `frame {i} to {sent.player}: frameCost bytes equal the encoded packet`)
			spec.eq(cost.visits, decoded.visits, `frame {i} to {sent.player}: frameCost visits equal the decoder's`)
			spec.eq(cost.entries, decoded.entries, `frame {i} to {sent.player}: frameCost entries equal the decoder's`)
			checked += 1
		end
	end
	self.link:assertClean()
	return checked
end

function world:logged(part: string): number
	return self.server.logged(part)
end

return world
