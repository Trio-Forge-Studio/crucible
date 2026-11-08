interface Holder {
	tag: number;
	m?(value: number): number;
	b: { tag: number; m?(value: number): number };
	control: { buttonAUp?: (value: number) => number };
}
export function run(holder: Holder | undefined) {
	let calls = 0;
	const arg = () => {
		calls++;
		return 2;
	};
	const a = holder?.m?.(arg());
	const b = holder?.["m"]?.(arg());
	const c = holder?.b.m?.(arg());
	const d = holder?.control.buttonAUp?.(arg());
	holder?.m?.(arg());
	return [a ?? -1, b ?? -1, c ?? -1, d ?? -1, calls];
}
