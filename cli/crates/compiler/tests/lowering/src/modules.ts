import { double, greeting, type Shape } from "./util";
import { quadruple } from "./nested";

export { greeting as hello } from "./util";

export function shout(shape: Shape) {
	return `${greeting} ${quadruple(shape.sides)} ${double(1)}`;
}

export function early() {
	return late();
}

function late() {
	return "late";
}

export namespace Counter {
	let count = 0;
	export function advance() {
		count += 1;
		return count;
	}
}

export function hoistedCall(base: number) {
	let total = base;
	const first = add(1);
	function add(n: number) {
		total += n;
		return total;
	}
	return `${first},${add(2)}`;
}

export function parity(n: number) {
	const result = isEven(n) ? "even" : "odd";
	function isEven(k: number): boolean {
		return k === 0 || isOdd(k - 1);
	}
	function isOdd(k: number): boolean {
		return k !== 0 && isEven(k - 1);
	}
	return result;
}

export function nestedBlock(flag: boolean) {
	let out = "none";
	if (flag) {
		out = describe("nested");
		function describe(label: string) {
			return `${label} ${out}`;
		}
	}
	return out;
}

export function laterConst() {
	const read = () => get();
	function get() {
		return value;
	}
	const value = 5;
	return read();
}

export function overloadedParity(n: number) {
	const result = isEven(n) ? "even" : "odd";
	function isEven(k: number): boolean {
		return k === 0 || isOdd(k - 1);
	}
	function isOdd(k: number): boolean;
	function isOdd(k: number): boolean {
		return k !== 0 && isEven(k - 1);
	}
	return result;
}
