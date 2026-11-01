export function arrays() {
	const list = [3, 1, 2];
	list.push(4);
	const last = list.pop();
	list.sort((a, b) => a < b);
	const mapped = list.map((v, i) => v * 10 + i);
	const evens = list.filter((v) => v % 2 === 0);
	const found = list.find((v) => v > 1);
	const total = list.reduce((sum, v) => sum + v, 0);
	return [
		list.size(),
		last,
		list.join("-"),
		mapped.join(","),
		evens.size(),
		found,
		list.indexOf(2),
		list.indexOf(9),
		total,
		list.every((v) => v > 0),
		list.some((v) => v > 2),
		list.includes(3),
	];
}

export function maps() {
	const map = new Map<string, number>();
	map.set("a", 1).set("b", 2);
	const had = map.delete("a");
	const missing = map.delete("z");
	const set = new Set(["x", "y"]);
	set.add("z");
	set.delete("x");
	return [map.get("b"), map.has("a"), map.size(), had, missing, set.size(), set.has("y"), set.has("x")];
}

export function strings(text: string) {
	return [text.size(), text.sub(1, 2), text.split(",").size(), typeIs(text, "string"), typeOf(1)];
}
