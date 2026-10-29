export function inject<T extends object>(): T {
	error("inject<T>() was not replaced; add the Crucible transformer to the compiler plugins");
}
