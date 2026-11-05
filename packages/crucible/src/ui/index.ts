import { createBinding } from "../internal/reactive/flush";
import { createRootScope } from "../internal/reactive/ownership";
import { computed, onCleanup, own, scope, state, untracked, type Readable, type State } from "../reactive";

export type { EventProps, IntrinsicProps, PropertyProps } from "./jsx-runtime";

declare const slotKind: unique symbol;

/** A part of the tree that changes after it mounts. `Show` and `For` return one. */
export interface Slot {
	readonly [slotKind]: true;
}

/** What a component returns, and what an element takes as a child. */
export type Node = Instance | Slot | readonly Node[] | undefined;

interface SlotFields {
	readonly attach: (parent: Instance) => void;
}

const slotMeta: LuaMetatable<SlotFields> = {};

function slot(attach: (parent: Instance) => void): Slot {
	return setmetatable({ attach }, slotMeta) as unknown as Slot;
}

/** Parents each instance in `node` to `parent`. Compiled JSX calls it for a child that is not an element. */
export function append(parent: Instance, node: Node) {
	if (node === undefined) return;
	if (typeIs(node, "Instance")) node.Parent = parent;
	else if (getmetatable(node) === slotMeta) (node as unknown as SlotFields).attach(parent);
	else for (const child of node as readonly Node[]) append(parent, child);
}

function ownNodes(node: Node) {
	if (node === undefined) return;
	if (typeIs(node, "Instance")) own(node);
	else if (getmetatable(node) !== slotMeta) for (const child of node as readonly Node[]) ownNodes(child);
}

const sinks = new Map<string, (target: Instance, value: unknown) => void>();

function sinkFor(property: string) {
	let sink = sinks.get(property);
	if (sink === undefined) {
		sink = (target, value) => {
			(target as unknown as Record<string, unknown>)[property] = value;
		};
		sinks.set(property, sink);
	}
	return sink;
}

/**
 * Sets `instance[property]` to the value of `readable` now, then again on each reactive flush where it changed, until
 * the current owner ends. Compiled JSX calls it for a prop whose type is a `Readable`. With no owner it throws.
 */
export function bind<T>(instance: Instance, property: string, readable: Readable<T>) {
	const value = untracked(readable);
	(instance as unknown as Record<string, unknown>)[property] = value;
	createBinding(readable, value, sinkFor(property), instance);
}

/**
 * Runs `component` once in a new scope and appends what it returns to `parent`. Returns a cleanup that ends the
 * scope: the instances it returned are destroyed, which disconnects their events, and its bindings and effects end.
 * Inside another owner, it also ends when that owner ends.
 */
export function mount(component: () => Node, parent: Instance): () => void {
	return scope(() => {
		const node = component();
		ownNodes(node);
		append(parent, node);
	});
}

/** A branch that only its dispose ends, whatever owner is current, so a flush can replace it. */
function branch(render: () => Node): [Node, () => void] {
	let node: Node;
	const dispose = createRootScope(() => {
		node = render();
		ownNodes(node);
	});
	return [node, dispose];
}

export interface ShowProps {
	when: Readable<unknown>;
	children: () => Node;
	fallback?: () => Node;
}

/**
 * Mounts `children()` while `when()` is truthy, as JavaScript defines it, and `fallback()` otherwise. The first branch
 * mounts at the call, so the first frame shows it. On a reactive flush where the condition flips, the branch's scope
 * ends and the other branch mounts. The current branch also ends with the owner that called `Show`.
 */
export function Show(props: ShowProps): Node {
	const shown = () => !!props.when();
	let parent: Instance | undefined;
	let node: Node;
	let dispose: (() => void) | undefined;
	const render = (on: boolean) => {
		if (dispose !== undefined) dispose();
		node = undefined;
		dispose = undefined;
		const make = on ? props.children : props.fallback;
		if (make === undefined) return;
		const [made, stop] = branch(make);
		node = made;
		dispose = stop;
		if (parent !== undefined) append(parent, node);
	};
	const baseline = untracked(shown);
	render(baseline);
	onCleanup(() => {
		if (dispose !== undefined) dispose();
	});
	createBinding(shown, baseline, (_: undefined, on: boolean) => render(on), undefined);
	return slot((target) => {
		parent = target;
		append(target, node);
	});
}

