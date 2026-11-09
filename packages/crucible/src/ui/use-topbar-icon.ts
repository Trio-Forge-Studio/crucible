import { cleanup, derive, effect, read, source } from "@rbxts/vide";
import { adapter } from "./adapter";
import { Attribute } from "./attributes";
import { heartbeat } from "./frame-loop";
import type { Value } from "./props";
import { ownerContext, registerTopbarIcon } from "./runtime";
import { useControllable } from "./use-controllable";
import { usePress, type PressBinding } from "./use-press";

const captionDelay = 0.5;

export interface TopbarIconOptions {
	selected?: Value<boolean>;
	defaultSelected?: boolean;
	onSelectedChange?: (selected: boolean) => void;
	disabled?: Value<boolean>;
	notice?: Value<number | boolean>;
}
export interface TopbarIconBinding extends PressBinding {
	selected: () => boolean;
	notice: () => number;
	captionShown: () => boolean;
}
export function useTopbarIcon(options: TopbarIconOptions): TopbarIconBinding {
	const [selected, setSelected] = useControllable({
		value: options.selected,
		defaultValue: options.defaultSelected ?? false,
		onChange: options.onSelectedChange,
	});
	let press!: PressBinding;
	ownerContext("Topbar", () => {
		press = usePress({ onPress: () => setSelected(!selected()), disabled: options.disabled, silent: true });
	});
	const notice = () => {
		const value = read(options.notice ?? 0);
		return typeIs(value, "boolean") ? (value ? 1 : 0) : value;
	};
	const hoverElapsed = source(false);
	effect(() => {
		hoverElapsed(false);
		if (adapter().device() !== "Keyboard" || !press.highlighted()) return;
		let waited = 0;
		const stop = heartbeat((_, delta) => {
			waited += delta;
			if (waited < captionDelay) return;
			stop();
			hoverElapsed(true);
		});
		cleanup(stop);
	});
	return {
		...press,
		selected,
		notice,
		captionShown: derive(() => hoverElapsed() && !selected()),
		action: (instance) => {
			press.action(instance);
			cleanup(registerTopbarIcon(instance, () => read(options.disabled ?? false)));
			effect(() => instance.SetAttribute(Attribute.Selected, selected()));
			effect(() => instance.SetAttribute(Attribute.Notice, notice()));
		},
	};
}
