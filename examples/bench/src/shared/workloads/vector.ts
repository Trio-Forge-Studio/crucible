const points: Array<Vector3> = [];
for (let i = 0; i < 1000; i++) {
	points.push(new Vector3(i % 13, i % 7, i % 5));
}

export function vector(): number {
	let sum = Vector3.zero;
	let dot = 0;
	for (const point of points) {
		sum = sum.add(point.mul(0.5)).sub(point.div(4));
		dot += point.Dot(sum);
	}
	return sum.Magnitude + dot;
}
