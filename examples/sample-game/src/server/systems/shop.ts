import { Subsystem } from "@trioforge/crucible";
import { Products } from "crucible:experience";
import { PlayerData } from "../../shared/profile";

const COIN_PACK = 100;

export class Shop extends Subsystem {
	private readonly coinPack = PlayerData.grant(Products.CoinPack, (_, data) => ({
		...data,
		coins: data.coins + COIN_PACK,
	}));
}
