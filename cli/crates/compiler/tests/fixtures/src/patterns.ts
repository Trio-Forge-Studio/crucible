const entries: [string, number][] = [["a", 1]];
export const counts = entries.map(([name, count]) => name.size() + count);
declare const tuple: LuaTuple<[number, string]>;
export const tupled = () => tuple;
const nested: [[number, string], boolean] = [[1, "x"], true];
const [[digit, letter], flag] = nested;
const holder = { pair: [2, "y"] as [number, string] };
const {
	pair: [two, why],
} = holder;
const scores = new Map<string, number>();
let key = "";
let score = 0;
for ([key, score] of scores) print(key, score);
let left = 0;
let right = "";
[left, right] = [3, "w"];
export const all = [digit, letter, flag, two, why, key, score, left, right];
