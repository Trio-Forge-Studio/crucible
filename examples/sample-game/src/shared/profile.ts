import { profileStore, serverOnly } from "@trioforge/crucible";
import type { Migration } from "@trioforge/crucible";

const fromGold: Migration = (old) => ({ coins: typeIs(old.gold, "number") ? old.gold : 0 });

export const PlayerData = profileStore("Sample.Players", {
	template: { coins: 0, pickups: serverOnly(0) },
	migrations: [fromGold],
});
