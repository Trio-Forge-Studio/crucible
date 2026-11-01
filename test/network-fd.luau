local spec = require("./lib/load")
local h = require("./lib/network-codec")
local layout = require("./lib/wire-layout")

local tests = {}
local r = h.realm()
local n = r.n
local one = "00 00 00 00 00 00 f0 3f"
local neg = "00 00 00 00 00 00 f0 bf"
local zero = "00 00 00 00 00 00 00 00"
local vec = one .. " " .. zero .. " " .. neg
local function obj(fields)
	local ordered = {}
	for name, node in fields do
		table.insert(ordered, { name, node })
	end
	table.sort(ordered, function(a, b)
		return a[1] < b[1]
	end)
	return n.object(ordered)
end
local function literals(values)
	local branches = {}
	for _, value in values do
		table.insert(branches, n.literal(value))
	end
	return n.union(branches)
end
local function optional(node)
	return n.union({ n["nil"], node })
end
local model, part, player, seat =
	h.roblox.Instance.new("Model"),
	h.roblox.Instance.new("Part"),
	h.roblox.Instance.new("Player"),
	h.roblox.Instance.new("Seat")
local vector = h.roblox.Vector3.new(1, 0, -1)
local place = n.union({ n["nil"], n.literal("Job"), n.literal("Warehouse") })
local keypad = literals({ "Closed", "Right", "Wait", "Wrong" })
local rarity = n.union({
	n["nil"],
	n.literal("Common"),
	n.literal("Divine"),
	n.literal("Epic"),
	n.literal("Legendary"),
	n.literal("Limited"),
	n.literal("Mythic"),
	n.literal("Rare"),
})
local kinds = literals({ "Cancel", "Decline", "Invite", "Join", "Leave", "Remove", "Watch" })
local houseCount = obj({ count = n.u32, houseId = n.string })
local itemCount = obj({ count = n.u32, id = n.string })
local job = obj({
	coins = n.f64,
	difficulty = n.string,
	endedAt = n.f64,
	houseId = n.string,
	id = n.string,
	items = n.array(itemCount),
	partyUserIds = n.array(n.f64),
	seconds = n.f64,
	specialIds = n.array(n.string),
	xp = n.f64,
})
local profile = obj({
	currencyEarned = n.f64,
	equippedSkin = n.string,
	equippedTitle = n.string,
	houseJobs = n.array(houseCount),
	itemsDelivered = n.f64,
	jobs = n.array(job),
	jobsCompleted = n.f64,
	mastery = n.string,
	savedSeconds = n.f64,
	sessionStartedAt = n.f64,
	xp = n.f64,
})
local snapshot = obj({
	Currency = n.f64,
	DisplayName = n.string,
	JoinedAt = optional(n.f64),
	LastSeen = optional(n.f64),
	Name = n.string,
	Online = n.bool,
	Place = place,
	Role = literals({ "Admin", "Owner", "Player" }),
	Skins = n.u32,
	Upgrades = n.array(obj({ Id = n.string, Tiers = n.u32 })),
	UserId = n.f64,
	Xp = n.f64,
})
local private = obj({
	Friends = n.array(obj({ DisplayName = n.string, Name = n.string, OnJob = n.bool, UserId = n.f64 })),
	Invites = n.array(obj({ From = n.f64, FromName = n.string, PartySize = n.u8, Until = n.f64 })),
})
local fixtures = {
	{ "Admin.Announcement", obj({ text = n.string }), { text = "A" }, "01 41" },
	{ "Admin.CommandTree", n["nil"], nil, "" },
	{
		"Admin.Entrance",
		obj({ musicId = optional(n.f64), text = optional(n.string), userId = n.f64 }),
		{ userId = -1 },
		"00 00 " .. neg,
	},
	{
		"Admin.Record",
		snapshot,
		{
			Currency = 1,
			DisplayName = "A",
			Name = "B",
			Online = true,
			Role = "Owner",
			Skins = 513,
			Upgrades = { { Id = "C", Tiers = 2 } },
			UserId = -1,
			Xp = 1,
		},
		one .. " 01 41 00 00 01 42 01 00 01 01 02 00 00 01 01 43 02 00 00 00 " .. neg .. " " .. one,
	},
	{ "Admin.RunCommand", n.dynamic, { __none = "__none" }, "06 01 06 5f 5f 6e 6f 6e 65 04 06 5f 5f 6e 6f 6e 65" },
	{ "Admin.WatchRecord", obj({ userId = optional(n.f64) }), {}, "00" },
	{ "Analytics.PurchasePrompted", n.f64, -1, neg },
	{ "Analytics.ScreenOpened", literals({ "SkinShop", "Upgrades" }), "Upgrades", "01" },
	{ "Appearance.BuySkin", n.string, "A", "01 41" },
	{ "Appearance.EquipSkin", n.string, "B", "01 42" },
	{
		"Appearance.RevealSkin",
		obj({ PreviewAs = rarity, Short = n.bool, Skin = n.string, Source = n.string }),
		{ PreviewAs = "Rare", Short = true, Skin = "A", Source = "B" },
		"07 01 01 41 01 42",
	},
	{ "Carry.BeginThrow", n["nil"], nil, "" },
	{ "Carry.Drop", n["nil"], nil, "" },
	{ "Carry.PickUp", n.instance({ "Model" }), model, "", 1 },
	{ "Carry.Throw", n.vector3, vector, vec },
	{ "Ceo.GoTo", n["nil"], nil, "" },
	{
		"CodeDoor.KeypadAnswer",
		obj({ keypad = optional(n.instance({ "BasePart" })), result = keypad }),
		{ keypad = part, result = "Wrong" },
		"01 03",
		1,
	},
	{
		"CodeDoor.KeypadTry",
		obj({ code = n.string, keypad = n.instance({ "BasePart" }) }),
		{ code = "12", keypad = part },
		"02 31 32",
		1,
	},
	{ "Codes.Redeem", n.string, "A", "01 41" },
	{
		"Codes.Result",
		literals({ "AlreadyRedeemed", "Expired", "NotEligible", "RateLimited", "Redeemed", "Unknown" }),
		"Redeemed",
		"04",
	},
	{
		"Contracts.Request",
		obj({ Contract = optional(n.string), Kind = literals({ "Accept", "Cancel" }) }),
		{ Contract = "A", Kind = "Cancel" },
		"01 01 41 01",
	},
	{ "Device.Report", literals({ "Keyboard", "PlayStation", "Touch", "VR", "Xbox" }), "VR", "03" },
	{
		"Doorbell.Rang",
		obj({ button = optional(n.instance({ "BasePart" })), chime = n.vector3, houseId = optional(n.string) }),
		{ button = part, chime = vector, houseId = "H-012" },
		"01 " .. vec .. " 01 05 48 2d 30 31 32",
		1,
	},
	{ "Doorbell.Ring", n.instance({ "BasePart" }), part, "", 1 },
	{ "Doors.Toggle", obj({ door = n.instance({ "Model" }), open = n.bool }), { door = model, open = true }, "01", 1 },
	{ "Hatches.Open", n.instance({ "BasePart" }), part, "", 1 },
	{ "Intro.SkipVote", n["nil"], nil, "" },
	{
		"Lifts.Call",
		obj({ button = n.instance({ "BasePart" }), expectedStop = n.u8 }),
		{ button = part, expectedStop = 3 },
		"03",
		1,
	},
	{
		"Lifts.Move",
		obj({ button = n.instance({ "BasePart" }), expectedStop = n.u8 }),
		{ button = part, expectedStop = 5 },
		"05",
		1,
	},
	{ "Loading.Loaded", n["nil"], nil, "" },
	{ "Loading.TeleportCovered", n["nil"], nil, "" },
	{ "Loading.Teleporting", place, "Warehouse", "02" },
	{ "NameColors.Equip", n.string, "A", "01 41" },
	{
		"Notifications.Notify",
		obj({
			Action = n.union({ n["nil"], n.literal("OpenParty") }),
			Kind = literals({ "Error", "Info", "Success" }),
			Text = n.string,
			Title = optional(n.string),
		}),
		{ Action = "OpenParty", Kind = "Success", Text = "A", Title = "B" },
		"01 02 01 41 01 01 42",
	},
	{
		"Party.Private",
		private,
		{
			Friends = { { DisplayName = "A", Name = "B", OnJob = true, UserId = -1 } },
			Invites = { { From = 1, FromName = "C", PartySize = 3, Until = -1 } },
		},
		"01 01 41 01 42 01 " .. neg .. " 01 " .. one .. " 01 43 03 " .. neg,
	},
	{
		"Party.Request",
		obj({ From = optional(n.f64), Kind = kinds, On = optional(n.bool), UserId = optional(n.f64) }),
		{ Kind = "Leave" },
		"00 04 00 00",
	},
	{ "PlayerCard.Profile", obj({ profile = optional(profile), userId = n.f64 }), { userId = -1 }, "00 " .. neg },
	{ "PlayerCard.RequestProfile", obj({ userId = n.f64 }), { userId = -1 }, neg },
	{ "Progression.ClaimDailyReward", n["nil"], nil, "" },
	{ "Progression.ClaimRewards", n["nil"], nil, "" },
	{
		"Progression.LevelUp",
		obj({ From = n.u16, Player = n.instance({ "Player" }), To = n.u16 }),
		{ From = 258, Player = player, To = 772 },
		"02 01 04 03",
		1,
	},
	{ "Progression.PurchaseUpgrade", n.string, "A", "01 41" },
	{
		"Progression.RewardGranted",
		obj({
			Currency = n.f64,
			Source = literals({ "Code", "DailyReward", "JobPayout", "Referral", "Refund" }),
			Xp = n.f64,
		}),
		{ Currency = 1, Source = "Refund", Xp = -1 },
		one .. " 04 " .. neg,
	},
	{
		"Replication.StateSync",
		n.dynamic,
		{ { __none = "__none" } },
		"05 01 06 01 06 5f 5f 6e 6f 6e 65 04 06 5f 5f 6e 6f 6e 65",
	},
	{ "Seating.Sit", n.instance({ "Seat" }), seat, "", 1 },
	{ "Seating.Stand", n["nil"], nil, "" },
	{ "Settings.Save", n.dynamic, { a = false }, "06 01 01 61 01" },
	{ "Titles.Equip", n.string, "A", "01 41" },
	{ "Vehicles.Drive", n.instance({ "Model" }), model, "", 1 },
	{
		"Vehicles.GetOff",
		optional(obj({ look = n.vector3, position = n.vector3 })),
		{ look = vector, position = h.roblox.Vector3.new(-1, 1, 0) },
		"01 " .. vec .. " " .. neg .. " " .. one .. " " .. zero,
	},
}

