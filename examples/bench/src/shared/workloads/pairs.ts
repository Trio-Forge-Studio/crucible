const scores = new Map<string, number>();
for (let i = 0; i < 500; i++) {
	scores.set(`player${i}`, i);
}

export function pairsStep(): number {
	let total = 0;
	for (let pass = 0; pass < 4; pass++) {
		for (const entry of pairs(scores)) {
			total += entry[1];
		}
	}
	return total;
}
