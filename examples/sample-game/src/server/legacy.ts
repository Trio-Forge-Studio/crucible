import { Subsystem, type JsonRecord } from "@trioforge/crucible";
import { PlayerData } from "../shared/profile";

/**
 * Brings in saves from before Crucible: `player_<UserId>` in `Sample.Legacy`, each save at schema version 0, so the
 * `fromGold` migration turns its gold into coins. It lives outside `systems`, so the sample's boot leaves it out: the
 * sample has no legacy saves, and the legacy import specs load it.
 */
export class LegacyImport extends Subsystem {
	private readonly legacy = PlayerData.importLegacy((player, stored) => {
		if (stored !== undefined) return undefined;
		const store = game.GetService("DataStoreService").GetDataStore("Sample.Legacy");
		const [old] = store.GetAsync<{ readonly save?: JsonRecord }>(`player_${player.UserId}`);
		return old?.save === undefined ? undefined : { record: old.save, version: 0 };
	});
}