local toClient = {
	["Admin.Announcement"] = true,
	["Admin.Entrance"] = true,
	["Admin.Record"] = true,
	["Appearance.RevealSkin"] = true,
	["CodeDoor.KeypadAnswer"] = true,
	["Codes.Result"] = true,
	["Doorbell.Rang"] = true,
	["Loading.Teleporting"] = true,
	["Notifications.Notify"] = true,
	["Party.Private"] = true,
	["PlayerCard.Profile"] = true,
	["Progression.LevelUp"] = true,
	["Progression.RewardGranted"] = true,
	["Replication.StateSync"] = true,
}
local definitions = {}
for _, fixture in fixtures do
	local name = fixture[1]
	local ask = name == "Admin.CommandTree" or name == "Admin.RunCommand"
	definitions[name] = r.catalog.define({
		name = name,
		kind = if ask then "ask-server" elseif toClient[name] then "to-client" else "to-server",
		input = fixture[2],
		output = if ask then n.dynamic else nil,
	})
end
local workedMap = n.map(literals({ "a", "b" }), n.u8)
definitions["Worked.Map"] = r.catalog.define({
	name = "Worked.Map",
	kind = "to-client",
	input = workedMap,
})
local published = r.catalog.seal()

local function byte(value)
	local out = {}
	repeat
		local low = value % 128
		value = value // 128
		table.insert(out, string.format("%02x", if value > 0 then low + 128 else low))
	until value == 0
	return table.concat(out, " ")
