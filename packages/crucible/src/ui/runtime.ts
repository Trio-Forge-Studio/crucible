import { atom, type Getter } from "@rbxts/charm";
import { cleanup, context, derive, effect, read, root, source, untrack } from "@rbxts/vide";
import { adapter, type AnvilAdapter } from "./adapter";
import { Attribute } from "./attributes";
import type { Value } from "./props";
import { enter, hasModal, leave, top, type ModalState } from "./modal-state";
import { decide, emptySelection, type Selection, type SelectionEvent } from "./selection-policy";
import { pickTopbarIcon } from "./topbar-policy";

const GuiService = game.GetService("GuiService");
const UserInputService = game.GetService("UserInputService");

export type Owner = number | "Base" | "Topbar";
export const ownerContext = context<Owner>("Base");
export const closeFallbackContext = context(false);
// Set inside a toast to a getter that is true while the toast takes no input. Anvil never selects a toast control for
// the player, but native navigation still reaches it.
export const toastContext = context<(() => boolean) | undefined>(undefined);
const state = source<ModalState<number>>({ entries: [] });
// Never destroyed, so module state stays derived for the life of the realm, inside or outside any Vide scope.
const [, derived] = root(() => ({
	modalShown: derive(() => hasModal(state())),
	topId: derive(() => top(state())?.id),
}));
const modalShown = derived.modalShown;
const modalAtom = atom(false);
// Gameplay computeds track this atom. Anvil tracks the Vide derive, because a Charm batch delays the atom's subscribers.
export const modalOpen: Getter<boolean> = () => modalAtom();
function setState(value: ModalState<number>): void {
	state(value);
	modalAtom(hasModal(untrack(state)));
}
const topId = derived.topId;
export const selectedObject = source<GuiObject | undefined>(undefined);
export const textEditing = source(false);
export const menuOpen = source(false);
interface Snapshot {
	readonly object: GuiObject;
	readonly owner: Owner;
}
interface ScopeRecord {
	readonly id: number;
	readonly mode: "Native" | "Pointer";
	readonly onClose?: () => void;
	restore?: Snapshot;
	root?: GuiObject;
	close?: GuiObject;
	selection: Selection<GuiObject>;
	stopAttachment?: () => void;
}
interface Control {
	readonly owner: Owner;
	readonly toast: boolean;
	readonly buttonADown?: (input: InputObject) => void;
	readonly buttonAUp?: (input: InputObject) => void;
}
const scopes = new Map<number, ScopeRecord>();
const controls = new Map<GuiObject, Control>();
const topbarIcons = new Map<GuiObject, () => boolean>();
let lastTopbarIcon: GuiObject | undefined;
let opener: GuiObject | undefined;
let serial = 0;
let writing = false;
let generation = 0;
let waiting: RBXScriptConnection[] = [];
let selectedWatch: RBXScriptConnection[] = [];
let capturedA: { object: GuiObject; input: InputObject; control: Control } | undefined;
const runtimeOwners = new Set<object>();

// The primitives draw the hover/selection geometry; blank only kit controls so TopbarPlus keeps its artwork.
const blankSelectionImage = new Instance("Frame");
blankSelectionImage.Name = "SelectionImage";
blankSelectionImage.BackgroundTransparency = 1;

