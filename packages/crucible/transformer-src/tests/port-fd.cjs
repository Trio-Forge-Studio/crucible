const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const ts = require("typescript");
const crypto = require("node:crypto");
const sources = {
	"src/core/Admin/shared/admin-types.ts": ["AdminSnapshot"],
	"src/core/Admin/shared/upgrade-snapshot.ts": ["UpgradeSnapshot"],
	"src/core/Admin/shared/messages.ts": ["WatchRecordRequest", "EntranceArrival", "AnnouncementMessage"],
	"src/core/Commands/shared/roles.ts": ["Role"],
	"src/core/Commands/shared/command-tree.ts": ["Category", "CommandInfo", "WireNode", "WireTree"],
	"src/core/Commands/shared/command-arguments.ts": ["Skip", "TargetSpec", "ArgumentSpec"],
	"src/core/Place/shared/environments.ts": ["PlaceName"],
	"src/core/Party/shared/model.ts": ["ReceivedInvite", "FriendElsewhere", "PartyPrivate", "PartyRequest"],
	"src/core/Notifications/shared/notification.ts": ["NotificationKind", "NotificationAction", "Notification"],
	"src/core/Contracts/shared/messages.ts": ["ContractRequest"],
	"src/core/Device/shared/messages.ts": ["NameplateDevice"],
	"src/core/Appearance/shared/messages.ts": ["SkinReveal"],
	"src/core/Appearance/shared/skin-catalog.ts": ["Rarity"],
	"src/core/Progression/shared/rewards.ts": ["RewardSource"],
	"src/job/CodeDoor/shared/code-door.ts": ["KeypadResult"],
	"src/job/Lifts/shared/messages.ts": ["LiftPress"],
	"src/warehouse/Codes/shared/codes.ts": ["CodeResult"],
	"src/warehouse/PlayerCard/shared/messages.ts": ["ProfileReply", "ProfileRequest"],
	"src/warehouse/PlayerCard/shared/public-profile.ts": [
		"ItemCountWire",
		"HouseCountWire",
		"JobRecordWire",
		"PublicProfileWire",
	],
	"src/warehouse/Vehicles/shared/parking.ts": ["ParkingPose"],
};
let fixture = 'import { f64, u32, u8 } from "@trioforge/crucible";\n';
const provenance = [];
for (const [file, names] of Object.entries(sources)) {
	const result = spawnSync(
		"git",
		["-C", "C:/Users/Admin/dev/trio-forge/fragile-delivery", "show", `origin/master:${file}`],
		{ encoding: "utf8" },
	);
	if (result.status !== 0) throw new Error(result.stderr);
	const sf = ts.createSourceFile(file, result.stdout, ts.ScriptTarget.Latest, true);
	for (const name of names) {
		const decl = sf.statements.find(
			(s) => s.name?.text === name && (ts.isInterfaceDeclaration(s) || ts.isTypeAliasDeclaration(s)),
		);
		if (!decl) throw new Error(`${file} lacks ${name}`);
		fixture += ts.createPrinter({ removeComments: true }).printNode(ts.EmitHint.Unspecified, decl, sf) + "\n";
	}
	provenance.push({
		file,
		declarations: names,
		sha256: crypto.createHash("sha256").update(result.stdout).digest("hex"),
	});
}
fs.writeFileSync(path.join(__dirname, "fixtures/network/fd.ts"), fixture);
fs.writeFileSync(
	path.join(__dirname, "fixtures/network/fd-sources.json"),
	JSON.stringify(provenance, null, "\t") + "\n",
);
console.log(
	`Extracted ${provenance.reduce((n, s) => n + s.declarations.length, 0)} payload declarations from ${provenance.length} FD files. No Messages registry copied.`,
);
