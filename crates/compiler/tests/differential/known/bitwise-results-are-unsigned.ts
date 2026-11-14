declare function trace(value: unknown, shape: string): void;
function id<T>(value: T): T {
	return value;
}
trace(id(-1) | 0, "n");
trace(id(1) << 33, "n");
