import { cleanup, derive, effect, read, source, untrack } from "@rbxts/vide";
import { heartbeat } from "./frame-loop";
import { adapter } from "./adapter";
import { Attribute } from "./attributes";
import { menuOpen, pressing, textEditing, useSelection } from "./runtime";
import { idlePress, transition, type PressEvent, type PressMode, type PressState } from "./press-state";
import type { Value } from "./props";

const UserInputService = game.GetService("UserInputService");

export type PressTiming = { hold?: Value<number | undefined>; repeating?: never } | { repeating: true; hold?: never };
export type PressOptions = PressTiming & {
	onPress: () => boolean | void;
	onPressedChange?: (pressed: boolean) => void;
	onHighlightChange?: (highlighted: boolean) => void;
	disabled?: Value<boolean>;
	selectFirst?: Value<boolean>;
	silent?: boolean;
};
export interface PressBinding {
	highlighted: () => boolean;
	pressed: () => boolean;
	progress: () => number;
	action: (instance: GuiButton) => void;
}
export function usePress(options: PressOptions): PressBinding {
	const host = adapter();
	const hovered = source(false);
	const state = source<PressState<InputObject>>(idlePress());
	let timedPress = false;
	const mode = (): PressMode => {
		const hold = read(options.hold);
		if (hold !== undefined) {
			assert(hold > 0 && hold < math.huge, "Hold duration must be finite and positive");
			return { kind: "Hold", seconds: hold };
		}
		return options.repeating ? { kind: "Repeat" } : { kind: "Activate" };
	};
	const fire = () =>
		untrack(() => {
			if (!selection.allowed() || menuOpen()) return;
			const refused = read(options.disabled ?? false) || pressing(selection.object(), options.onPress) === false;
			if (!options.silent) host.feedback(refused ? "Refused" : "Press");
		});
	const update = (event: PressEvent<InputObject>) =>
		untrack(() => {
			const before = state();
			const result = transition(before, mode(), event);
			state(result.state);
			if ((before.source === undefined) !== (result.state.source === undefined))
				options.onPressedChange?.(result.state.source !== undefined);
			if (result.fire) fire();
		});
	const begin = (input: InputObject) =>
		untrack(() => {
			if (!selection.allowed() || menuOpen() || textEditing()) return;
			if (state().source === undefined) timedPress = mode().kind !== "Activate";
			if (read(options.disabled ?? false)) {
				if (!options.silent && mode().kind !== "Activate") host.feedback("Refused");
				return;
			}
			update({ kind: "Begin", source: input });
		});
	const selection = useSelection({
		selectFirst: options.selectFirst,
		buttonADown: options.hold !== undefined || options.repeating ? begin : undefined,
		buttonAUp: (input) => update({ kind: "End", source: input }),
	});
	const highlighted = derive(() => host.device() !== "Touch" && (hovered() || selection.selected()));
	const pressed = derive(() => state().source !== undefined);
	let previouslySelected = false;
	effect(() => {
		const selected = selection.selected();
		if (
			!selection.allowed() ||
			read(options.disabled ?? false) ||
			menuOpen() ||
			textEditing() ||
			(previouslySelected && !selected)
		)
			update({ kind: "Cancel" });
		previouslySelected = selected;
	});
	let focusPlayed = false;
	effect(() => {
		const on = highlighted();
		if (on && !focusPlayed && !options.silent) host.feedback("Focus");
		focusPlayed = on;
		untrack(() => options.onHighlightChange?.(on));
		cleanup(() => {
			if (on) untrack(() => options.onHighlightChange?.(false));
		});
	});
	const ticking = derive(() => pressed() || state().progress > 0);
	effect(() => {
		if (!ticking()) return;
		cleanup(heartbeat((_, delta) => update({ kind: "Tick", delta })));
	});
	cleanup(() => update({ kind: "Cancel" }));
	const pointer = (input: InputObject) =>
		input.UserInputType === Enum.UserInputType.MouseButton1 || input.UserInputType === Enum.UserInputType.Touch;
	return {
		highlighted,
		pressed,
		progress: () => state().progress,
		action: (instance: GuiButton) => {
			selection.action(instance);
			effect(() => instance.SetAttribute(Attribute.Highlighted, highlighted()));
			effect(() => instance.SetAttribute(Attribute.Pressed, pressed()));
			effect(() => instance.SetAttribute(Attribute.Disabled, read(options.disabled ?? false)));
			cleanup(
				instance.Activated.Connect(() => {
					if (!timedPress && mode().kind === "Activate") fire();
				}),
			);
			cleanup(instance.MouseEnter.Connect(() => hovered(true)));
			cleanup(
				instance.MouseLeave.Connect(() => {
					hovered(false);
					update({ kind: "Cancel" });
				}),
			);
			cleanup(
				instance.InputBegan.Connect((input) => {
					if (pointer(input)) begin(input);
				}),
			);
			cleanup(
				UserInputService.InputEnded.Connect((input) => {
					if (pointer(input)) update({ kind: "End", source: input });
				}),
			);
		},
	};
}
