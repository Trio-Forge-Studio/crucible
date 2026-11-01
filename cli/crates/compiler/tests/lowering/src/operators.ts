export function truthy(n: number, s: string, flag?: boolean) {
	const parts = new Array<string>();
	if (n) parts.push("n");
	if (s) parts.push("s");
	if (!flag) parts.push("!flag");
	return parts.join(",");
}

export function pick(n: number, fallback: number) {
	return n || fallback;
}

export function both(n: number, other: number) {
	return n && other;
}

export function coalesce(value: number | undefined, fallback: number) {
	return value ?? fallback;
}

export function maybeFalse(value: boolean | undefined) {
	return value ?? true;
}

export function fill(box: { value?: number }) {
	box.value ??= 7;
	return box.value;
}

export function concat(label: string, n: number) {
	let text = label + n;
	text += "!";
	return text + (n + 1);
}

export function increments() {
	let i = 0;
	const a = i++;
	const b = ++i;
	return [a, b, i];
}

export function bits(a: number, b: number) {
	return [a & b, a | b, a << 2];
}

export function chain(record?: { inner?: { value: number; get(): number } }) {
	return [record?.inner?.value, record?.inner?.get()];
}

export function ternary(flag: boolean, n: number) {
	return flag ? n * 2 : n - 1;
}

export function callNonNull(record?: { run?: () => number }) {
	return record?.run!();
}

function show(a: number, b: number) {
	return `${a},${b}`;
}

export function order() {
	let i = 0;
	let s = "a";
	let n = 1;
	n += n = 5;
	return [show(i, i++), s + (s = "b"), n];
}

export function eitherOr(n: number) {
	return (n && "s") || "fallback";
}

export function propertyCompound() {
	const o = { x: 1, y: 1 };
	o.x += o.x = 5;
	const box = { value: 2 };
	box.value *= (box.value = 3) + 1;
	return [o.x, box.value];
}

export function closureOperands() {
	let x = 1;
	const bump = () => {
		x = 5;
		return 1;
	};
	const sum = x + bump();
	x = 1;
	x += bump();
	const compound = x;
	let list = [10, 20];
	const swap = () => {
		list = [30, 40];
		return 0;
	};
	const element = list[swap()];
	list = [10, 20];
	const first = list;
	list[swap()] = 99;
	return [sum, compound, element, first[0], list[0]];
}
