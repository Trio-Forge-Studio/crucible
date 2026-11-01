export function sumEvens(limit: number) {
	let total = 0;
	for (let i = 0; i < limit; i++) {
		if (i % 2 === 1) continue;
		total += i;
	}
	return total;
}

export function countDown(from: number) {
	const seen = new Array<number>();
	let n = from;
	while (n > 0) {
		seen.push(n);
		n--;
	}
	do {
		seen.push(n);
		n--;
	} while (n > 0);
	return seen.join(",");
}

export function classify(n: number) {
	switch (n) {
		case 0:
			return "zero";
		case 1:
		case 2:
			return "small";
		case 3: {
			let label = "three";
			label += "!";
			return label;
		}
		default:
			return "big";
	}
}

export function fallthrough(n: number) {
	const steps = new Array<string>();
	switch (n) {
		case 1:
			steps.push("one");
		case 2:
			steps.push("two");
			break;
		default:
			steps.push("other");
	}
	return steps.join(",");
}

export function iterate(list: string[], lookup: Map<string, number>, members: Set<string>) {
	const parts = new Array<string>();
	for (const item of list) parts.push(item);
	for (const [key, value] of lookup) if (key === "a") parts.push(`${key}=${value}`);
	for (const member of members) if (member === "m") parts.push(member);
	return parts.join(" ");
}
