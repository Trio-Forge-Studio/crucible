declare function trace(value: unknown, shape: string): void;
function look(xs: number[]): number {
	trace(xs, "[n]");
	return 9;
}
const xs: number[] = [];
xs.push(0, look(xs));
function say(n: number): number {
	trace(n, "n");
	return n;
}
const ys: number[] = [];
ys.unshift(say(1), say(2));
