export interface SliderRange {
	min: number;
	max: number;
	step: number;
}
function finite(value: number): boolean {
	return value === value && math.abs(value) < math.huge;
}
export function validateRange(range: SliderRange): void {
	assert(
		finite(range.min) && finite(range.max) && range.min <= range.max,
		"Slider bounds must be finite and ordered",
	);
	assert(finite(range.step) && range.step > 0, "Slider step must be finite and positive");
}
export function snapValue(value: number, range: SliderRange): number {
	assert(finite(value), "Slider value must be finite");
	const clamped = math.clamp(value, range.min, range.max);
	if (clamped === range.max) return range.max;
	return math.clamp(range.min + math.round((clamped - range.min) / range.step) * range.step, range.min, range.max);
}
export function fractionOf(value: number, range: SliderRange): number {
	return range.max === range.min ? 0 : (value - range.min) / (range.max - range.min);
}
export function valueAt(fraction: number, range: SliderRange): number {
	return snapValue(range.min + math.clamp(fraction, 0, 1) * (range.max - range.min), range);
}

const repeatDelay = 0.4;
export interface HoldRepeat {
	readonly elapsed: number;
	readonly due: number;
}
export const holdStart: HoldRepeat = { elapsed: 0, due: repeatDelay };
// At most one step per frame, so a long frame never jumps several steps.
export function tickRepeat(hold: HoldRepeat, delta: number): { state: HoldRepeat; fire: boolean } {
	const elapsed = hold.elapsed + delta;
	if (elapsed < hold.due) return { state: { elapsed, due: hold.due }, fire: false };
	return { state: { elapsed, due: elapsed + math.max(0.04, 0.15 - (elapsed - repeatDelay) * 0.04) }, fire: true };
}

export class SliderCommit {
	private changed?: number;
	begin(): void {
		this.changed = undefined;
	}
	change(value: number): void {
		this.changed = value;
	}
	release(): number | undefined {
		const value = this.changed;
		this.changed = undefined;
		return value;
	}
}
