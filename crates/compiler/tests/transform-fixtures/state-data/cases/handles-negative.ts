import { profileStore, replicated, serverOnly, type ServerOnly } from "@trioforge/crucible";

declare const any: never;

export const N14 = profileStore("N.N14.ReservedRoot", {
	template: { coins: 0, update: 0 } as { coins: number; update: number },
});
export const N15 = profileStore("N.N15.ReservedNested", { template: any as { settings: { remove: boolean } } });
export const N16 = profileStore("N.N16.ReservedInElement", { template: { log: [] as { set: number }[] } });
export const N17 = profileStore("N.N17.UntypedArray", { template: { quests: [] } });
export const N18 = profileStore("N.N18.UntypedRecord", { template: { upgrades: {} } });
export const N19 = profileStore("N.N19.ServerOnlyElement", { template: { jobs: [] as ServerOnly<number>[] } });
export const N20 = profileStore("N.N20.ServerOnlyTwice", { template: { a: serverOnly({ b: serverOnly(0) }) } });
export const N22 = replicated<{ readonly x: ServerOnly<number> }>("N.N22.ServerOnlyReplicated", any);
export const N23 = profileStore("N.N23.AlwaysUndefined", { template: { note: undefined } });
export const N24 = profileStore("N.N24.ServerOnlyMapValue", {
	template: { m: {} as Record<string, ServerOnly<number>> },
});
export const N25 = profileStore("N.N25.ViewNameDotted", { template: { coins: 0, "a.b": 0 } });
export const N26 = profileStore("N.N26.ViewNameLong.Abcdefghijklmnopqrstu", { template: { specialJobsCompleted: 0 } });
