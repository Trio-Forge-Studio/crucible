class Base {
	readonly marker = 1;
}
class Leaf extends Base {}
export function instances() {
	const leaf = new Leaf();
	return [leaf instanceof Base, leaf instanceof Leaf, {} instanceof Base];
}
export function members() {
	const object: { a?: number; b?: boolean } = { a: 3, b: false };
	const before = "b" in object;
	const removed = delete object.a;
	const again = delete object["a"];
	return [before, removed, again, "a" in object];
}
interface Target {
	value: number;
	run?(n: number): number;
}
export function optional(present: boolean) {
	let visits = 0;
	let args = 0;
	const target: Target = {
		value: 4,
		run(n) {
			return this.value + n;
		},
	};
	if (!present) delete target.run;
	function get() {
		visits++;
		return target;
	}
	function arg() {
		args++;
		return 3;
	}
	const result = get().run?.(arg());
	get().run?.(arg());
	return [result ?? -1, visits, args];
}
export function checked() {
	return ({ value: 7 } satisfies { value: number }).value;
}
export function promise() {
	return new Promise<number>((resolve) => resolve(9));
}

export function pushMissing(value: () => number) {
	const items: number[] = [];
	items.push(value());
	return items.size();
}
