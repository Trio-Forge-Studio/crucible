import { cleanup, derive, effect, read, source, untrack } from "@rbxts/vide";
import { adapter } from "./adapter";
import { Attribute } from "./attributes";
import { heartbeat } from "./frame-loop";
import type { Value } from "./props";
import { menuOpen, textEditing, useSelection } from "./runtime";
import { fractionOf, holdStart, SliderCommit, snapValue, tickRepeat, validateRange, valueAt } from "./slider-state";
import { useControllable } from "./use-controllable";

const UserInputService = game.GetService("UserInputService");

export interface SliderOptions {
	value?: Value<number>;
	defaultValue?: number;
	min?: number;
	max: number;
	step?: number;
	onValueChange?: (value: number) => void;
	onValueCommit?: (value: number) => void;
	disabled?: Value<boolean>;
	selectFirst?: Value<boolean>;
	silent?: boolean;
}
export interface SliderBinding {
	value: () => number;
	fraction: () => number;
	dragging: () => boolean;
	highlighted: () => boolean;
	control: (instance: GuiButton) => void;
	track: (instance: GuiObject) => void;
}
interface Gesture {
	readonly kind: "Drag" | "Hold";
	readonly input: InputObject;
	readonly stop: (() => void)[];
}
const directions = new Map<Enum.KeyCode, number>([
	[Enum.KeyCode.DPadLeft, -1],
	[Enum.KeyCode.DPadRight, 1],
	[Enum.KeyCode.Left, -1],
	[Enum.KeyCode.Right, 1],
]);
const stickDeadzone = 0.4;
const stickThreshold = 0.55;

function stickHeld(): boolean {
	return UserInputService.GetConnectedGamepads().some((gamepad) =>
		UserInputService.GetGamepadState(gamepad).some(
			(input) => input.KeyCode === Enum.KeyCode.Thumbstick1 && input.Position.Magnitude > stickDeadzone,
		),
	);
}

