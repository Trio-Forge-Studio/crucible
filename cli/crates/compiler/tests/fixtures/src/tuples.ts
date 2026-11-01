export function pair(): LuaTuple<[number, string]> {
	return $tuple(1, "one");
}
const [first, second] = pair();
export const both = [first, second];
for (const [key, value] of pairs(new Map<string, number>())) print(key, value);