// The top bar draws above every modal, so its icons must stay pressable to close the modal they opened.
export function active(owner: Owner): boolean {
	if (owner === "Topbar") return true;
	return owner === "Base" ? !modalShown() : topId() === owner;
}
function native(owner: Owner): boolean {
	if (owner === "Topbar") return !modalShown();
	return active(owner) && (owner === "Base" || scopes.get(owner)?.mode === "Native");
}
function finite(value: number): boolean {
	return value === value && math.abs(value) < math.huge;
}
export function alive(object: GuiObject): boolean {
	if (!object.FindFirstAncestorOfClass("PlayerGui")) return false;
	let ancestor: Instance | undefined = object;
	while (ancestor) {
		if (ancestor.IsA("GuiObject") && !ancestor.Visible) return false;
		if (ancestor.IsA("LayerCollector") && !ancestor.Enabled) return false;
		ancestor = ancestor.Parent;
	}
	return true;
}
function available(object: GuiObject): boolean {
	return alive(object) && object.GetAttribute(Attribute.Disabled) !== true;
}
function ready(object: GuiObject, isRoot: boolean): boolean {
	if ((!isRoot && !object.Selectable) || !alive(object)) return false;
	let ancestor: Instance | undefined = object;
	while (ancestor) {
		if (ancestor.IsA("GuiObject")) {
			const size = ancestor.AbsoluteSize;
			const position = ancestor.AbsolutePosition;
			if (
				!finite(size.X) ||
				!finite(size.Y) ||
				!finite(position.X) ||
				!finite(position.Y) ||
				size.X < 0 ||
				size.Y < 0
			)
				return false;
			if ((ancestor === object || ancestor.ClipsDescendants) && (size.X < 1 || size.Y < 1)) return false;
		}
		ancestor = ancestor.Parent;
	}
	return true;
}
function disconnect(connections: RBXScriptConnection[]): void {
	for (const connection of connections) connection.Disconnect();
}
function watch(object: GuiObject, changed: () => void): RBXScriptConnection[] {
	const connections = [
		object.AncestryChanged.Connect(changed),
		object.GetPropertyChangedSignal("Selectable").Connect(changed),
	];
	let ancestor: Instance | undefined = object;
	while (ancestor) {
		if (ancestor.IsA("GuiObject")) {
			connections.push(ancestor.GetPropertyChangedSignal("AbsoluteSize").Connect(changed));
			connections.push(ancestor.GetPropertyChangedSignal("AbsolutePosition").Connect(changed));
			connections.push(ancestor.GetPropertyChangedSignal("Visible").Connect(changed));
			connections.push(ancestor.GetPropertyChangedSignal("ClipsDescendants").Connect(changed));
		} else if (ancestor.IsA("LayerCollector"))
			connections.push(ancestor.GetPropertyChangedSignal("Enabled").Connect(changed));
		ancestor = ancestor.Parent;
	}
	return connections;
}
function cancelRequest(): void {
	generation++;
	disconnect(waiting);
	waiting = [];
}
function writeSelection(object?: GuiObject): void {
	writing = true;
	GuiService.SelectedObject = object;
	selectedObject(object);
	writing = false;
}
function canSelect(owner: Owner): boolean {
	return native(owner) && adapter().device() === "Gamepad" && !menuOpen() && !textEditing();
}
function request(owner: Owner, target: GuiObject | "Root"): void {
	cancelRequest();
	const token = generation;
	const object = target === "Root" ? (typeIs(owner, "number") ? scopes.get(owner)?.root : undefined) : target;
	if (!object) return;
	let queued = false;
	const retry = () => {
		if (token !== generation) return;
		queued = false;
		disconnect(waiting);
		waiting = [];
		if (!canSelect(owner)) return;
		if (target !== "Root" && controls.get(object)?.owner !== owner) return;
		if (ready(object, target === "Root")) {
			if (target === "Root") {
				writing = true;
				GuiService.Select(object);
				selectedObject(GuiService.SelectedObject);
				writing = false;
			} else writeSelection(object);
		} else waiting = watch(object, queue);
	};
	const queue = () => {
		if (!queued) {
			queued = true;
			task.defer(retry);
		}
	};
	queue();
}
// Runtime entry points run from cleanups and signals, often while another Vide effect is active; untrack keeps them from
// subscribing that effect to runtime state.
function dispatch(id: number, event: SelectionEvent<GuiObject>): void {
	untrack(() => decideFor(id, event));
}
function decideFor(id: number, event: SelectionEvent<GuiObject>): void {
	const record = scopes.get(id);
	if (!record) return;
	const current = GuiService.SelectedObject;
	const decision = decide(record.selection, event, {
		current: current && controls.get(current)?.owner === id ? current : undefined,
		close: record.close,
		alive: (object) => controls.get(object)?.owner === id && available(object),
		canSelect: canSelect(id),
	});
	record.selection = decision.selection;
	if (decision.target !== undefined) request(id, decision.target);
}
function cancelA(): void {
	const session = capturedA;
	capturedA = undefined;
	session?.control.buttonAUp?.(session.input);
}
function selectionChanged(): void {
	untrack(followSelection);
}
function followSelection(): void {
	const previous = selectedObject();
	const current = GuiService.SelectedObject;
	selectedObject(current);
	disconnect(selectedWatch);
	selectedWatch = [];
	if (capturedA && capturedA.object !== current) cancelA();
	if (!writing) cancelRequest();
	if (current && topbarIcons.has(current)) lastTopbarIcon = current;
	const control = current && controls.get(current);
	if (current && control && typeIs(control.owner, "number")) {
		if (!control.toast) dispatch(control.owner, { kind: "Selected", object: current });
	} else if (!writing) {
		const previousControl = previous && controls.get(previous);
		const owner = previousControl?.owner;
		const id = topId();
		if (previous && typeIs(owner, "number") && active(owner))
			dispatch(owner, { kind: "Lost", removed: !alive(previous) });
		else if (id !== undefined && ((current && !control) || (previous && !previousControl)))
			dispatch(id, { kind: "Lost", removed: false });
	}
	if (current && control)
		selectedWatch = watch(current, () => {
			if (GuiService.SelectedObject !== current || !active(control.owner) || alive(current)) return;
			writeSelection();
			if (typeIs(control.owner, "number")) dispatch(control.owner, { kind: "Lost", removed: true });
		});
}

