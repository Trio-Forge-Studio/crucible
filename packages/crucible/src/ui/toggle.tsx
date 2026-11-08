import Vide, { effect } from "@rbxts/vide";
import { Attribute } from "./attributes";
import { buttonHost, frameHost, partContext, type AsChildProps, type ButtonPartProps, type PartContext } from "./part";
import { useToggle, type ToggleBinding, type ToggleOptions } from "./use-toggle";

export type ToggleRootProps = ToggleOptions & ButtonPartProps<ToggleBinding>;
export type ToggleIndicatorProps = AsChildProps<Frame>;
const options = ["checked", "defaultChecked", "onCheckedChange", "disabled", "selectFirst", "silent"];
const checkboxContext = partContext<ToggleBinding>("Checkbox.Root");
const switchContext = partContext<ToggleBinding>("Switch.Root");

function root(context: PartContext<ToggleBinding>, props: ToggleRootProps, part: string): Vide.Node {
	const toggle = useToggle(props);
	return context.provide(toggle, () => buttonHost(props, part, options, toggle.action, toggle));
}
function mark(toggle: ToggleBinding, instance: GuiObject): void {
	effect(() => instance.SetAttribute(Attribute.Checked, toggle.checked()));
}

export const Checkbox = {
	Root: (props: ToggleRootProps) => root(checkboxContext, props, "Checkbox.Root"),
	// Shown only while checked, so the game draws the check mark inside it.
	Indicator: (props: ToggleIndicatorProps) => {
		const toggle = checkboxContext.use("Checkbox.Indicator");
		return frameHost(
			props,
			"Checkbox.Indicator",
			[],
			() => props.children,
			(instance) => {
				mark(toggle, instance);
				effect(() => {
					instance.Visible = toggle.checked();
				});
			},
		);
	},
};
export const Switch = {
	Root: (props: ToggleRootProps) => root(switchContext, props, "Switch.Root"),
	Thumb: (props: ToggleIndicatorProps) => {
		const toggle = switchContext.use("Switch.Thumb");
		return frameHost(
			props,
			"Switch.Thumb",
			[],
			() => props.children,
			(instance) => mark(toggle, instance),
		);
	},
};
