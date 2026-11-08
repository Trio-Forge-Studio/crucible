import { atom } from "@rbxts/charm";
import { effect, Logging, receive, Subsystem } from "@trioforge/crucible";
import { Network, publish } from "@trioforge/crucible/server";
import { Collect, Collected } from "../../shared/messages";
import { PlayerData } from "../../shared/profile";
import { TotalCoins } from "../../shared/state";
import { GlobalEvent } from "./event";

const MAX_PICKUP = 5;
const log = Logging.getLogger("Sample.Wallet");

export class Wallet extends Subsystem {
	private readonly total = publish(TotalCoins);
	private readonly multiplier = atom(1);

	private readonly collect = receive(Collect, (player, { size }) => {
		if (size < 1 || size > MAX_PICKUP) return;
		const gained = size * this.multiplier();
		const wallet = PlayerData(player);
		if (!wallet.coins.update((coins) => coins + gained)) return;
		wallet.pickups.update((pickups) => pickups + 1);
		Network.send(player, Collected, { gained });
		log.info("{player} collected {gained}", { player: player.Name, gained });
	});

	protected override awake() {
		effect(() => {
			const event = GlobalEvent();
			const live = event.endsAt === 0 || event.endsAt > os.time();
			const multiplier = live ? event.multiplier : 1;
			this.multiplier(multiplier);
			log.info("coin multiplier {multiplier}", { multiplier });
		});
		effect(() => {
			let sum = 0;
			for (const player of PlayerData.players()) sum += PlayerData(player).coins();
			this.total(sum);
		});
	}
}
