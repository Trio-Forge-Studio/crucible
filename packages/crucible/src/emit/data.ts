import type { Migration } from "../data";
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
