import { Subsystem } from "@trioforge/crucible";
import { PlayerData } from "../shared/profile";

export class StatsImport extends Subsystem {
	private readonly legacy = PlayerData.importLegacy((player, stored) => {
		if (stored === undefined || stored.record.statsImported === true) return undefined;
		const store = game.GetService("DataStoreService").GetDataStore("Sample.Stats");
		const [old] = store.GetAsync<number>(`Pickups_${player.UserId}`);
		if (old === undefined) return undefined;
		const pickups = typeIs(stored.record.pickups, "number") ? stored.record.pickups : 0;
		return {
			record: { ...stored.record, pickups: pickups + old, statsImported: true },
			version: stored.version,
		};
	});
}
