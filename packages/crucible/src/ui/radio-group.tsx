import Vide, { effect, read } from "@rbxts/vide";
import { Attribute } from "./attributes";
import { createGroup, type Group, type Orientation } from "./group";
import { buttonHost, frameHost, partContext, type AsChildProps, type ButtonPartProps } from "./part";
import type { Value } from "./props";
import {
	useRadioGroup,
	type RadioGroupBinding,
	type RadioGroupItemBinding,
	type RadioGroupOptions,
} from "./use-radio-group";

interface RadioGroupContext {
	radio: RadioGroupBinding<unknown>;
	group: Group;
	disabled?: Value<boolean>;
}
const context = partContext<RadioGroupContext>("RadioGroup.Root");
const itemContext = partContext<RadioGroupItemBinding>("RadioGroup.Item");

export interface RadioGroupRootProps<T> extends RadioGroupOptions<T> {
	orientation?: Value<Orientation>;
	loop?: Value<boolean>;
	children: (radio: RadioGroupBinding<T>) => Vide.Node;
}
function RadioGroupRoot<T>(props: RadioGroupRootProps<T>): Vide.Node {
	const radio = useRadioGroup<T>(props);
	const group = createGroup({ orientation: props.orientation ?? "Vertical", loop: props.loop });
	return context.provide({ radio: radio as RadioGroupBinding<unknown>, group, disabled: props.disabled }, () =>
		props.children(radio),
	);
}

export type RadioGroupItemProps = {
	value: unknown;
	disabled?: Value<boolean>;
	selectFirst?: Value<boolean>;
} & ButtonPartProps<RadioGroupItemBinding>;
function RadioGroupItem(props: RadioGroupItemProps): Vide.Node {
	const { radio, group, disabled } = context.use("RadioGroup.Item");
	const item = radio.item(props.value, { disabled: props.disabled, selectFirst: props.selectFirst });
	const apply = (instance: GuiButton) => {
		item.action(instance);
		group.item(instance, () => read(disabled ?? false) || read(props.disabled ?? false));
	};
	return itemContext.provide(item, () =>
		buttonHost(props, "RadioGroup.Item", ["value", "disabled", "selectFirst"], apply, item),
	);
}

export type RadioGroupIndicatorProps = AsChildProps<Frame>;
// Shown only while its item is checked, so the game draws the dot inside it.
function RadioGroupIndicator(props: RadioGroupIndicatorProps): Vide.Node {
	const item = itemContext.use("RadioGroup.Indicator");
	return frameHost(
		props,
		"RadioGroup.Indicator",
		[],
		() => props.children,
		(instance) => {
			effect(() => instance.SetAttribute(Attribute.Checked, item.checked()));
			effect(() => {
				instance.Visible = item.checked();
			});
		},
	);
}

export const RadioGroup = { Root: RadioGroupRoot, Item: RadioGroupItem, Indicator: RadioGroupIndicator };
