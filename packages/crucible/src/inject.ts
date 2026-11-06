export function inject<T extends object>(): T {
	error("inject<T>() was not compiled by the Crucible compiler; use crucible build");
}
