import { f64 } from "@trioforge/crucible";
import { config, dataStore, playerConfig, profileMessage } from "@trioforge/crucible/server";
import type { Keybinds } from "../shared/fd";

declare const any: never;

export const Settings = profileMessage<{ readonly keybinds: Keybinds }>("Probe.Keybinds");
interface MigratedEntry {
	readonly At: number;
	readonly UserId: number;
	readonly Name: string;
	readonly Text: string;
	readonly Ok: boolean;
	readonly Place: string;
	readonly TargetUserIds?: readonly f64[];
	readonly TargetUserId?: number;
	readonly Id?: string;
}
export const CommandLog = dataStore<{ readonly entries: readonly MigratedEntry[] }>("FD.CommandLog", {
	defaults: { entries: [] },
});
export const PartySecrets = dataStore<{ readonly key: string }>("FD.Party", { defaults: any });
export const Counters = dataStore<Readonly<Record<string, number>>>("Game.Counters", { defaults: {} });
export const AdminEditMessage = profileMessage<{
	readonly Command: string;
	readonly Values: readonly (string | number | boolean)[];
	readonly By: string;
}>("Admin.Edit");
interface GlobalEvent {
	readonly name: string;
	readonly startsAt: number;
	readonly endsAt: number;
	readonly multipliers: { readonly coins: number; readonly xp: number };
}
export const GlobalEventConfig = config<GlobalEvent>("GlobalEvent", any);
export const WelcomeDiscount = playerConfig<number>("Shop.WelcomeDiscount", 0);
export const Mode = config<"Walk" | "Drive">("Game.Mode", "Walk");
