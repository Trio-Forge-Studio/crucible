declare function trace(value: unknown, shape: string): void;

function look(xs: number[]): number {
	trace(xs, "[n]");
	return 9;
}

const xs: number[] = [];
xs.push(0, look(xs));
trace(xs, "[n]");

function say(n: number): number {
	trace(n, "n");
	return n;
}

const ys: number[] = [3];
const length = ys.unshift(say(1), say(2));
trace(length, "n");
trace(ys, "[n]");

let target = xs;
let value = 4;
function redirect(): number {
	target = ys;
	value = 8;
	return 7;
}

const pushed = target.push(value, redirect(), value++);
trace(pushed, "n");
trace(xs, "[n]");
trace(ys, "[n]");
trace(value, "n");

target = xs;
const redirected = target.unshift(value, redirect());
trace(redirected, "n");
trace(xs, "[n]");
trace(ys, "[n]");

target = xs;
target.push(redirect());
trace(xs, "[n]");
trace(ys, "[n]");
