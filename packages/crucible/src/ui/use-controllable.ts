import { read, source, untrack } from "@rbxts/vide";
import type { Getter, Value } from "./props";

export interface ControllableOptions<T> {
	value?: Value<T>;
	defaultValue: T;
	onChange?: (value: T) => void;
}
export function useControllable<T>(options: ControllableOptions<T>): [Getter<T>, (value: T) => void] {
	const controlled = options.value;
	if (controlled !== undefined) {
		const get = () => read(controlled);
		return [
			get,
			(value) => {
				if (value !== untrack(get)) options.onChange?.(value);
			},
		];
	}
	const state = source(options.defaultValue);
	return [
		() => state(),
		(value) => {
			if (value === untrack(state)) return;
			state(value);
			options.onChange?.(value);
		},
	];
}
