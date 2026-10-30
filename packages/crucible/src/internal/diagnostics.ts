import { loggingPort } from "./log-output";

let emitting = false;
let terminal = false;

/** @internal */
export function runtimeDiagnostic(level: "warning" | "error", message: string) {
	if (!emitting) {
		emitting = true;
		const [ok] = pcall(() => loggingPort().writeInternal(level, message));
		emitting = false;
		if (ok) return;
	}
	if (terminal) return;
	terminal = true;
	pcall(() => warn(`[Crucible] ${message}`));
	terminal = false;
}
