export interface ModalEntry<Id> {
	readonly id: Id;
	readonly order: number | undefined;
	readonly serial: number;
}
export interface ModalState<Id> {
	readonly entries: readonly ModalEntry<Id>[];
}
export function enter<Id>(state: ModalState<Id>, entry: ModalEntry<Id>): ModalState<Id> {
	const existing = state.entries.find((value) => value.id === entry.id);
	if (existing?.order === entry.order && existing !== undefined) return state;
	return {
		entries: existing
			? state.entries.map((value) => (value.id === entry.id ? { ...value, order: entry.order } : value))
			: [...state.entries, entry],
	};
}
export function leave<Id>(state: ModalState<Id>, id: Id): ModalState<Id> {
	if (!state.entries.some((value) => value.id === id)) return state;
	return { entries: state.entries.filter((value) => value.id !== id) };
}
export function top<Id>(state: ModalState<Id>): ModalEntry<Id> | undefined {
	let result: ModalEntry<Id> | undefined;
	for (const entry of state.entries) {
		if (entry.order === undefined) return undefined;
		if (
			!result ||
			entry.order > (result.order ?? 0) ||
			(entry.order === result.order && entry.serial > result.serial)
		)
			result = entry;
	}
	return result;
}
export function hasModal<Id>(state: ModalState<Id>): boolean {
	return !state.entries.isEmpty();
}