export function useSlider(options: SliderOptions): SliderBinding {
	const host = adapter();
	const range = { min: options.min ?? 0, max: options.max, step: options.step ?? 1 };
	validateRange(range);
	const [raw, setRaw] = useControllable({
		value: options.value,
		defaultValue: snapValue(options.defaultValue ?? range.min, range),
		onChange: options.onValueChange,
	});
	const value = () => snapValue(raw(), range);
	const disabled = () => read(options.disabled ?? false);
	const selection = useSelection({ selectFirst: options.selectFirst });
	const blocked = () => !selection.allowed() || menuOpen() || textEditing();
	const hovered = source(false);
	const dragging = source(false);
	const track = source<GuiObject | undefined>(undefined);
	const commit = new SliderCommit();
	let gesture: Gesture | undefined;

	const finish = () =>
		untrack(() => {
			const ended = gesture;
			gesture = undefined;
			dragging(false);
			if (!ended) return;
			for (const stop of ended.stop) stop();
			const committed = commit.release();
			if (committed !== undefined) options.onValueCommit?.(committed);
		});
	const set = (requested: number) =>
		untrack(() => {
			const snapped = snapValue(requested, range);
			if (snapped === value()) return;
			commit.change(snapped);
			setRaw(snapped);
		});
	const begin = (kind: Gesture["kind"], input: InputObject): Gesture | undefined => {
		if (disabled()) {
			if (!options.silent) host.feedback("Refused");
			return;
		}
		const started: Gesture = { kind, input, stop: [] };
		gesture = started;
		commit.begin();
		return started;
	};
	const point = (x: number) => {
		const bar = untrack(track);
		if (bar && bar.AbsoluteSize.X > 0) set(valueAt((x - bar.AbsolutePosition.X) / bar.AbsoluteSize.X, range));
	};
	const beginDrag = (input: InputObject) => {
		const mouse = input.UserInputType === Enum.UserInputType.MouseButton1;
		if (!mouse && input.UserInputType !== Enum.UserInputType.Touch) return;
		if (gesture || untrack(blocked)) return;
		const started = begin("Drag", input);
		if (!started) return;
		dragging(true);
		point(input.Position.X);
		const moved = UserInputService.InputChanged.Connect((changed) => {
			if (mouse ? changed.UserInputType === Enum.UserInputType.MouseMovement : changed === input)
				point(changed.Position.X);
		});
		const ended = UserInputService.InputEnded.Connect((changed) => {
			if (changed === input) finish();
		});
		started.stop.push(
			() => moved.Disconnect(),
			() => ended.Disconnect(),
		);
	};
	const beginHold = (input: InputObject, direction: number) => {
		finish();
		const started = begin("Hold", input);
		if (!started) return;
		const nudge = () => set(untrack(value) + direction * range.step);
		nudge();
		let schedule = holdStart;
		started.stop.push(
			heartbeat((_, delta) => {
				const result = tickRepeat(schedule, delta);
				schedule = result.state;
				if (result.fire) nudge();
			}),
		);
	};

	effect(() => {
		if (blocked() || disabled()) finish();
	});
	const listening = derive(() => selection.selected() && !blocked());
	effect(() => {
		if (!listening()) return;
		let stickDirection = 0;
		let stickReady = !stickHeld();
		cleanup(
			UserInputService.InputBegan.Connect((input) => {
				const direction = directions.get(input.KeyCode);
				if (direction !== undefined) beginHold(input, direction);
			}),
		);
		cleanup(
			UserInputService.InputChanged.Connect((input) => {
				if (input.KeyCode !== Enum.KeyCode.Thumbstick1) return;
				const position = input.Position;
				if (position.Magnitude < stickDeadzone) stickReady = true;
				if (!stickReady) return;
				const horizontal = math.abs(position.X) > stickThreshold && math.abs(position.X) > math.abs(position.Y);
				const direction = horizontal ? math.sign(position.X) : 0;
				if (direction === stickDirection) return;
				stickDirection = direction;
				if (direction !== 0) beginHold(input, direction);
				else if (gesture?.input.KeyCode === Enum.KeyCode.Thumbstick1) finish();
			}),
		);
		cleanup(
			UserInputService.InputEnded.Connect((input) => {
				if (
					gesture?.kind === "Hold" &&
					gesture.input.KeyCode === input.KeyCode &&
					gesture.input.UserInputType === input.UserInputType
				)
					finish();
			}),
		);
		cleanup(() => {
			if (gesture?.kind === "Hold") finish();
		});
	});
	cleanup(UserInputService.WindowFocusReleased.Connect(finish));
	cleanup(
		UserInputService.GamepadDisconnected.Connect((gamepad) => {
			if (gesture?.input.UserInputType === gamepad) finish();
		}),
	);
	cleanup(finish);

	const focused = derive(() => host.device() !== "Touch" && (hovered() || selection.selected()));
	const highlighted = derive(() => dragging() || focused());
	let focusPlayed = false;
	effect(() => {
		const on = focused();
		if (on && !focusPlayed && !options.silent) host.feedback("Focus");
		focusPlayed = on;
	});
	return {
		value,
		fraction: () => fractionOf(value(), range),
		dragging: () => dragging(),
		highlighted,
		control: (instance) => {
			selection.action(instance);
			// Pinned so native selection never takes the horizontal input the slider adjusts with.
			instance.NextSelectionLeft = instance;
			instance.NextSelectionRight = instance;
			effect(() => instance.SetAttribute(Attribute.Highlighted, highlighted()));
			effect(() => instance.SetAttribute(Attribute.Dragging, dragging()));
			effect(() => instance.SetAttribute(Attribute.Disabled, disabled()));
			cleanup(instance.MouseEnter.Connect(() => hovered(true)));
			cleanup(instance.MouseLeave.Connect(() => hovered(false)));
			cleanup(instance.InputBegan.Connect(beginDrag));
		},
		track: (instance) => {
			track(instance);
			cleanup(instance.InputBegan.Connect(beginDrag));
			cleanup(() => {
				if (untrack(track) === instance) track(undefined);
			});
		},
	};
}
