declare function trace(value: unknown, shape: string): void;
function id<T>(value: T): T {
	return value;
}
trace(`${id(-0)}`, "s");
trace(`${id(1e-7)}`, "s");
