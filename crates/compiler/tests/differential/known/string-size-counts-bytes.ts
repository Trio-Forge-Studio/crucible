declare function trace(value: unknown, shape: string): void;
function id<T>(value: T): T {
	return value;
}
trace(id("é").size(), "n");
