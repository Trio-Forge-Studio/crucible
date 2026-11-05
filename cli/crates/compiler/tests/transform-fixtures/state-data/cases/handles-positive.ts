import {
	profileStore,
	replicatedPerPlayer,
	serverOnly,
	type ClientProfile,
	type ServerOnly,
} from "@trioforge/crucible";
import type { PlayerData as FdPlayerData } from "./fd";

declare const any: never;

type Fields = Omit<FdPlayerData, "version" | "purchaseIds" | "redeemedCodes" | "analytics"> & {
	readonly redeemedCodes: ServerOnly<FdPlayerData["redeemedCodes"]>;
	readonly analytics: ServerOnly<FdPlayerData["analytics"]>;
};
declare const DataTemplate: Fields;
export const PlayerData = profileStore("FD.Players", { template: DataTemplate });
export const FdClient = replicatedPerPlayer<ClientProfile<typeof PlayerData>>("Probe.FdClientType", any);

declare const DataTemplatePlus: Fields & { readonly mood: number };
export const PlayerDataPlus = profileStore("FD.Players.Plus", { template: DataTemplatePlus });
declare const DataTemplateSecret: Fields & { readonly secret: ServerOnly<number> };
export const PlayerDataSecret = profileStore("FD.Players.Secret", { template: DataTemplateSecret });

export const Hidden = profileStore("Game.Hidden", {
	template: {
		coins: 0,
		secret: { a: serverOnly(0), b: { c: serverOnly("") } },
		mixed: { shown: 0, hidden: serverOnly(0) },
	},
});
export const HiddenClient = replicatedPerPlayer<ClientProfile<typeof Hidden>>("Probe.HiddenClientType", any);
export const AllHidden = profileStore("Game.AllHidden", { template: { a: serverOnly(0), b: { c: serverOnly(0) } } });

export const Collide = profileStore("Game.Collide", { template: { n: { a: { b: 0 } } } });
export const CollidePlus = profileStore("Game.Collide.Plus", { template: { n: { a: { b: 0 }, "a.b": 0 } } });
export const CollideMarks = profileStore("Game.Collide.Marks", {
	template: { coins: 0, n: { a: { b: serverOnly(0) }, "a.b": serverOnly(0), shown: 0 } },
});
export const Markers = profileStore("Game.Markers", {
	template: { m: {} as Record<string, { x: number }>, o: { "*": { x: 0 } } },
});
export const DottedSecret = profileStore("Game.DottedSecret", { template: { coins: 0, "a.b": serverOnly(0) } });

interface Analytics {
	readonly timesJoined: number;
	readonly lastSeen?: number;
}
export const Inline = profileStore("Game.Inline", {
	template: {
		currency: 0,
		upgrades: {} as Record<string, number>,
		questProgress: [] as number[],
		title: undefined as string | undefined,
		settings: { vibration: true, graphics: "Auto" as "Auto" | "Low" },
		purchaseIds: serverOnly([] as string[]),
		analytics: serverOnly({ timesJoined: 0 } as Analytics),
	},
});
