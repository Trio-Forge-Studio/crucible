export async function later() {}

export function* numbers() {}

export const pattern = /ab+/;

export let counter = 0;

export function loose(value: unknown) {
	return value == undefined;
}

export function shadow() {
	const table = 1;
	return table;
}

export function whole(steps: IterableFunction<LuaTuple<number[]>>) {
	// oxlint-disable-next-line roblox-ts/misleading-lua-tuple-checks -- Exercises the unsupported variable-length iterator step.
	for (const step of steps) {
		print(step.size());
	}
}

export function robloxInstance(value: unknown) {
	return value instanceof Instance;
}
export function arrayIn(value: number[]) {
	return 0 in value;
}
export function tupleIn(value: [number, string]) {
	return 0 in value;
}
export function mapIn(value: Map<string, number>) {
	return "a" in value;
}
export function setIn(value: Set<string>) {
	return "a" in value;
}
export function arrayDelete(value: (number | undefined)[]) {
	return delete value[0];
}
