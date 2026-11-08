export type PressMode =
	| { readonly kind: "Activate" }
	| { readonly kind: "Hold"; readonly seconds: number }
	| { readonly kind: "Repeat" };
export interface PressState<Source> {
	readonly source?: Source;
	readonly elapsed: number;
	readonly progress: number;
	readonly fired: boolean;
	readonly nextRepeatAt: number;
	readonly repeatInterval: number;
}
export type PressEvent<Source> =
	| { readonly kind: "Begin"; readonly source: Source }
	| { readonly kind: "Tick"; readonly delta: number }
	| { readonly kind: "End"; readonly source: Source }
	| { readonly kind: "Cancel" };
export interface PressTransition<Source> {
	readonly state: PressState<Source>;
	readonly fire: boolean;
	readonly ended: boolean;
}
export function idlePress<Source>(): PressState<Source> {
	return { elapsed: 0, progress: 0, fired: false, nextRepeatAt: 0.4, repeatInterval: 0.15 };
}
export function transition<Source>(
	state: PressState<Source>,
	mode: PressMode,
	event: PressEvent<Source>,
): PressTransition<Source> {
	if (event.kind === "Begin")
		return state.source !== undefined
			? { state, fire: false, ended: false }
			: { state: { ...idlePress<Source>(), source: event.source }, fire: mode.kind === "Repeat", ended: false };
	if (event.kind === "Cancel" || event.kind === "End") {
		if (event.kind === "End" && event.source !== state.source) return { state, fire: false, ended: false };
		return {
			state: { ...idlePress<Source>(), progress: state.progress },
			fire: false,
			ended: state.source !== undefined,
		};
	}
	const delta = math.max(0, event.delta);
	if (state.source === undefined)
		return { state: { ...state, progress: math.max(0, state.progress - delta / 0.25) }, fire: false, ended: false };
	const elapsed = state.elapsed + delta;
	if (mode.kind === "Hold") {
		const fire = !state.fired && elapsed >= mode.seconds;
		return {
			state: {
				...state,
				elapsed,
				fired: state.fired || fire,
				progress: state.fired || fire ? 0 : math.min(1, elapsed / mode.seconds),
			},
			fire,
			ended: false,
		};
	}
	if (mode.kind === "Repeat" && elapsed >= state.nextRepeatAt)
		return {
			state: {
				...state,
				elapsed,
				nextRepeatAt: elapsed + state.repeatInterval,
				repeatInterval: math.max(0.04, state.repeatInterval * 0.85),
			},
			fire: true,
			ended: false,
		};
	return { state: { ...state, elapsed }, fire: false, ended: false };
}
