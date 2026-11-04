const WORD = 2 ** 32;
const DOUBLE = 2 ** 53;

export type SeedPart = string | number;
export type RngState = readonly [number, number, number, number];

// Luau has no 32-bit multiply, and a full product of two words overflows a double's 53 bits, so it splits one side.
function imul(a: number, b: number): number {
	const low = a * (b % 65536);
	const high = ((a * math.floor(b / 65536)) % WORD) * 65536;
	return (low + high) % WORD;
}

function fmix(h: number): number {
	h = imul(bit32.bxor(h, bit32.rshift(h, 16)), 0x85ebca6b);
	h = imul(bit32.bxor(h, bit32.rshift(h, 13)), 0xc2b2ae35);
	return bit32.bxor(h, bit32.rshift(h, 16));
}

const STRING = 0x5354524e;
const NUMBER = 0x4e554d42;

function words(parts: readonly SeedPart[]): number[] {
	const result = [parts.size()];
	for (const part of parts) {
		if (typeIs(part, "string")) {
			result.push(STRING, part.size());
			for (let index = 1; index <= part.size(); index += 4) {
				const [a, b, c, d] = string.byte(part, index, index + 3);
				result.push((a ?? 0) + (b ?? 0) * 0x100 + (c ?? 0) * 0x10000 + (d ?? 0) * 0x1000000);
			}
		} else {
			if (part !== part || part % 1 !== 0 || math.abs(part) > DOUBLE)
				error(`a seed number must be an integer within ±2^53, got ${part}`, 3);
			result.push(NUMBER, part % WORD, math.floor(part / WORD) % WORD);
		}
	}
	return result;
}

const LANES = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a];

function hashState(input: readonly number[]): [number, number, number, number] {
	const state = LANES.map((lane) => {
		let h = lane;
		for (const word of input) h = fmix((imul(bit32.bxor(h, word), 0x9e3779b1) + lane) % WORD);
		return fmix(h);
	});
	if (state.every((word) => word === 0)) state[0] = 1;
	return [state[0], state[1], state[2], state[3]];
}

function entropy(): number[] {
	return [
		math.random(0, 0xffff),
		math.random(0, 0xffff),
		math.random(0, 0xffff),
		math.floor(os.clock() * 1e6) % WORD,
	];
}

export class Rng {
	private a: number;
	private b: number;
	private c: number;
	private d: number;

	constructor(...seed: SeedPart[]) {
		[this.a, this.b, this.c, this.d] = hashState(seed.size() > 0 ? words(seed) : entropy());
	}

	static fromState(state: RngState): Rng {
		const rng = new Rng(0);
		for (const word of state)
			if (word % 1 !== 0 || word < 0 || word >= WORD) error(`a state word must be a uint32, got ${word}`, 2);
		if (state.every((word) => word === 0)) error("a state can't be all zeroes", 2);
		[rng.a, rng.b, rng.c, rng.d] = state;
		return rng;
	}

	state(): RngState {
		return [this.a, this.b, this.c, this.d];
	}

	clone(): Rng {
		return Rng.fromState(this.state());
	}

	fork(): Rng {
		return new Rng(this.uint32(), this.uint32(), this.uint32(), this.uint32());
	}

	uint32(): number {
		const result = (bit32.lrotate((this.b * 5) % WORD, 7) * 9) % WORD;
		const t = bit32.lshift(this.b, 9);
		this.c = bit32.bxor(this.c, this.a);
		this.d = bit32.bxor(this.d, this.b);
		this.b = bit32.bxor(this.b, this.c);
		this.a = bit32.bxor(this.a, this.d);
		this.c = bit32.bxor(this.c, t);
		this.d = bit32.lrotate(this.d, 11);
		return result;
	}

	float(): number;
	float(min: number, max: number): number;
	float(min?: number, max?: number): number {
		const unit = (bit32.rshift(this.uint32(), 5) * 2 ** 26 + bit32.rshift(this.uint32(), 6)) / DOUBLE;
		if (min === undefined || max === undefined) return unit;
		if (!(min <= max)) error(`float needs min <= max, got ${min} and ${max}`, 2);
		return min + unit * (max - min);
	}

	int(min: number, max: number): number {
		if (min % 1 !== 0 || max % 1 !== 0) error(`int needs integer bounds, got ${min} and ${max}`, 2);
		const span = max - min + 1;
		if (span < 1) error(`int needs min <= max, got ${min} and ${max}`, 2);
		if (span > WORD) error(`int spans at most 2^32 values, got ${span}`, 2);
		const limit = WORD - (WORD % span);
		let value = this.uint32();
		while (value >= limit) value = this.uint32();
		return min + (value % span);
	}

	chance(probability: number): boolean {
		return this.float() < probability;
	}

	normal(mean = 0, deviation = 1): number {
		const radius = math.sqrt(-2 * math.log(1 - this.float()));
		return mean + deviation * radius * math.cos(2 * math.pi * this.float());
	}

	pick<T extends defined>(items: readonly T[]): T {
		if (items.size() === 0) error("can't pick from an empty array", 2);
		return items[this.int(0, items.size() - 1)];
	}

	weightedIndex(weights: readonly number[]): number {
		let total = 0;
		for (const weight of weights) {
			if (!(weight >= 0 && weight < math.huge))
				error(`a weight must be finite and non-negative, got ${weight}`, 2);
			total += weight;
		}
		if (total <= 0) error("weights must add up to more than zero", 2);
		let roll = this.float() * total;
		let last = 0;
		for (let index = 0; index < weights.size(); index++) {
			if (weights[index] <= 0) continue;
			last = index;
			roll -= weights[index];
			if (roll < 0) return index;
		}
		// Rounding can leave a sliver of the roll past the last bucket.
		return last;
	}

	weighted<T extends defined>(items: readonly T[], weight: (item: T, index: number) => number): T {
		return items[this.weightedIndex(items.map(weight))];
	}

	shuffle<T extends defined>(items: readonly T[]): T[] {
		const result = [...items];
		for (let index = result.size() - 1; index > 0; index--) {
			const other = this.int(0, index);
			[result[index], result[other]] = [result[other], result[index]];
		}
		return result;
	}

	sample<T extends defined>(items: readonly T[], count: number): T[] {
		if (count % 1 !== 0 || count < 0 || count > items.size())
			error(`sample needs a count from 0 to ${items.size()}, got ${count}`, 2);
		const pool = [...items];
		const result: T[] = [];
		for (let index = 0; index < count; index++) {
			const other = this.int(index, pool.size() - 1);
			[pool[index], pool[other]] = [pool[other], pool[index]];
			result.push(pool[index]);
		}
		return result;
	}
}
