// Type fixture: every accepted form from the approved contract's K1, K16 and R5 probes, against the real package.
import { askServer, ClientReady, receive, toClient, toServer } from "@trioforge/crucible";
import type { AskResult, AskServer, PayloadArgs, ToClient, ToServer, u8 } from "@trioforge/crucible";
import { Network as ClientNetwork } from "@trioforge/crucible/client";
import { Network as ServerNetwork } from "@trioforge/crucible/server";

declare const player: Player;
declare const door: Model;
declare const button: BasePart;
declare const cond: boolean;

export const Analytics = toServer<"Upgrades" | "SkinShop">("Analytics.ScreenOpened");
export const Teleporting = toClient<"Job" | "Warehouse" | undefined>("Loading.Teleporting");
export const Codes = toClient<"Ok" | "Invalid" | "Expired">("Codes.Result");
export const Num = toClient<1 | 2 | 3>("X.Num");
export const Flag = toClient<true | undefined>("X.B");
export const Pick = askServer<"a" | "b", number>("X.Pick");
export const DoorToggle = toServer<{ readonly door: Model; readonly open: boolean }>("Doors.Toggle");
export const StandUp = toServer<undefined>("Seating.Stand");
export const Notify = toClient<{ Kind: "Info" | "Error"; Text: string }>("Notifications.Notify");
export const LiftCall = toServer<{ readonly button: BasePart; readonly expectedStop: u8 }>("Lifts.Call");
export const Tree = askServer<undefined, unknown>("Admin.CommandTree");
export const Prof = askServer<{ readonly userId: number }, { name: string } | undefined>("PlayerCard.Profile", {
	timeout: 30,
});

export function sends() {
	ClientNetwork.send(Analytics, "Upgrades");
	ServerNetwork.send(player, Teleporting, "Job");
	ServerNetwork.send(player, Codes, "Ok");
	ServerNetwork.broadcast(Codes, "Ok");
	ServerNetwork.send(player, Num, 2);
	ServerNetwork.send(player, Flag, true);
	const picked: AskResult<number> = ClientNetwork.ask(Pick, "a");
	ClientNetwork.send(DoorToggle, { door, open: true });
	ClientNetwork.send(StandUp);
	ServerNetwork.send(player, Teleporting, undefined);
	ServerNetwork.send(player, Notify, { Kind: "Info", Text: "Saved" });
	ClientNetwork.send(LiftCall, { button, expectedStop: 3 });
	const tree = ClientNetwork.ask(Tree);
	if (tree.ok) print(tree.value);
	else print(tree.reason);
	return picked;
}

export function relay<P>(m: ToClient<P>, ...p: PayloadArgs<P>) {
	ServerNetwork.broadcast(m, ...p);
}
export function relayTo<P>(who: Player, m: ToClient<P>, ...p: PayloadArgs<P>) {
	return ServerNetwork.send(who, m, ...p);
}
export function forward<Q, R>(m: AskServer<Q, R>, ...q: PayloadArgs<Q>): AskResult<R> {
	return ClientNetwork.ask(m, ...q);
}
export function up<P>(m: ToServer<P>, ...p: PayloadArgs<P>) {
	return ClientNetwork.send(m, ...p);
}

export class Receivers {
	readonly toggle = receive(DoorToggle, (p, { door, open }) => {
		print(p.Name, door.Name, open);
	});
	readonly stand = receive(StandUp, (p) => {
		print(p.Name);
	});
	readonly profile = receive(Prof, (_p, { userId }) => (userId > 0 ? { name: "x" } : undefined));
	readonly tree = receive(Tree, () => undefined);
	readonly treeNothing = receive(Tree, () => {});
	readonly ready = receive(ClientReady, (p) => () => print(p.Name));
	readonly ready2 = receive(ClientReady, (p) => {
		print(p.Name);
	});
	readonly codes = receive(Codes, (result) => {
		print(result);
	});
	readonly pick = receive(Pick, (_p, which) => (which === "a" ? 1 : 2));
	readonly maybe = receive(ClientReady, () => (cond ? () => {} : undefined));
	readonly early = receive(ClientReady, (p) => {
		if (cond) return () => print(p.Name);
	});
}
