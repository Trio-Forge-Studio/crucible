declare function trace(value: unknown, shape: string): void;
function id<T>(value: T): T {
	return value;
}
trace(id(-7) % 3, "n");
trace(id(7) % -3, "n");
trace(id(-6) % -3, "n");
