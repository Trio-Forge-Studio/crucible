const assert = require("node:assert/strict");
const { test } = require("node:test");
const { spawnSync } = require("node:child_process");
const vm = require("node:vm");
const { project, inspect, compile, descriptorExpressions, fs, path } = require("./network-harness.cjs");
const N = (name) => `__crucible.n.${name}`;
const S = N("string"),
	F = N("f64"),
	B = N("bool"),
	Nil = N("nil");
const L = (v) => `${N("literal")}(${JSON.stringify(v)})`;
const U = (...v) => `${N("union")}([${v.join(", ")}])`;
const A = (v) => `${N("array")}(${v})`;
const I = (v) => `${N("instance")}([${JSON.stringify(v)}])`;
const O = (fields) =>
	`${N("object")}([${fields.map(([name, node]) => `[${JSON.stringify(name)}, ${node}]`).join(", ")}])`;
const V = (key, value, body) => `${N("variant")}(${JSON.stringify(key)}, ${JSON.stringify(value)}, ${body})`;
const opt = (node) => U(Nil, node);
const literals = (...values) => U(...values.map(L));
const role = literals("Admin", "Owner", "Player");
const upgrade = O([
	["Id", S],
	["Tiers", N("u32")],
]);
const admin = O([
	["Currency", F],
	["DisplayName", S],
	["JoinedAt", opt(F)],
	["LastSeen", opt(F)],
	["Name", S],
	["Online", B],
	["Place", U(Nil, L("Job"), L("Warehouse"))],
	["Role", role],
	["Skins", N("u32")],
	["Upgrades", A(upgrade)],
	["UserId", F],
	["Xp", F],
]);
const privateParty = O([
	[
		"Friends",
		A(
			O([
				["DisplayName", S],
				["Name", S],
				["OnJob", B],
				["UserId", F],
			]),
		),
	],
	[
		"Invites",
		A(
			O([
				["From", F],
				["FromName", S],
				["PartySize", N("u8")],
				["Until", F],
			]),
		),
	],
]);
const item = O([
	["count", N("u32")],
	["id", S],
]);
const house = O([
	["count", N("u32")],
	["houseId", S],
]);
const job = O([
	["coins", F],
	["difficulty", S],
	["endedAt", F],
	["houseId", S],
	["id", S],
	["items", A(item)],
	["partyUserIds", A(F)],
	["seconds", F],
	["specialIds", A(S)],
	["xp", F],
]);
const profile = O([
	["currencyEarned", F],
	["equippedSkin", S],
	["equippedTitle", S],
	["houseJobs", A(house)],
	["itemsDelivered", F],
	["jobs", A(job)],
	["jobsCompleted", F],
	["mastery", S],
	["savedSeconds", F],
	["sessionStartedAt", F],
	["xp", F],
]);
const party = U(
	...[
		["Cancel", "UserId", F],
		["Decline", "From", F],
		["Invite", "UserId", F],
		["Join", "From", F],
		["Leave"],
		["Remove", "UserId", F],
		["Watch", "On", B],
	].map(([value, field, node]) => V("Kind", value, O(field ? [[field, node]] : []))),
);
const lift = O([
	["button", I("BasePart")],
	["expectedStop", N("u8")],
]);
const rows = [
	["Admin.Announcement", "toClient", "AnnouncementMessage", O([["text", S]])],
	["Admin.CommandTree", "askServer", "undefined, unknown", Nil, N("dynamic")],
	[
		"Admin.Entrance",
		"toClient",
		"EntranceArrival",
		O([
			["musicId", opt(F)],
			["text", opt(S)],
			["userId", F],
		]),
	],
	[
		"Admin.Record",
		"toClient",
		'Pick<AdminSnapshot, "Currency"|"DisplayName"|"JoinedAt"|"LastSeen"|"Name"|"Online"|"Place"|"Role"|"Skins"|"Upgrades"|"UserId"|"Xp">',
		admin,
	],
	["Admin.RunCommand", "askServer", "unknown, unknown", N("dynamic"), N("dynamic")],
	["Admin.WatchRecord", "toServer", "WatchRecordRequest", O([["userId", opt(F)]])],
	["Analytics.PurchasePrompted", "toServer", "number", F],
	["Analytics.ScreenOpened", "toServer", '"Upgrades"|"SkinShop"', literals("SkinShop", "Upgrades")],
	["Appearance.BuySkin", "toServer", "string", S],
	["Appearance.EquipSkin", "toServer", "string", S],
	[
		"Appearance.RevealSkin",
		"toClient",
		"SkinReveal",
		O([
			["PreviewAs", U(Nil, ...["Common", "Divine", "Epic", "Legendary", "Limited", "Mythic", "Rare"].map(L))],
			["Short", B],
			["Skin", S],
			["Source", S],
		]),
	],
	["Carry.BeginThrow", "toServer", "undefined", Nil],
	["Carry.Drop", "toServer", "undefined", Nil],
	["Carry.PickUp", "toServer", "Model", I("Model")],
	["Carry.Throw", "toServer", "Vector3", N("vector3")],
	["Ceo.GoTo", "toServer", "undefined", Nil],
	[
		"CodeDoor.KeypadAnswer",
		"toClient",
		"{keypad: BasePart|undefined; result: KeypadResult}",
		O([
			["keypad", opt(I("BasePart"))],
			["result", literals("Closed", "Right", "Wait", "Wrong")],
		]),
	],
	[
		"CodeDoor.KeypadTry",
		"toServer",
		"{keypad: BasePart; code: string}",
		O([
			["code", S],
			["keypad", I("BasePart")],
		]),
	],
	["Codes.Redeem", "toServer", "string", S],
	[
		"Codes.Result",
		"toClient",
		"CodeResult",
		literals("AlreadyRedeemed", "Expired", "NotEligible", "RateLimited", "Redeemed", "Unknown"),
	],
	[
		"Contracts.Request",
		"toServer",
		"ContractRequest",
		O([
			["Contract", opt(S)],
			["Kind", literals("Accept", "Cancel")],
		]),
	],
	["Device.Report", "toServer", "NameplateDevice", literals("Keyboard", "PlayStation", "Touch", "VR", "Xbox")],
	[
		"Doorbell.Rang",
		"toClient",
		"{button:BasePart|undefined;chime:Vector3;houseId:string|undefined}",
		O([
			["button", opt(I("BasePart"))],
			["chime", N("vector3")],
			["houseId", opt(S)],
		]),
	],
	["Doorbell.Ring", "toServer", "BasePart", I("BasePart")],
	[
		"Doors.Toggle",
		"toServer",
		"{door:Model;open:boolean}",
		O([
			["door", I("Model")],
			["open", B],
		]),
	],
	["Hatches.Open", "toServer", "BasePart", I("BasePart")],
	["Intro.SkipVote", "toServer", "undefined", Nil],
	["Lifts.Call", "toServer", "LiftPress", lift],
	["Lifts.Move", "toServer", "LiftPress", lift],
	["Loading.Loaded", "toServer", "undefined", Nil],
	["Loading.TeleportCovered", "toServer", "undefined", Nil],
	["Loading.Teleporting", "toClient", "PlaceName|undefined", U(Nil, L("Job"), L("Warehouse"))],
	["NameColors.Equip", "toServer", "string", S],
	[
		"Notifications.Notify",
		"toClient",
		"Notification",
		O([
			["Action", opt(L("OpenParty"))],
			["Kind", literals("Error", "Info", "Success")],
			["Text", S],
			["Title", opt(S)],
		]),
	],
	["Party.Private", "toClient", "PartyPrivate", privateParty],
	["Party.Request", "toServer", "PartyRequest", party],
	[
		"PlayerCard.Profile",
		"toClient",
		"ProfileReply",
		O([
			["profile", opt(profile)],
			["userId", F],
		]),
	],
	["PlayerCard.RequestProfile", "toServer", "ProfileRequest", O([["userId", F]])],
	["Progression.ClaimDailyReward", "toServer", "undefined", Nil],
	["Progression.ClaimRewards", "toServer", "undefined", Nil],
	[
		"Progression.LevelUp",
		"toClient",
		"{Player:Player;From:u16;To:u16}",
		O([
			["From", N("u16")],
			["Player", I("Player")],
			["To", N("u16")],
		]),
	],
	["Progression.PurchaseUpgrade", "toServer", "string", S],
	[
		"Progression.RewardGranted",
		"toClient",
		"{Currency:f64;Xp:f64;Source:RewardSource}",
		O([
			["Currency", F],
			["Source", literals("Code", "DailyReward", "JobPayout", "Referral", "Refund")],
			["Xp", F],
		]),
	],
	["Replication.StateSync", "toClient", "unknown", N("dynamic")],
	["Seating.Sit", "toServer", "Seat", I("Seat")],
	["Seating.Stand", "toServer", "undefined", Nil],
	["Settings.Save", "toServer", "unknown", N("dynamic")],
	["Titles.Equip", "toServer", "string", S],
	["Vehicles.Drive", "toServer", "Model", I("Model")],
	[
		"Vehicles.GetOff",
		"toServer",
		"ParkingPose|undefined",
		opt(
			O([
				["look", N("vector3")],
				["position", N("vector3")],
			]),
		),
	],
];

