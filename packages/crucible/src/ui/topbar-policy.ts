export interface TopbarCandidate<T> {
	readonly object: T;
	readonly x: number;
	readonly available: boolean;
}
export function pickTopbarIcon<T extends defined>(
	candidates: readonly TopbarCandidate<T>[],
	remembered?: T,
): T | undefined {
	let first: TopbarCandidate<T> | undefined;
	for (const candidate of candidates) {
		if (!candidate.available) continue;
		if (candidate.object === remembered) return remembered;
		if (first === undefined || candidate.x < first.x) first = candidate;
	}
	return first?.object;
}
export interface TopbarLayout {
	readonly gap: number;
	readonly offsetY: number;
	readonly sizeY: number;
}
export function topbarLayout(height: number, insetY: number, tenFoot: boolean): TopbarLayout {
	const legacy = height <= 36;
	return {
		gap: tenFoot ? 10 : legacy ? 12 : insetY - 50,
		offsetY: (tenFoot ? -9 : legacy ? 2 : 0) + (height === 0 ? 13 : 0),
		sizeY: height === 0 ? 50 : -2,
	};
}
