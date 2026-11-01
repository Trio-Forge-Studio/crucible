import * as __crucible from "@trioforge/crucible/emit";

export const descriptors = [
	__crucible.n.object([["text", __crucible.n.string]]),
	__crucible.n.nil,
	__crucible.n.dynamic,
	__crucible.n.object([
		["musicId", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])],
		["text", __crucible.n.union([__crucible.n.nil, __crucible.n.string])],
		["userId", __crucible.n.f64],
	]),
	__crucible.n.object([
		["Currency", __crucible.n.f64],
		["DisplayName", __crucible.n.string],
		["JoinedAt", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])],
		["LastSeen", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])],
		["Name", __crucible.n.string],
		["Online", __crucible.n.bool],
		[
			"Place",
			__crucible.n.union([__crucible.n.nil, __crucible.n.literal("Job"), __crucible.n.literal("Warehouse")]),
		],
		[
			"Role",
			__crucible.n.union([
				__crucible.n.literal("Admin"),
				__crucible.n.literal("Owner"),
				__crucible.n.literal("Player"),
			]),
		],
		["Skins", __crucible.n.u32],
		[
			"Upgrades",
			__crucible.n.array(
				__crucible.n.object([
					["Id", __crucible.n.string],
					["Tiers", __crucible.n.u32],
				]),
			),
		],
		["UserId", __crucible.n.f64],
		["Xp", __crucible.n.f64],
	]),
	__crucible.n.object([["userId", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])]]),
	__crucible.n.f64,
	__crucible.n.union([__crucible.n.literal("SkinShop"), __crucible.n.literal("Upgrades")]),
	__crucible.n.object([
		[
			"PreviewAs",
			__crucible.n.union([
				__crucible.n.nil,
				__crucible.n.literal("Common"),
				__crucible.n.literal("Divine"),
				__crucible.n.literal("Epic"),
				__crucible.n.literal("Legendary"),
				__crucible.n.literal("Limited"),
				__crucible.n.literal("Mythic"),
				__crucible.n.literal("Rare"),
			]),
		],
		["Short", __crucible.n.bool],
		["Skin", __crucible.n.string],
		["Source", __crucible.n.string],
	]),
	__crucible.n.instance(["Model"]),
	__crucible.n.vector3,
	__crucible.n.object([
		["keypad", __crucible.n.union([__crucible.n.nil, __crucible.n.instance(["BasePart"])])],
		[
			"result",
			__crucible.n.union([
				__crucible.n.literal("Closed"),
				__crucible.n.literal("Right"),
				__crucible.n.literal("Wait"),
				__crucible.n.literal("Wrong"),
			]),
		],
	]),
	__crucible.n.object([
		["code", __crucible.n.string],
		["keypad", __crucible.n.instance(["BasePart"])],
	]),
	__crucible.n.object([
		["button", __crucible.n.union([__crucible.n.nil, __crucible.n.instance(["BasePart"])])],
		["chime", __crucible.n.vector3],
		["houseId", __crucible.n.union([__crucible.n.nil, __crucible.n.string])],
	]),
	__crucible.n.object([
		["door", __crucible.n.instance(["Model"])],
		["open", __crucible.n.bool],
	]),
	__crucible.n.object([
		["button", __crucible.n.instance(["BasePart"])],
		["expectedStop", __crucible.n.u8],
	]),
	__crucible.n.object([
		["Action", __crucible.n.union([__crucible.n.nil, __crucible.n.literal("OpenParty")])],
		[
			"Kind",
			__crucible.n.union([
				__crucible.n.literal("Error"),
				__crucible.n.literal("Info"),
				__crucible.n.literal("Success"),
			]),
		],
		["Text", __crucible.n.string],
		["Title", __crucible.n.union([__crucible.n.nil, __crucible.n.string])],
	]),
	__crucible.n.object([
		["From", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])],
		[
			"Kind",
			__crucible.n.union([
				__crucible.n.literal("Cancel"),
				__crucible.n.literal("Decline"),
				__crucible.n.literal("Invite"),
				__crucible.n.literal("Join"),
				__crucible.n.literal("Leave"),
				__crucible.n.literal("Remove"),
				__crucible.n.literal("Watch"),
			]),
		],
		["On", __crucible.n.union([__crucible.n.nil, __crucible.n.bool])],
		["UserId", __crucible.n.union([__crucible.n.nil, __crucible.n.f64])],
	]),
	__crucible.n.union([
		__crucible.n.variant("Kind", "Cancel", __crucible.n.object([["UserId", __crucible.n.f64]])),
		__crucible.n.variant("Kind", "Decline", __crucible.n.object([["From", __crucible.n.f64]])),
		__crucible.n.variant("Kind", "Invite", __crucible.n.object([["UserId", __crucible.n.f64]])),
		__crucible.n.variant("Kind", "Join", __crucible.n.object([["From", __crucible.n.f64]])),
		__crucible.n.variant("Kind", "Leave", __crucible.n.object([])),
		__crucible.n.variant("Kind", "Remove", __crucible.n.object([["UserId", __crucible.n.f64]])),
		__crucible.n.variant("Kind", "Watch", __crucible.n.object([["On", __crucible.n.bool]])),
	]),
	__crucible.n.object([
		[
			"profile",
			__crucible.n.union([
				__crucible.n.nil,
				__crucible.n.object([
					["currencyEarned", __crucible.n.f64],
					["equippedSkin", __crucible.n.string],
					["equippedTitle", __crucible.n.string],
					[
						"houseJobs",
						__crucible.n.array(
							__crucible.n.object([
								["count", __crucible.n.u32],
								["houseId", __crucible.n.string],
							]),
						),
					],
					["itemsDelivered", __crucible.n.f64],
					[
						"jobs",
						__crucible.n.array(
							__crucible.n.object([
								["coins", __crucible.n.f64],
								["difficulty", __crucible.n.string],
								["endedAt", __crucible.n.f64],
								["houseId", __crucible.n.string],
								["id", __crucible.n.string],
								[
									"items",
									__crucible.n.array(
										__crucible.n.object([
											["count", __crucible.n.u32],
											["id", __crucible.n.string],
										]),
									),
								],
								["partyUserIds", __crucible.n.array(__crucible.n.f64)],
								["seconds", __crucible.n.f64],
								["specialIds", __crucible.n.array(__crucible.n.string)],
								["xp", __crucible.n.f64],
							]),
						),
					],
					["jobsCompleted", __crucible.n.f64],
					["mastery", __crucible.n.string],
					["savedSeconds", __crucible.n.f64],
					["sessionStartedAt", __crucible.n.f64],
					["xp", __crucible.n.f64],
				]),
			]),
		],
		["userId", __crucible.n.f64],
	]),
	__crucible.n.object([
		["From", __crucible.n.u16],
		["Player", __crucible.n.instance(["Player"])],
		["To", __crucible.n.u16],
	]),
	__crucible.n.object([
		["Currency", __crucible.n.f64],
		[
			"Source",
			__crucible.n.union([
				__crucible.n.literal("Code"),
				__crucible.n.literal("DailyReward"),
				__crucible.n.literal("JobPayout"),
				__crucible.n.literal("Referral"),
				__crucible.n.literal("Refund"),
			]),
		],
		["Xp", __crucible.n.f64],
	]),
	__crucible.n.instance(["Seat"]),
	__crucible.n.union([
		__crucible.n.nil,
		__crucible.n.object([
			["look", __crucible.n.vector3],
			["position", __crucible.n.vector3],
		]),
	]),
	__crucible.n.map(__crucible.n.string, __crucible.n.u8),
	__crucible.n.map(__crucible.n.u8, __crucible.n.string),
	__crucible.n.map(__crucible.n.u16, __crucible.n.bool),
	__crucible.n.map(__crucible.n.u32, __crucible.n.f64),
	__crucible.n.map(__crucible.n.union([__crucible.n.literal("a"), __crucible.n.literal("b")]), __crucible.n.u8),
	__crucible.n.map(
		__crucible.n.union([__crucible.n.literal(-1), __crucible.n.literal(2), __crucible.n.literal("c")]),
		__crucible.n.string,
	),
	__crucible.n.tuple([__crucible.n.f32, __crucible.n.cframe, __crucible.n.color3]),
	__crucible.n.array(__crucible.n.literal("Tick")),
	__crucible.n.array(__crucible.n.object([["Kind", __crucible.n.literal("Leave")]])),
	__crucible.n.union([__crucible.n.literal(false), __crucible.n.literal(3), __crucible.n.literal("x")]),
	__crucible.n.union([__crucible.n.nil, __crucible.n.instance(["BasePart", "Model"])]),
	__crucible.n.union([
		__crucible.n.nil,
		__crucible.n.cframe,
		__crucible.n.color3,
		__crucible.n.instance(["Player"]),
		__crucible.n.vector3,
		__crucible.n.bool,
		__crucible.n.f64,
		__crucible.n.string,
		__crucible.n.array(__crucible.n.u8),
	]),
	__crucible.n.union([
		__crucible.n.literal("None"),
		__crucible.n.variant("Kind", 1, __crucible.n.object([["a", __crucible.n.f64]])),
		__crucible.n.variant("Kind", 2, __crucible.n.object([])),
	]),
	__crucible.n.object([
		["inner", __crucible.n.map(__crucible.n.string, __crucible.n.array(__crucible.n.tuple([__crucible.n.u8])))],
	]),
];
