import { toServer, toClient, askServer, u16, f64 } from "@trioforge/crucible";
import {
	AdminSnapshot,
	WatchRecordRequest,
	EntranceArrival,
	AnnouncementMessage,
	WireTree,
	PlaceName,
	PartyPrivate,
	PartyRequest,
	Notification,
	ContractRequest,
	NameplateDevice,
	SkinReveal,
	RewardSource,
	KeypadResult,
	LiftPress,
	CodeResult,
	ProfileReply,
	ProfileRequest,
	ParkingPose,
} from "./fd";
export const M0 = toClient<AnnouncementMessage>("Admin.Announcement");
export const M1 = askServer<undefined, unknown>("Admin.CommandTree");
export const M2 = toClient<EntranceArrival>("Admin.Entrance");
export const M3 =
	toClient<
		Pick<
			AdminSnapshot,
			| "Currency"
			| "DisplayName"
			| "JoinedAt"
			| "LastSeen"
			| "Name"
			| "Online"
			| "Place"
			| "Role"
			| "Skins"
			| "Upgrades"
			| "UserId"
			| "Xp"
		>
	>("Admin.Record");
export const M4 = askServer<unknown, unknown>("Admin.RunCommand");
export const M5 = toServer<WatchRecordRequest>("Admin.WatchRecord");
export const M6 = toServer<number>("Analytics.PurchasePrompted");
export const M7 = toServer<"Upgrades" | "SkinShop">("Analytics.ScreenOpened");
export const M8 = toServer<string>("Appearance.BuySkin");
export const M9 = toServer<string>("Appearance.EquipSkin");
export const M10 = toClient<SkinReveal>("Appearance.RevealSkin");
export const M11 = toServer<undefined>("Carry.BeginThrow");
export const M12 = toServer<undefined>("Carry.Drop");
export const M13 = toServer<Model>("Carry.PickUp");
export const M14 = toServer<Vector3>("Carry.Throw");
export const M15 = toServer<undefined>("Ceo.GoTo");
export const M16 = toClient<{ keypad: BasePart | undefined; result: KeypadResult }>("CodeDoor.KeypadAnswer");
export const M17 = toServer<{ keypad: BasePart; code: string }>("CodeDoor.KeypadTry");
export const M18 = toServer<string>("Codes.Redeem");
export const M19 = toClient<CodeResult>("Codes.Result");
export const M20 = toServer<ContractRequest>("Contracts.Request");
export const M21 = toServer<NameplateDevice>("Device.Report");
export const M22 = toClient<{ button: BasePart | undefined; chime: Vector3; houseId: string | undefined }>(
	"Doorbell.Rang",
);
export const M23 = toServer<BasePart>("Doorbell.Ring");
export const M24 = toServer<{ door: Model; open: boolean }>("Doors.Toggle");
export const M25 = toServer<BasePart>("Hatches.Open");
export const M26 = toServer<undefined>("Intro.SkipVote");
export const M27 = toServer<LiftPress>("Lifts.Call");
export const M28 = toServer<LiftPress>("Lifts.Move");
export const M29 = toServer<undefined>("Loading.Loaded");
export const M30 = toServer<undefined>("Loading.TeleportCovered");
export const M31 = toClient<PlaceName | undefined>("Loading.Teleporting");
export const M32 = toServer<string>("NameColors.Equip");
export const M33 = toClient<Notification>("Notifications.Notify");
export const M34 = toClient<PartyPrivate>("Party.Private");
export const M35 = toServer<PartyRequest>("Party.Request");
export const M36 = toClient<ProfileReply>("PlayerCard.Profile");
export const M37 = toServer<ProfileRequest>("PlayerCard.RequestProfile");
export const M38 = toServer<undefined>("Progression.ClaimDailyReward");
export const M39 = toServer<undefined>("Progression.ClaimRewards");
export const M40 = toClient<{ Player: Player; From: u16; To: u16 }>("Progression.LevelUp");
export const M41 = toServer<string>("Progression.PurchaseUpgrade");
export const M42 = toClient<{ Currency: f64; Xp: f64; Source: RewardSource }>("Progression.RewardGranted");
export const M43 = toClient<unknown>("Replication.StateSync");
export const M44 = toServer<Seat>("Seating.Sit");
export const M45 = toServer<undefined>("Seating.Stand");
export const M46 = toServer<unknown>("Settings.Save");
export const M47 = toServer<string>("Titles.Equip");
export const M48 = toServer<Model>("Vehicles.Drive");
export const M49 = toServer<ParkingPose | undefined>("Vehicles.GetOff");
export const Typed = askServer<undefined, WireTree>("Admin.TypedTree");
export const Full = toClient<AdminSnapshot>("Admin.FullRecord");
