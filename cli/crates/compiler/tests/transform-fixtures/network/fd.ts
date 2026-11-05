import { f64, u32, u8 } from "@trioforge/crucible";
export interface AdminSnapshot {
	readonly UserId: f64;
	readonly Name: string;
	readonly DisplayName: string;
	readonly Role: Role;
	readonly Online: boolean;
	readonly Place?: PlaceName;
	readonly JoinedAt?: f64;
	readonly LastSeen?: f64;
	readonly Skins: u32;
	readonly Currency: f64;
	readonly Xp: f64;
	readonly Upgrades: readonly UpgradeSnapshot[];
	readonly DailyStreak: u32;
	readonly DailyClaims: u32;
	readonly JobsCompleted: u32;
	readonly ItemsDelivered: u32;
	readonly ItemsBroken: u32;
	readonly CurrencyEarned: f64;
	readonly TimesJoined: u32;
	readonly PlayTime: f64;
}
export interface UpgradeSnapshot {
	readonly Id: string;
	readonly Tiers: u32;
}
export interface WatchRecordRequest {
	readonly userId?: f64;
}
export interface EntranceArrival {
	readonly userId: f64;
	readonly text?: string;
	readonly musicId?: f64;
}
export interface AnnouncementMessage {
	readonly text: string;
}
export type Role = "Player" | "Admin" | "Owner";
export type Category = "Data" | "Movement" | "Self" | "Moderation" | "Server" | "Job";
export interface CommandInfo {
	readonly name: string;
	readonly help: string;
	readonly category: Category;
	readonly role: Role;
	readonly place?: PlaceName;
	readonly destructive?: boolean;
	readonly reads?: boolean;
}
export interface WireNode {
	readonly name: string;
	readonly executable: boolean;
	readonly children: readonly number[];
	readonly argument?: ArgumentSpec;
}
export interface WireTree {
	readonly nodes: readonly WireNode[];
	readonly commands: readonly CommandInfo[];
}
export type Skip = "Caller" | "Outranked";
export interface TargetSpec {
	readonly type: "player" | "user";
	readonly many: boolean;
	readonly skip?: Skip;
}
export type ArgumentSpec =
	| TargetSpec
	| {
			readonly type: "amount";
			readonly min: number;
			readonly max: number;
	  }
	| {
			readonly type: "toggle";
	  }
	| {
			readonly type: "choice";
			readonly options: readonly string[];
			readonly repeats: boolean;
	  }
	| {
			readonly type: "duration";
	  }
	| {
			readonly type: "text";
			readonly maxLength: number;
			readonly greedy: boolean;
	  };
export type PlaceName = "Warehouse" | "Job";
export interface ReceivedInvite {
	readonly From: f64;
	readonly FromName: string;
	readonly PartySize: u8;
	readonly Until: f64;
}
export interface FriendElsewhere {
	readonly UserId: f64;
	readonly Name: string;
	readonly DisplayName: string;
	readonly OnJob: boolean;
}
export interface PartyPrivate {
	readonly Invites: readonly ReceivedInvite[];
	readonly Friends: readonly FriendElsewhere[];
}
export type PartyRequest =
	| {
			readonly Kind: "Invite";
			readonly UserId: f64;
	  }
	| {
			readonly Kind: "Join";
			readonly From: f64;
	  }
	| {
			readonly Kind: "Decline";
			readonly From: f64;
	  }
	| {
			readonly Kind: "Leave";
	  }
	| {
			readonly Kind: "Remove";
			readonly UserId: f64;
	  }
	| {
			readonly Kind: "Cancel";
			readonly UserId: f64;
	  }
	| {
			readonly Kind: "Watch";
			readonly On: boolean;
	  };
export type NotificationKind = "Info" | "Success" | "Error";
export type NotificationAction = "OpenParty";
export interface Notification {
	readonly Kind: NotificationKind;
	readonly Text: string;
	readonly Title?: string;
	readonly Action?: NotificationAction;
}
export interface ContractRequest {
	readonly Kind: "Accept" | "Cancel";
	readonly Contract?: string;
}
export type NameplateDevice = "Keyboard" | "Touch" | "Xbox" | "PlayStation" | "VR";
export interface SkinReveal {
	readonly Skin: string;
	readonly Source: string;
	readonly Short: boolean;
	readonly PreviewAs?: Rarity;
}
export type Rarity = "Common" | "Rare" | "Epic" | "Limited" | "Legendary" | "Mythic" | "Divine";
export type RewardSource = "Code" | "DailyReward" | "JobPayout" | "Referral" | "Refund";
export type KeypadResult = "Right" | "Wrong" | "Wait" | "Closed";
export interface LiftPress {
	readonly button: BasePart;
	readonly expectedStop: u8;
}
export type CodeResult = "Redeemed" | "Unknown" | "Expired" | "AlreadyRedeemed" | "NotEligible" | "RateLimited";
export interface ProfileReply {
	readonly userId: f64;
	readonly profile?: PublicProfileWire;
}
export interface ProfileRequest {
	readonly userId: f64;
}
export interface ItemCountWire {
	readonly id: string;
	readonly count: u32;
}
export interface HouseCountWire {
	readonly houseId: string;
	readonly count: u32;
}
export interface JobRecordWire {
	readonly id: string;
	readonly houseId: string;
	readonly difficulty: string;
	readonly specialIds: readonly string[];
	readonly endedAt: f64;
	readonly seconds: f64;
	readonly coins: f64;
	readonly xp: f64;
	readonly items: readonly ItemCountWire[];
	readonly partyUserIds: readonly f64[];
}
export interface PublicProfileWire {
	readonly xp: f64;
	readonly equippedSkin: string;
	readonly equippedTitle: string;
	readonly houseJobs: readonly HouseCountWire[];
	readonly jobs: readonly JobRecordWire[];
	readonly jobsCompleted: f64;
	readonly itemsDelivered: f64;
	readonly currencyEarned: f64;
	readonly savedSeconds: f64;
	readonly sessionStartedAt: f64;
	readonly mastery: string;
}
export interface ParkingPose {
	readonly position: Vector3;
	readonly look: Vector3;
}
