import type { Getter } from "./props";
export type InputDevice = "Gamepad" | "Touch" | "Keyboard";
export type PressFeedback = "Focus" | "Press" | "Refused";
export interface CloseInput {
	readonly held: () => boolean;
	readonly onPressed: (listener: () => void) => () => void;
	readonly onReleased: (listener: () => void) => () => void;
}
export interface AnvilAdapter {
	readonly device: Getter<InputDevice>;
	readonly close: CloseInput;
	readonly claimMenuInput: () => () => void;
	readonly feedback: (kind: PressFeedback) => void;
}
let installed: AnvilAdapter | undefined;
export function installAnvil(adapter: AnvilAdapter): void {
	if (installed === adapter) return;
	assert(installed === undefined, "Anvil already has a different adapter");
	installed = adapter;
}
export function adapter(): AnvilAdapter {
	assert(installed, "Anvil has no adapter. Call installAnvil before rendering Anvil UI.");
	return installed;
}
