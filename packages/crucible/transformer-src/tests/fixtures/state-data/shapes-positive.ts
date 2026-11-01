import { replicated } from "@trioforge/crucible";

interface Score {
	readonly deliveredValue: number;
	readonly damaged: boolean;
}
interface Countdown {
	readonly party: string;
	readonly members: readonly number[];
	readonly endsAt: number;
}

export const P1 = replicated<number | undefined>("P.P1", undefined);
export const P2 = replicated<Score>("P.P2", { deliveredValue: 0, damaged: false });
export const P3 = replicated<ReadonlyMap<string, Countdown>>("P.P3", new Map());
export const P4 = replicated<readonly { readonly id: string; readonly at?: Vector3 }[]>("P.P4", []);
export const P5 = replicated<"walk" | "drive">("P.P5", "walk");
export const P6 = replicated<{ readonly kind: "a"; readonly x: number } | { readonly kind: "b" }>("P.P6", {
	kind: "b",
});
export const P7 = replicated<Record<string, number>>("P.P7", {});
export const P8 = replicated<readonly [number, string]>("P.P8", [1, "a"]);
export const P9 = replicated<{ readonly [key: string]: Countdown }>("P.P9", {});
declare const p10: { readonly cf: CFrame; readonly c: Color3; readonly tag: string };
export const P10 = replicated<{ readonly cf: CFrame; readonly c: Color3; readonly tag: string }>("P.P10", p10);
type L11 = {
	readonly a: {
		readonly b: {
			readonly c: {
				readonly d: {
					readonly e: {
						readonly f: {
							readonly g: { readonly h: { readonly i: { readonly j: { readonly k: number } } } };
						};
					};
				};
			};
		};
	};
};
declare const p11: { readonly y: { readonly z: L11 } };
export const P11 = replicated<{ readonly y: { readonly z: L11 } }>("P.P11", p11);
declare const p12: { readonly "01": number };
export const P12 = replicated<{ readonly "01": number }>("P.P12", p12);
export const P13 = replicated<Readonly<Partial<Record<string, number>>>>("P.P13", {});
export const P14 = replicated<ReadonlyMap<string, number | undefined>>("P.P14", new Map());
export const P15 = replicated<{ readonly a: number } & { readonly b?: string }>("P.P15", { a: 1 });
export const P16 = replicated<Readonly<Record<"Game" | "Music", number>>>("P.P16", { Game: 1, Music: 1 });
export const P17 = replicated<ReadonlyMap<"Money" | "Jobs", number>>("P.P17", new Map());
