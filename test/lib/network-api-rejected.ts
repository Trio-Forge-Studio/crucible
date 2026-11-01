// Type fixture: each statement after the declarations must fail to type-check (rules 6, 16 and 25).
import { askServer, ClientReady, receive, toClient, toServer } from "@trioforge/crucible";
import type { ToServer } from "@trioforge/crucible";
import { Network as ClientNetwork } from "@trioforge/crucible/client";
import { Network as ServerNetwork } from "@trioforge/crucible/server";

declare const player: Player;
declare const door: Model;
declare const wide: ToServer<{ x: number | string }>;
const Analytics = toServer<"Upgrades" | "SkinShop">("Analytics.ScreenOpened");
const Codes = toClient<"Ok" | "Invalid" | "Expired">("Codes.Result");
const Num = toClient<1 | 2 | 3>("X.Num");
const Pick = askServer<"a" | "b", number>("X.Pick");
const DoorToggle = toServer<{ readonly door: Model; readonly open: boolean }>("Doors.Toggle");
const StandUp = toServer<undefined>("Seating.Stand");
const Prof = askServer<{ readonly userId: number }, { name: string } | undefined>("PlayerCard.Profile");
// REJECTED
export const n1 = () => ClientNetwork.send(Analytics, "Nope");
export const n2 = () => ServerNetwork.send(player, Num, 4);
export const n3 = () => ClientNetwork.ask(Pick, "c");
export const n4 = () => ClientNetwork.send(Codes, "Ok");
export const n5 = () => ServerNetwork.send(player, Analytics, "Upgrades");
export const n6 = () => ClientNetwork.send(DoorToggle, { door, open: 1 });
export const n7 = () => ClientNetwork.send(StandUp, undefined);
export const n8: ToServer<{ x: number }> = wide;
export const n9 = () => ClientNetwork.ask(DoorToggle, { door, open: true });
export const n10 = () => ServerNetwork.broadcast(Analytics, "Upgrades");
export class N11 {
	readonly a = receive(Prof, (_p, { userId }) => {
		print(userId);
	});
}
export class N12 {
	readonly b = receive(ClientReady, () => 5);
}
export class N13 {
	readonly c = receive(DoorToggle, (_p, _payload: string) => {});
}
export class N14 {
	readonly d = receive(Codes, (_p: Player, _r) => {});
}
export class N15 {
	readonly e = receive(Pick, () => "one");
}
export class N16 {
	readonly f = receive(5 as unknown as number, () => {});
}
