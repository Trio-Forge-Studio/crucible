export function numeric(): number {
	let x = 0.5;
	let total = 0;
	for (let i = 0; i < 20000; i++) {
		x = (x * 3.7 + 0.1) % 1;
		total += math.sqrt(x) * x - x / 3;
	}
	return total;
}
