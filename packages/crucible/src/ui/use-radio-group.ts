import { effect, read } from "@rbxts/vide";
import { Attribute } from "./attributes";
import type { Value } from "./props";
import { useControllable } from "./use-controllable";
import { usePress, type PressBinding } from "./use-press";

export interface RadioGroupOptions<T> {
	value?: Value<T>;
	defaultValue?: T;
	onValueChange?: (value: T) => void;
	disabled?: Value<boolean>;
}
export interface RadioGroupItemOptions {
	disabled?: Value<boolean>;
	selectFirst?: Value<boolean>;
}
export interface RadioGroupItemBinding extends PressBinding {
	checked: () => boolean;
}
export interface RadioGroupBinding<T> {
	value: () => T | undefined;
	item: (value: T, options?: RadioGroupItemOptions) => RadioGroupItemBinding;
}
// Items check on press only: Roblox's gamepad navigation passes through a group on its way to other controls.
export function useRadioGroup<T>(options: RadioGroupOptions<T> = {}): RadioGroupBinding<T> {
	const [value, setValue] = useControllable<T | undefined>({
		value: options.value,
		defaultValue: options.defaultValue,
		onChange: (checked) => {
			if (checked !== undefined) options.onValueChange?.(checked);
		},
	});
	return {
		value,
		item: (key, item = {}) => {
			const checked = () => value() === key;
			const press = usePress({
				disabled: () => read(options.disabled ?? false) || read(item.disabled ?? false),
				selectFirst: item.selectFirst,
				onPress: () => setValue(key),
			});
			return {
				...press,
				checked,
				action: (instance) => {
					press.action(instance);
					effect(() => instance.SetAttribute(Attribute.Checked, checked()));
				},
			};
		},
	};
}
