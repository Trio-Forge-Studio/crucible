import { profileStore } from "@trioforge/crucible";

declare const any: never;

export const N1 = profileStore<{ readonly at: Vector3 }>("N.N1.Vector3", { template: any });
export const N2 = profileStore<{ readonly tint: Color3 }>("N.N2.Color3", { template: any });
export const N4 = profileStore<{ readonly owner: Player }>("N.N4.Instance", { template: any });
export const N5 = profileStore<{ readonly blob: unknown }>("N.N5.Unknown", { template: any });
export const N6 = profileStore<{ readonly slots: Record<number, string> }>("N.N6.NumericKeys", { template: any });
interface Tree {
	readonly children: readonly Tree[];
}
export const N11 = profileStore<Tree>("N.N11.Recursive", { template: any });
export const N13 = profileStore<{ readonly ids: readonly (string | undefined)[] }>("N.N13.UndefinedElements", {
	template: any,
});