interface Row {
	readonly node: Node;
	readonly dispose: () => void;
	readonly index: State<number>;
}

export interface ForProps<T> {
	each: Readable<readonly T[]>;
	children: (item: T, index: Readable<number>) => Node;
}

/**
 * Mounts `children(item, index)` once for each item of `each()`, keyed by the item itself, and `index` counts from 0.
 * On a reactive flush where `each()` returns another array, an item still in it keeps its row and gets its new index,
 * a new item mounts a row, and a removed item's row ends. An array that holds one item twice throws. Every row also
 * ends with the owner that called `For`.
 */
export function For<T>(props: ForProps<T>): Node {
	let parent: Instance | undefined;
	let rows = new Map<T, Row>();
	const update = (list: readonly T[]) => {
		const seen = new Set<T>();
		for (let i = 0; i < list.size(); i++) {
			if (seen.has(list[i])) error(`For: each() holds the same item twice, at index ${i}`, 0);
			seen.add(list[i]);
		}
		const kept = new Map<T, Row>();
		for (let i = 0; i < list.size(); i++) {
			const item = list[i];
			let row = rows.get(item);
			if (row !== undefined) {
				rows.delete(item);
				row.index.set(i);
			} else {
				const index = state(i);
				const [node, dispose] = branch(() => props.children(item, index));
				row = { node, dispose, index };
				if (parent !== undefined) append(parent, node);
			}
			kept.set(item, row);
		}
		for (const [, removed] of rows) removed.dispose();
		rows = kept;
	};
	const baseline = untracked(props.each);
	update(baseline);
	onCleanup(() => {
		for (const [, row] of rows) row.dispose();
	});
	createBinding(props.each, baseline, (_: undefined, list: readonly T[]) => update(list), undefined);
	return slot((target) => {
		parent = target;
		for (const [, row] of rows) append(target, row.node);
	});
}

/** The device the player used last: keyboard and mouse, touch, or a gamepad. */
export type InputMode = "mouse" | "touch" | "gamepad";

/** `landscape` while the viewport is at least as wide as it is tall. */
export type Orientation = "portrait" | "landscape";

/** The client-wide state that the `md:`, `portrait:`, `touch:` and `dark:` class variants read. */
export interface Environment {
	/** `workspace.CurrentCamera.ViewportSize`, following a change of `CurrentCamera`. */
	readonly viewport: Readable<Vector2>;
	readonly orientation: Readable<Orientation>;
	/** From `UserInputService.LastInputTypeChanged`. An input type of no mode keeps the previous mode. */
	readonly input: Readable<InputMode>;
	/** What `dark:` reads. The game sets it, and it starts false. */
	readonly dark: State<boolean>;
}

function modeOf(inputType: Enum.UserInputType): InputMode | undefined {
	switch (inputType) {
		case Enum.UserInputType.Keyboard:
		case Enum.UserInputType.MouseButton1:
		case Enum.UserInputType.MouseButton2:
		case Enum.UserInputType.MouseButton3:
		case Enum.UserInputType.MouseMovement:
		case Enum.UserInputType.MouseWheel:
			return "mouse";
		case Enum.UserInputType.Touch:
			return "touch";
		case Enum.UserInputType.Gamepad1:
		case Enum.UserInputType.Gamepad2:
		case Enum.UserInputType.Gamepad3:
		case Enum.UserInputType.Gamepad4:
		case Enum.UserInputType.Gamepad5:
		case Enum.UserInputType.Gamepad6:
		case Enum.UserInputType.Gamepad7:
		case Enum.UserInputType.Gamepad8:
			return "gamepad";
	}
	return undefined;
}

let viewportSize: State<Vector2> | undefined;
let inputMode: State<InputMode> | undefined;

