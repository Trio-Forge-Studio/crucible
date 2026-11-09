import { effect } from "@rbxts/vide";
import { Attribute } from "./attributes";
import type { Value } from "./props";
import { useControllable } from "./use-controllable";
import { usePress, type PressBinding } from "./use-press";

export interface ToggleOptions {
	checked?: Value<boolean>;
	defaultChecked?: boolean;
	onCheckedChange?: (checked: boolean) => void;
	disabled?: Value<boolean>;
	selectFirst?: Value<boolean>;
	silent?: boolean;
}
export interface ToggleBinding extends PressBinding {
	checked: () => boolean;
}
export function useToggle(options: ToggleOptions = {}): ToggleBinding {
	const [checked, setChecked] = useControllable({
		value: options.checked,
		defaultValue: options.defaultChecked ?? false,
		onChange: options.onCheckedChange,
	});
	const press = usePress({
		disabled: options.disabled,
		selectFirst: options.selectFirst,
		silent: options.silent,
		onPress: () => setChecked(!checked()),
	});
	return {
		...press,
		checked,
		action: (instance) => {
			press.action(instance);
			effect(() => instance.SetAttribute(Attribute.Checked, checked()));
		},
	};
}
