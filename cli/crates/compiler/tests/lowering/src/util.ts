export const greeting = "hi";

export function double(n: number) {
	return n * 2;
}

export const enum Level {
	Low,
	High = 5,
}

export interface Shape {
	readonly sides: number;
}
