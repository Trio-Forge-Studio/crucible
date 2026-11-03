function pair(): LuaTuple<[number, string]> {
	return $tuple(1, "one");
}

export function destructure(list: number[], record: { a: number; b?: string }) {
	const [n, word] = pair();
	const [first, , third = 30, ...rest] = list;
	const { a, b: renamed = "default" } = record;
	return [n, word, first, third, rest.size(), a, renamed];
}

export function parameters({ x, y = 2 }: { x: number; y?: number }, [p, q]: [number, number], ...more: number[]) {
	return x + y + p + q + more.size();
}

export function assign(list: number[]) {
	let a = 0;
	let b = 0;
	[a, b] = list;
	const target = { left: 0 };
	[target.left] = pair();
	return [a, b, target.left];
}

function triple(): LuaTuple<[number, number, number]> {
	return $tuple(1, 2, 3);
}

function handlers(): LuaTuple<[() => string, () => string]> {
	return $tuple(
		() => "first",
		() => "second",
	);
}

export function tuples() {
	const values = triple();
	values[0] = 10;
	const calls = handlers();
	return [values[0], values[1], calls[0](), calls?.[1]()];
}
