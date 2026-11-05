declare const serio: unique symbol;
export type f64 = number & { readonly [serio]: "f64" };
export const rebindableActions = ["Interact", "Carry", "Throw"] as const;
export type RebindableAction = (typeof rebindableActions)[number];
export type Keybinds = Partial<Record<RebindableAction, string>>;
export type ItemTallies = Readonly<Partial<Record<string, number>>>;
export interface JobRecord {
	readonly id: string;
	readonly houseId: string;
	readonly difficulty?: string;
	readonly specialIds: readonly string[];
	readonly endedAt: number;
	readonly seconds: number;
	readonly coins: number;
	readonly xp: number;
	readonly items: ItemTallies;
	readonly partyUserIds: readonly number[];
}
export interface Career {
	readonly firstJoined: number;
	readonly houseJobs: Readonly<Partial<Record<string, number>>>;
	readonly jobs: readonly JobRecord[];
}
export interface PlayedAnchor {
	readonly savedSeconds: number;
	readonly sessionStartedAt: number;
}
export type VolumeChannel = "Game" | "Music" | "Interface";
export type ButtonPrompts = "Auto" | "Xbox" | "PlayStation";
export type FreeCursorMode = "Hold" | "Toggle";
export type GraphicsQuality = "Auto" | "High" | "Medium" | "Low";
export interface PlayerSettings {
	readonly keybinds: Keybinds;
	readonly volume: Readonly<Record<VolumeChannel, number>>;
	readonly vibration: boolean;
	readonly buttonPrompts: ButtonPrompts;
	readonly freeCursor: FreeCursorMode;
	readonly graphics: GraphicsQuality;
	readonly reduceBlur: boolean;
	readonly debugInfo: boolean;
}
export interface DailyRewardHistory {
	readonly lastClaimDay: number;
	readonly claims: number;
	readonly streak: number;
}
export interface EntranceSettings {
	readonly message: boolean;
	readonly warehouseText: string;
	readonly jobText: string;
	readonly music: boolean;
	readonly musicId: number;
}
export interface CoreData {
	readonly career: Career;
	readonly version: number;
	readonly currency: number;
	readonly xp: number;
	readonly upgrades: Readonly<Record<string, number>>;
	readonly mastery: Readonly<
		Record<
			string,
			{
				readonly level: number;
				readonly tasks: number;
			}
		>
	>;
	readonly ownedSkins: Readonly<Record<string, boolean>>;
	readonly equippedSkin: string;
	readonly equippedTitle: string;
	readonly equippedNameColor: string;
	readonly purchaseIds: readonly string[];
	readonly pendingRewards: {
		readonly currency: number;
		readonly xp: number;
	};
	readonly dailyReward: DailyRewardHistory;
	readonly redeemedCodes: Readonly<Record<string, number>>;
	readonly settings: PlayerSettings;
	readonly ftueStep: number;
	readonly firstPurchaseLogged: boolean;
	readonly referral: {
		readonly referredBy: number;
		readonly rewarded: number;
	};
	readonly entrance: EntranceSettings;
	readonly stats: {
		readonly jobsCompleted: number;
		readonly specialJobsCompleted: number;
		readonly itemsDelivered: number;
		readonly itemsBroken: number;
		readonly currencyEarned: number;
	};
	readonly analytics: {
		readonly timesJoined: number;
		readonly playTime: number;
		readonly lastSeen?: number;
		readonly codesRedeemed: number;
		readonly sessionJobs: number;
	};
}
export interface PlayerData extends CoreData {}
export type OwnerData = Omit<PlayerData, "purchaseIds" | "redeemedCodes" | "analytics"> & {
	readonly played: PlayedAnchor;
};
export type AdminEdit = {
	readonly By: string;
} & (
	| {
			readonly Command: "give" | "take" | "givexp" | "takexp";
			readonly Values: readonly [number];
	  }
	| {
			readonly Command: "upgrade";
			readonly Values: readonly [string, boolean?];
	  }
	| {
			readonly Command: "upgrades" | "resetupgrades" | "daily";
			readonly Values: readonly [];
	  }
);
export interface LogEntry {
	readonly At: number;
	readonly UserId: number;
	readonly Name: string;
	readonly Text: string;
	readonly Ok: boolean;
	readonly Place: string;
	readonly TargetUserIds?: readonly f64[];
}
export interface SavedEntry extends LogEntry {
	readonly TargetUserId?: number;
}
