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

export function attempt() {
	try {
		counter += 1;
	} catch {}
}

export function check(value: unknown) {
	return value instanceof Instance;
}