test("FD catalogue lowers all 50 approved payloads PartyRequest variants and typed WireTree", () => {
	assert.equal(rows.length, 50);
	const fd = fs.readFileSync(path.join(__dirname, "fixtures/network/fd.ts"), "utf8");
	const commandInfo = O([
		["category", literals("Data", "Job", "Moderation", "Movement", "Self", "Server")],
		["destructive", opt(B)],
		["help", S],
		["name", S],
		["place", U(Nil, L("Job"), L("Warehouse"))],
		["reads", opt(B)],
		["role", role],
	]);
	const argument = U(
		V(
			"type",
			"amount",
			O([
				["max", F],
				["min", F],
			]),
		),
		V(
			"type",
			"choice",
			O([
				["options", A(S)],
				["repeats", B],
			]),
		),
		V("type", "duration", O([])),
		V(
			"type",
			"player",
			O([
				["many", B],
				["skip", U(Nil, L("Caller"), L("Outranked"))],
			]),
		),
		V(
			"type",
			"text",
			O([
				["greedy", B],
				["maxLength", F],
			]),
		),
		V("type", "toggle", O([])),
		V(
			"type",
			"user",
			O([
				["many", B],
				["skip", U(Nil, L("Caller"), L("Outranked"))],
			]),
		),
	);
	const tree = O([
		["commands", A(commandInfo)],
		[
			"nodes",
			A(
				O([
					["argument", U(Nil, ...descriptorBranches(argument))],
					["children", A(F)],
					["executable", B],
					["name", S],
				]),
			),
		],
	]);
	const text =
		'import {toServer,toClient,askServer,u16,u8,u32,f64} from "@trioforge/crucible";\nimport {' +
		[...fd.matchAll(/export (?:interface|type) (\w+)/g)].map((m) => m[1]).join(",") +
		'} from "./fd";\n' +
		rows.map(([name, kind, type], i) => `export const M${i}=${kind}<${type}>("${name}");`).join("\n") +
		'\nexport const Typed=askServer<undefined,WireTree>("Admin.TypedTree");\nexport const Full=toClient<AdminSnapshot>("Admin.FullRecord");\n';
	const game = project({ "src/shared/fd.ts": fd, "src/shared/messages.ts": text });
	const result = inspect(game);
	assert.deepEqual(
		result.diagnostics.map((d) => d.messageText),
		[],
	);
	const actual = descriptorExpressions(result.output.get("src/shared/messages.ts"));
	for (const [name, kind, , request, response] of rows)
		assert.deepEqual(actual.get(name), kind === "askServer" ? [request, response, "undefined"] : [request], name);
	assert.deepEqual(actual.get("Admin.TypedTree"), [Nil, tree, "undefined"]);
	assert.match(actual.get("Admin.FullRecord")[0], /DailyClaims.*JobsCompleted.*PlayTime/);
	compile(game);
	const luau = fs.readFileSync(path.join(game, "out/shared/messages.luau"), "utf8");
	for (const [name] of rows) assert.ok(luau.includes(`"${name}"`), `${name} emitted by rbxtsc`);
	const n = {};
	for (const kind of [
		"nil",
		"bool",
		"f64",
		"f32",
		"u8",
		"u16",
		"u32",
		"string",
		"vector3",
		"color3",
		"cframe",
		"dynamic",
	])
		n[kind] = { kind };
	for (const [kind, keys] of Object.entries({
		literal: ["value"],
		instance: ["classes"],
		object: ["fields"],
		array: ["element"],
		tuple: ["elements"],
		map: ["key", "value"],
		union: ["branches"],
		variant: ["key", "value", "body"],
	}))
		n[kind] = (...args) => Object.fromEntries([["kind", kind], ...keys.map((k, i) => [k, args[i]])]);
	const expected = Object.fromEntries(
		[...actual].map(([name, expressions]) => [
			name,
			expressions
				.slice(0, name === "Admin.TypedTree" || rows.find((r) => r[0] === name)?.[1] === "askServer" ? 2 : 1)
				.map((expression) => vm.runInNewContext(expression, { __crucible: { n } })),
		]),
	);
	fs.writeFileSync(path.join(game, "expected.json"), JSON.stringify(expected));
	fs.copyFileSync(
		path.join(__dirname, "fixtures/network/emitted-descriptors.luau.txt"),
		path.join(game, "emitted-descriptors.luau"),
	);
	const lune = spawnSync("lune", ["run", "emitted-descriptors.luau"], {
		cwd: game,
		encoding: "utf8",
		timeout: 30000,
	});
	assert.equal(lune.status, 0, lune.stdout + lune.stderr);
	assert.match(lune.stdout, /52 emitted descriptor declarations matched/);
});

function descriptorBranches(expression) {
	const { ts } = require("./network-harness.cjs");
	const sf = ts.createSourceFile("descriptor.ts", expression, ts.ScriptTarget.Latest, true);
	return sf.statements[0].expression.arguments[0].elements.map((e) => e.getText(sf));
}
