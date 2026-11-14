// A computed key of a logical or compound assignment runs once. The compiler has always done so, but the generator
// writes only pure keys, so a lowering that evaluated the key twice passed every generated program.
declare function trace(value: unknown, shape: string): void;
let k = 0;
function bump(): number {
	k++;
	return k - 1;
}
const flags: boolean[] = [false, true, false];
flags[bump()] ||= true;
flags[bump()] &&= false;
trace(flags, "[b]");
const maybe: (number | undefined)[] = [1, undefined, 3];
k = 1;
maybe[bump()] ??= 7;
trace(maybe, "[?n]");
const counts: number[] = [0, 0, 0];
k = 0;
counts[bump()] += 5;
counts[bump()] |= 6;
trace((counts[bump()] -= 2), "n");
trace(counts, "[n]");
trace(k, "n");
