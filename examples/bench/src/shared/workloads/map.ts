const source: Array<number> = [];
for (let i = 0; i < 2000; i++) {
	source.push(i);
}

export function map(): number {
	const mapped = source.map((value) => value * 2 + 1);
	let total = 0;
	for (const value of mapped) {
		total += value;
	}
	return total;
}
