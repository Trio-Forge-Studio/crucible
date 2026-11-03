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

export function letters(word: string) {
	const out = new Array<string>();
	for (const ch of word) out.push(ch);
	return out.join("-");
}

export function captured() {
	const reads = new Array<() => number>();
	for (let i = 0; i < 3; i++) reads.push(() => i);
	return reads.map((read) => read()).join(",");
}

export function capturedAndChanged() {
	const reads = new Array<() => number>();
	for (let i = 0; i < 6; i++) {
		reads.push(() => i);
		i++;
	}
	return reads.map((read) => read()).join(",");
}

export function capturedWithContinue() {
	const reads = new Array<() => number>();
	for (let i = 0; i < 6; i++) {
		if (i === 1) {
			i = 3;
			continue;
		}
		reads.push(() => i);
	}
	return reads.map((read) => read()).join(",");
}

export function shadowedIncrement() {
	const step = 1;
	let total = 0;
	for (let i = 0; i < 6; i += step) {
		const step = 5;
		total += step;
	}
	return total;
}

export function shadowedCondition() {
	let n = 3;
	let passes = 0;
	do {
		const n = 0;
		passes += 1 + n;
	} while (--n > 0);
	return passes;
}
