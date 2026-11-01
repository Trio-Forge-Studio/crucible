import type { ServerOnly } from "@trioforge/crucible";
import { config, dataStore, profileMessage } from "@trioforge/crucible/server";
import type { AdminEdit, SavedEntry } from "../shared/fd";

declare const any: never;

export const N3 = dataStore<{ readonly spawn: CFrame }>("N.N3.CFrame", { defaults: any });
export const N7 = dataStore<string>("N.N7.ScalarRoot", { defaults: any });
export const N8 = dataStore<readonly string[]>("N.N8.ArrayRoot", { defaults: any });
export const N9 = profileMessage<AdminEdit>("N.N9.FdAdminEdit");
export const N10 = dataStore<{ readonly entries: readonly SavedEntry[] }>("N.N10.SerioBrand", { defaults: any });
export const N12 = config<{ readonly a: number } | { readonly b: string }>("N.N12.NoDiscriminant", any);
export const N21 = dataStore<{ readonly key: ServerOnly<string> }>("N.N21.ServerOnlyRecord", { defaults: any });
