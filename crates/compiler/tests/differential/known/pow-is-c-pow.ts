declare function trace(value: unknown, shape: string): void;
function id<T>(value: T): T {
	return value;
}
trace(id(-0) ** 0.5, "n");
trace(id(-0.30000000000000004) ** 3, "n");
trace(id(1) ** (0 / 0), "n");