function start() {
	if (viewportSize !== undefined) return;
	const input = game.GetService("UserInputService");
	const workspace = game.GetService("Workspace");
	viewportSize = state(workspace.CurrentCamera?.ViewportSize ?? Vector2.zero);
	inputMode = state(
		modeOf(input.GetLastInputType()) ??
			(input.TouchEnabled && !input.MouseEnabled
				? "touch"
				: input.GamepadEnabled && !input.KeyboardEnabled
					? "gamepad"
					: "mouse"),
	);
	let resized: RBXScriptConnection | undefined;
	const follow = (camera: Camera | undefined) => {
		resized?.Disconnect();
		resized = camera
			?.GetPropertyChangedSignal("ViewportSize")
			.Connect(() => viewportSize!.set(camera.ViewportSize));
	};
	follow(workspace.CurrentCamera);
	workspace.GetPropertyChangedSignal("CurrentCamera").Connect(() => {
		const camera = workspace.CurrentCamera;
		follow(camera);
		viewportSize!.set(camera?.ViewportSize ?? Vector2.zero);
	});
	input.LastInputTypeChanged.Connect((inputType) => {
		const mode = modeOf(inputType);
		if (mode !== undefined) inputMode!.set(mode);
	});
}

const viewport = () => {
	start();
	return viewportSize!();
};

/**
 * The client's viewport, orientation, input mode and dark flag. The first read of `viewport`, `orientation` or
 * `input` connects to the camera and `UserInputService` for the rest of the session, so a game installs nothing.
 */
export const environment: Environment = {
	viewport,
	orientation: computed(() => {
		const size = viewport();
		return size.X >= size.Y ? "landscape" : "portrait";
	}),
	input: () => {
		start();
		return inputMode!();
	},
	dark: state(false),
};

/** Whether the pointer is over an element, presses it, or it has focus. The `hover:`, `active:` and `focus:` variants read these. */
export interface Interaction {
	readonly hover: Readable<boolean>;
	/** True from a left click or touch that starts on the element until that input ends, wherever it ends. */
	readonly press: Readable<boolean>;
	/** True while the element is the selected GuiObject, or while a TextBox has the keyboard. */
	readonly focus: Readable<boolean>;
}

const interactions = setmetatable(new Map<GuiObject, Interaction>(), { __mode: "k" });

/**
 * The interaction state of `instance`, made on the first call and shared by every later one. It follows the
 * instance's events until the instance is destroyed. A press in progress stops listening for its release when the
 * owner of the first call ends. The first call needs an owner and throws without one.
 */
export function interaction(instance: GuiObject): Interaction {
	const known = interactions.get(instance);
	if (known !== undefined) return known;
	const hover = state(false);
	const press = state(false);
	const focus = state(false);
	instance.MouseEnter.Connect(() => hover.set(true));
	instance.MouseLeave.Connect(() => hover.set(false));
	let pressing: InputObject | undefined;
	let release: RBXScriptConnection | undefined;
	const ended = (input: InputObject) => {
		if (input !== pressing) return;
		pressing = undefined;
		release?.Disconnect();
		release = undefined;
		press.set(false);
	};
	instance.InputBegan.Connect((input) => {
		const kind = input.UserInputType;
		if (pressing !== undefined) return;
		if (kind !== Enum.UserInputType.MouseButton1 && kind !== Enum.UserInputType.Touch) return;
		pressing = input;
		press.set(true);
		release = game.GetService("UserInputService").InputEnded.Connect(ended);
	});
	instance.InputEnded.Connect(ended);
	onCleanup(() => release?.Disconnect());
	let selected = false;
	let typing = false;
	const refocus = () => focus.set(selected || typing);
	instance.SelectionGained.Connect(() => {
		selected = true;
		refocus();
	});
	instance.SelectionLost.Connect(() => {
		selected = false;
		refocus();
	});
	if (instance.IsA("TextBox")) {
		instance.Focused.Connect(() => {
			typing = true;
			refocus();
		});
		instance.FocusLost.Connect(() => {
			typing = false;
			refocus();
		});
	}
	const made: Interaction = { hover, press, focus };
	interactions.set(instance, made);
	return made;
}