// A pointer press can open a scope while gamepad selection sits elsewhere, so the pressed control is the restore target.
export function pressing<R>(object: GuiObject | undefined, run: () => R): R {
	const previous = opener;
	opener = object;
	try {
		return run();
	} finally {
		opener = previous;
	}
}
function snapshot(): Snapshot | undefined {
	const object = opener ?? GuiService.SelectedObject;
	const control = object && controls.get(object);
	if (!object || !control || control.toast) return undefined;
	// The top bar sits outside every scope, so an icon is a restore target only when it opened the scope.
	if (control.owner === "Topbar" && object !== opener) return undefined;
	return { object, owner: control.owner };
}
function restore(target: Snapshot): void {
	const { object, owner } = target;
	if (controls.get(object)?.owner !== owner || !available(object)) return;
	if (typeIs(owner, "number")) dispatch(owner, { kind: "Restore", object });
	else request(owner, object);
}
export function nextScopeId(): number {
	return ++serial;
}
export function openScope(id: number, mode: "Native" | "Pointer", onClose?: () => void, restoreFocus = true): void {
	scopes.set(id, { id, mode, onClose, restore: restoreFocus ? snapshot() : undefined, selection: emptySelection() });
	// Every control outside a trap stops being selectable, so a selection left on one would outlive the trap unowned.
	const selected = GuiService.SelectedObject;
	if (selected && controls.has(selected)) writeSelection();
	setState(enter(state(), { id, serial: id, order: undefined }));
}
export function attachScope(id: number, root: GuiObject): () => void {
	const record = scopes.get(id);
	if (!record) return () => {};
	record.stopAttachment?.();
	record.root = root;
	let orderConnection: RBXScriptConnection | undefined;
	const update = () => {
		if (scopes.get(id) !== record) return;
		orderConnection?.Disconnect();
		const gui = root.FindFirstAncestorWhichIsA("ScreenGui");
		if (gui) orderConnection = gui.GetPropertyChangedSignal("DisplayOrder").Connect(update);
		setState(enter(state(), { id, serial: id, order: gui?.DisplayOrder }));
	};
	const connection = root.AncestryChanged.Connect(update);
	const detach = () => {
		connection.Disconnect();
		orderConnection?.Disconnect();
	};
	record.stopAttachment = detach;
	update();
	return detach;
}
export function closeScope(id: number): void {
	untrack(() => close(id));
}
function close(id: number): void {
	const record = scopes.get(id);
	if (!record) return;
	record.stopAttachment?.();
	const wasTop = topId() === id || topId() === undefined;
	const selected = GuiService.SelectedObject;
	const selectedOwner = selected && controls.get(selected)?.owner;
	const restoring = wasTop && !record.selection.paused && (selected === undefined || selectedOwner === id);
	if (selectedOwner === id) writeSelection();
	scopes.delete(id);
	// A scope opened from inside this one restores where this one would have, so closing out of order still lands home.
	for (const [, other] of scopes) if (other.restore?.owner === id) other.restore = record.restore;
	try {
		setState(leave(state(), id));
	} finally {
		if (restoring && record.restore) restore(record.restore);
	}
}
// An untrapped scope adds no owner. It only puts selection back when it unmounts holding it.
export function holdRestore(): (host?: GuiObject) => void {
	const saved = snapshot();
	return (host) => {
		const selected = GuiService.SelectedObject;
		if (saved && (selected === undefined || (host !== undefined && selected.IsDescendantOf(host))))
			untrack(() => restore(saved));
	};
}
export function registerControl(object: GuiObject, control: Control, fallback = false): () => void {
	controls.set(object, control);
	object.SelectionImageObject = blankSelectionImage;
	const record = typeIs(control.owner, "number") ? scopes.get(control.owner) : undefined;
	if (fallback && record) record.close = object;
	return () => {
		const selected = GuiService.SelectedObject === object;
		// Vide builds a replacement before disposing the selected control; clear before Roblox chooses a neighbour.
		if (selected) writeSelection();
		if (capturedA?.object === object) cancelA();
		controls.delete(object);
		if (record?.close === object) record.close = undefined;
		if (typeIs(control.owner, "number")) {
			dispatch(control.owner, { kind: "Unmarked", object });
			const owner = control.owner;
			if (selected) task.defer(() => dispatch(owner, { kind: "Lost", removed: true }));
		}
	};
}
export function registerTopbarIcon(object: GuiObject, disabled: () => boolean): () => void {
	topbarIcons.set(object, disabled);
	return () => {
		topbarIcons.delete(object);
		if (lastTopbarIcon === object) lastTopbarIcon = undefined;
	};
}
function selectTopbarIcon(): void {
	if (GuiService.SelectedObject !== undefined || !canSelect("Topbar")) return;
	const candidates = [...topbarIcons].map(([object, disabled]) => ({
		object,
		x: object.AbsolutePosition.X,
		available: alive(object) && !disabled(),
	}));
	const target = pickTopbarIcon(candidates, lastTopbarIcon);
	if (target) request("Topbar", target);
}
export function markControl(owner: Owner, object: GuiObject, marked: boolean): void {
	if (!typeIs(owner, "number")) return;
	const record = scopes.get(owner);
	const current = GuiService.SelectedObject;
	const replaces = current && record?.selection.marked.includes(current) ? current : undefined;
	dispatch(owner, marked ? { kind: "Marked", object, replaces } : { kind: "Unmarked", object });
}
export function useSelection(
	options: {
		selectFirst?: Value<boolean>;
		buttonADown?: (input: InputObject) => void;
		buttonAUp?: (input: InputObject) => void;
	} = {},
) {
	const owner = ownerContext();
	const inert = toastContext();
	const fallback = closeFallbackContext() && !inert;
	const object = source<GuiObject>();
	const allowed = () => active(owner) && !inert?.();
	effect(() => {
		const instance = object();
		if (instance) instance.Selectable = native(owner) && !inert?.();
	});
	effect(() => {
		const instance = object();
		if (!instance || inert) return;
		markControl(owner, instance, read(options.selectFirst ?? false));
		cleanup(() => markControl(owner, instance, false));
	});
	return {
		allowed,
		object: () => object(),
		selected: () => object() !== undefined && selectedObject() === object(),
		action: (instance: GuiObject) => {
			const unregister = registerControl(
				instance,
				{ owner, toast: inert !== undefined, buttonADown: options.buttonADown, buttonAUp: options.buttonAUp },
				fallback,
			);
			object(instance);
			cleanup(unregister);
		},
	};
}
export function startMenuRuntime(): () => void {
	const host = adapter();
	const token = {};
	runtimeOwners.add(token);
	if (runtimeOwners.size() === 1) start(host);
	return () => {
		if (runtimeOwners.delete(token) && runtimeOwners.isEmpty()) stop?.();
	};
}
let stop: (() => void) | undefined;
function start(host: AnvilAdapter): void {
	let armed = !host.close.held();
	let previousTop: number | undefined;
	const connections = [
		GuiService.GetPropertyChangedSignal("SelectedObject").Connect(selectionChanged),
		GuiService.GetPropertyChangedSignal("MenuIsOpen").Connect(() => menuOpen(GuiService.MenuIsOpen)),
		UserInputService.TextBoxFocused.Connect(() => textEditing(true)),
		UserInputService.TextBoxFocusReleased.Connect(() =>
			task.defer(() => textEditing(UserInputService.GetFocusedTextBox() !== undefined)),
		),
		UserInputService.InputBegan.Connect((input) => {
			if (input.KeyCode !== Enum.KeyCode.ButtonA || capturedA || textEditing() || menuOpen()) return;
			const object = GuiService.SelectedObject;
			const control = object && controls.get(object);
			if (!object || !control?.buttonADown || !native(control.owner)) return;
			capturedA = { object, input, control };
			control.buttonADown(input);
		}),
		UserInputService.InputBegan.Connect((input) => {
			if (input.KeyCode === Enum.KeyCode.DPadUp) selectTopbarIcon();
		}),
		UserInputService.InputEnded.Connect((input) => {
			if (input.KeyCode === Enum.KeyCode.ButtonA && capturedA?.input.UserInputType === input.UserInputType)
				cancelA();
		}),
		UserInputService.GamepadDisconnected.Connect((gamepad) => {
			if (capturedA?.input.UserInputType === gamepad) cancelA();
			armed = !host.close.held();
		}),
	];
	const stopClose = [
		host.close.onPressed(() => {
			const id = topId();
			const record = id !== undefined ? scopes.get(id) : undefined;
			if (!armed || !record || record.mode !== "Native" || textEditing() || menuOpen()) return;
			armed = false;
			record.onClose?.();
		}),
		host.close.onReleased(() => {
			armed = !host.close.held();
		}),
	];
	menuOpen(GuiService.MenuIsOpen);
	textEditing(UserInputService.GetFocusedTextBox() !== undefined);
	selectionChanged();
	const [stopEffects] = root(() => {
		effect(() => {
			const id = topId();
			cancelRequest();
			cancelA();
			armed = !host.close.held();
			const old = previousTop;
			previousTop = id;
			if (id !== undefined && id !== old)
				task.defer(() => {
					if (topId() === id) dispatch(id, { kind: "Shown" });
				});
		});
		effect(() => {
			const id = topId();
			const editing = textEditing();
			const menu = menuOpen();
			armed = !host.close.held();
			if (editing || menu) {
				cancelRequest();
				cancelA();
			}
			if (id !== undefined && scopes.get(id)?.mode === "Native" && !editing) {
				if (!menu && GuiService.SelectedObject === undefined)
					task.defer(() => {
						if (topId() === id) dispatch(id, { kind: "Shown" });
					});
				cleanup(host.claimMenuInput());
			}
		});
		effect(() => {
			if (host.device() !== "Gamepad") {
				cancelRequest();
				cancelA();
				return;
			}
			const id = topId();
			if (id !== undefined && GuiService.SelectedObject === undefined)
				task.defer(() => {
					if (topId() === id) dispatch(id, { kind: "Shown" });
				});
		});
	});
	stop = () => {
		disconnect(connections);
		for (const dispose of stopClose) dispose();
		stopEffects();
		cancelA();
		cancelRequest();
		disconnect(selectedWatch);
		selectedWatch = [];
		stop = undefined;
	};
}
