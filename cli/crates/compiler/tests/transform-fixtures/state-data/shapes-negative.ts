import { replicated } from "@trioforge/crucible";

interface Countdown {
	readonly party: string;
	readonly members: readonly number[];
	readonly endsAt: number;
}

export const N1 = replicated<{ readonly "1": number }>("N.N1", { "1": 1 });
export const N2 = replicated<{ readonly 2: number }>("N.N2", { 2: 1 });
enum Slot {
	A,
	B,
}
export const N3 = replicated<Record<Slot, number>>("N.N3", { [Slot.A]: 1, [Slot.B]: 2 });
export const N4 = replicated<{ readonly "-1": number }>("N.N4", { "-1": 1 });
export const N5 = replicated<{ readonly "2.5": number }>("N.N5", { "2.5": 1 });
export const N6 = replicated<{}>("N.N6", {});
interface Empty {}
export const N7 = replicated<Empty>("N.N7", {});
export const N8 = replicated<Record<never, number>>("N.N8", {});
interface Tree {
	readonly name: string;
	readonly children: readonly Tree[];
}
export const N9 = replicated<Tree>("N.N9", { name: "root", children: [] });
type Json = string | number | boolean | readonly Json[] | { readonly [key: string]: Json };
export const N10 = replicated<Json>("N.N10", "x");
interface Node2 {
	readonly next?: Node2;
}
export const N11 = replicated<Node2>("N.N11", {});
export const N12 = replicated<Readonly<Record<number, Countdown>>>("N.N12", {});
export const N13 = replicated<ReadonlyMap<number, string>>("N.N13", new Map());
export const N14 = replicated<readonly (string | undefined)[]>("N.N14", []);
export const N15 = replicated<readonly [number, string?]>("N.N15", [1]);
declare const part: Part;
export const N16 = replicated<{ readonly door?: Model }>("N.N16", { door: part as unknown as Model });
declare const n17: { readonly by: Player };
export const N17 = replicated<{ readonly by: Player }>("N.N17", n17);
export const N18 = replicated<{ readonly run: () => void }>("N.N18", { run: () => {} });
export const N19 = replicated<unknown>("N.N19", 1);
export const N20 = replicated<any>("N.N20", 1);
export const N21 = replicated<UDim2>("N.N21", new UDim2());
export const N22 = replicated<Enum.KeyCode>("N.N22", Enum.KeyCode.E);
export const N23 = replicated<object>("N.N23", {});
declare const brand: unique symbol;
declare const n24: { readonly [brand]: number; readonly x: number };
export const N24 = replicated<{ readonly [brand]: number; readonly x: number }>("N.N24", n24);
export const N25 = replicated<ReadonlySet<string>>("N.N25", new Set());
export const N26 = replicated<{ readonly a: number } | { readonly a: number; readonly b: string }>("N.N26", { a: 1 });
export const N27 = replicated<{ readonly a: number } & { run(): void }>("N.N27", { a: 1, run() {} });
export const N28 = replicated<ReadonlyMap<string, readonly number[]> | readonly string[]>("N.N28", []);
export function generic<T>(value: T) {
	return replicated<T>("N.N29", value);
}
