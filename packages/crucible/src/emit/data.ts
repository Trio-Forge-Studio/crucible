import type { Migration, ProfileMessage, ProfileOptions, ProfileStore } from "../data";
import { declareProfileStore } from "../internal/data/declarations";
import { declareProfileMessage } from "../internal/data/messages";
import type { MigrationStep } from "../internal/data/admission";
import { declareRecords } from "../internal/data/record-declarations";
import type { Node } from "../internal/network/schema";
import type { DataStore, StoreOptions } from "../server/data";

export function dataStore<T extends object>(
	name: string,
	options: StoreOptions<NoInfer<T>>,
	value: Node,
): DataStore<T> {
	return declareRecords<T>(name, options.defaults, options.migrations as readonly Migration[] | undefined, value);
}

export function profileStore<T extends object>(
	name: string,
	options: ProfileOptions<T>,
	persist: Node,
	replica: Node,
	serverOnlyPaths: readonly (readonly string[])[],
): ProfileStore<T> {
	return declareProfileStore(
		name,
		options.template,
		options.migrations as unknown as readonly MigrationStep[] | undefined,
		persist,
		replica,
		serverOnlyPaths,
	) as unknown as ProfileStore<T>;
}

export function profileMessage<P>(name: string, payload: Node): ProfileMessage<P> {
	return declareProfileMessage(name, payload) as unknown as ProfileMessage<P>;
}
