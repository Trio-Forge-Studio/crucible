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