end

local function rejectsEveryCut(node, packet, sidecar, slots)
	local reasons = layout.cutReasons(node, packet)
	for length = 0, buffer.len(packet) - 1 do
		local cut = buffer.create(length)
		buffer.copy(cut, 0, packet, 0, length)
		h.rejects(function()
			h.decode(r, node, cut, sidecar, slots)
		end, reasons[length])
	end
end

local function header(opcode, fields, reason)
	local writer = r.codec.Writer.new()
	writer:u8(opcode)
	for _, field in fields or {} do
		writer:varuint(field)
	end
	if reason ~= nil then
		writer:u8(reason)
	end
	return (writer:finish(buffer.create(0)))
end

local function message(definition, value, nonce)
	local writer = r.codec.Writer.new()
	definition.input.encode(writer, value, "payload")
	local opcode = if nonce == nil then 0 else 1
	local fields = if nonce == nil
		then header(opcode, { definition.index, #writer.slots })
		else header(opcode, { definition.index, nonce, #writer.slots })
	return writer:finish(fields)
end

tests["9 12 FD names seal into one realm catalog in byte order"] = function()
	local names = {}
	for name in definitions do
		table.insert(names, name)
	end
	table.sort(names)
	spec.eq(#names, 51, "50 FD declarations and the worked map")
	spec.eq(published, table.concat(names, "\n"), "published Catalog text")
	for index, name in names do
		spec.eq(definitions[name].index, index - 1, `{name} index is its sealed position`)
		spec.eq(r.catalog.definitionAt(index - 1), definitions[name], `{name} resolves by index`)
	end
end

tests["9 index 128 frames as a two-byte varuint after 128 earlier sorted names"] = function()
	local large = h.realm()
	local node = large.n.object({ { "door", large.n.instance({ "Model" }) }, { "open", large.n.bool } })
	local toggle = large.catalog.define({ name = "Doors.Toggle", kind = "to-server", input = node })
	for i = 127, 0, -1 do
		large.catalog.define({ name = string.format("Admin.M%03d", i), kind = "to-server", input = large.n["nil"] })
	end
	large.catalog.define({ name = "Zones.Last", kind = "to-server", input = large.n["nil"] })
	large.catalog.seal()
	spec.eq(large.catalog.catalogSize(), 130, "130 sealed declarations")
	spec.eq(large.catalog.definitionAt(127).name, "Admin.M127", "last one-byte index")
	spec.eq(toggle.index, 128, "Doors.Toggle sorts after the 128 Admin names")
	spec.eq(large.catalog.definitionAt(128), toggle, "index 128 resolves")
	local framed = message(toggle, { door = model, open = true })
	spec.eq(h.hex(framed), "00 80 01 01 01", "index 128 is the varuint 80 01")
	local reader = large.codec.Reader.new(framed, 1, {}, 0)
	spec.eq(reader:varuint(), 128, "header index decodes")
end

for _, fixture in fixtures do
	tests[`FD {fixture[1]} payload golden bytes`] = function()
		local node, value, golden = fixture[2], fixture[3], fixture[4]
		local packet, sidecar, slots = h.encode(r, node, value)
		spec.eq(h.hex(packet), golden, "golden payload")
		spec.eq(slots, fixture[5] or 0, "slot count")
		local definition = definitions[fixture[1]]
		local ask = definition.kind == "ask-server"
		local framed = message(definition, value, if ask then 1 else nil)
		local prefix = if ask then `01 {byte(definition.index)} 01` else `00 {byte(definition.index)}`
		local expected = `{prefix} {byte(slots)}` .. (if golden == "" then "" else ` {golden}`)
		spec.eq(h.hex(framed), expected, "header from the realm's sealed index, then the payload")
		local decoded = h.decode(r, node, packet, sidecar, slots)
		spec.eq(h.hex(h.encode(r, node, decoded)), golden, "decoded round trip")
		rejectsEveryCut(node, packet, sidecar, slots)
	end
end

tests["71 worked map cuts reject by the wire layout of a two-byte entry"] = function()
	local packet = h.bytes("02 00 01 01 02")
	spec.eq(h.hex(h.encode(r, workedMap, { a = 1, b = 2 })), h.hex(packet), "worked map payload")
	rejectsEveryCut(workedMap, packet)
end

tests["worked bytes command responses absent button and Party Leave variant"] = function()
	spec.eq(h.hex(h.encode(r, n.dynamic, nil)), "00", "CommandTree nil response")
	local tree = definitions["Admin.CommandTree"]
	local ix = byte(tree.index)
	spec.eq(h.hex(message(tree, nil, 1)), `01 {ix} 01 00`, "first ask of the session")
	local reply = r.codec.Writer.new()
	tree.output.encode(reply, nil, "response")
	spec.eq(h.hex(reply:finish(header(2, { tree.index, 1, 0 }))), `02 {ix} 01 00 00`, "OK with a dynamic nil")
	spec.eq(h.hex(header(3, { tree.index, 1 }, 0)), `03 {ix} 01 00`, "ERROR unavailable")
	spec.eq(h.hex(header(3, { tree.index, 1 }, 3)), `03 {ix} 01 03`, "ERROR invalid")
	local toggle = definitions["Doors.Toggle"]
	spec.eq(
		h.hex(message(toggle, { door = model, open = true })),
		`00 {byte(toggle.index)} 01 01`,
		"Doors.Toggle EVENT with one slot"
	)
	local rang = definitions["Doorbell.Rang"]
	spec.eq(
		h.hex(message(rang, { chime = vector, houseId = "H-012" })),
		`00 {byte(rang.index)} 00 00 {vec} 01 05 48 2d 30 31 32`,
		"Doorbell.Rang without its button"
	)
	local map = definitions["Worked.Map"]
	spec.eq(
		h.hex(message(map, { b = 2, a = 1 })),
		`00 {byte(map.index)} 00 02 00 01 01 02`,
		"literal-union map keys by tag"
	)
	spec.eq(h.hex(header(4)), "04", "READY")
	spec.eq(h.hex(header(5, {}, 0)), "05 00", "FAIL overflow")
	local node = obj({ button = optional(n.instance({ "BasePart" })), chime = n.vector3, houseId = optional(n.string) })
	spec.eq(
		h.hex(h.encode(r, node, { chime = vector, houseId = "H-012" })),
		"00 " .. vec .. " 01 05 48 2d 30 31 32",
		"missing button"
	)
	local branches = {}
	for _, kind in { "Cancel", "Decline", "Invite", "Join", "Leave", "Remove", "Watch" } do
		table.insert(
			branches,
			n.variant("Kind", kind, if kind == "Invite" then obj({ UserId = n.f64 }) else n.object({}))
		)
	end
	local party = n.union(branches)
	spec.eq(h.hex(h.encode(r, party, { Kind = "Leave" })), "04", "Leave")
	spec.eq(h.hex(h.encode(r, party, { Kind = "Invite", UserId = 123 })), "02 00 00 00 00 00 c0 5e 40", "Invite")
end

tests["FD PlayerCard.Profile nested arrays have golden field order and widths"] = function()
	local value = {
		currencyEarned = 1,
		equippedSkin = "A",
		equippedTitle = "B",
		houseJobs = { { count = 513, houseId = "C" } },
		itemsDelivered = -1,
		jobs = {
			{
				coins = 1,
				difficulty = "D",
				endedAt = -1,
				houseId = "E",
				id = "F",
				items = { { count = 2, id = "G" } },
				partyUserIds = { -1 },
				seconds = 1,
				specialIds = { "H" },
				xp = -1,
			},
		},
		jobsCompleted = 1,
		mastery = "I",
		savedSeconds = -1,
		sessionStartedAt = 1,
		xp = -1,
	}
	local golden = one
		.. " 01 41 01 42 01 01 02 00 00 01 43 "
		.. neg
		.. " 01 "
		.. one
		.. " 01 44 "
		.. neg
		.. " 01 45 01 46 01 02 00 00 00 01 47 01 "
		.. neg
		.. " "
		.. one
		.. " 01 01 48 "
		.. neg
		.. " "
		.. one
		.. " 01 49 "
		.. neg
		.. " "
		.. one
		.. " "
		.. neg
	spec.eq(h.hex(h.encode(r, profile, value)), golden, "nested profile bytes")
	local decoded = h.decode(r, profile, h.bytes(golden))
	spec.eq(decoded.jobs[1].items[1].count, 2, "nested u32")
	spec.eq(decoded.jobs[1].partyUserIds[1], -1, "negative Studio user id")
	spec.eq(h.hex(h.encode(r, profile, decoded)), golden, "nested round trip")
end

return tests
