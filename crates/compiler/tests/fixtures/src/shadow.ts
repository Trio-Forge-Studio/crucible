function typeIs(value: unknown, name: string) {
	return name === "number" && value !== undefined;
}
export const shadowed = typeIs(1, "number");
