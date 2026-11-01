import { double, Level } from "../util";

export function quadruple(n: number) {
	return double(double(n));
}

export const high = Level.High;
