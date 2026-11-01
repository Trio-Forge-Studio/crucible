export function isNumber(value: unknown) {
	return typeIs(value, "number");
}

export function checked(count: number) {
	assert(count, "empty");
}
