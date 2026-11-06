const values: Array<number> = [];
for (let i = 0; i < 1000; i++) {
	values.push(i % 7);
}

export function loop(): number {
	let total = 0;
	for (let pass = 0; pass < 20; pass++) {
		for (let i = 0; i < values.size(); i++) {
			total += values[i] * 2;
		}
	}
	return total;
}
