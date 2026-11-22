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
// How far to move a caption centred under an icon so it stays at least `margin` inside the icon row.
export function captionShift(center: number, width: number, left: number, right: number, margin: number): number {
	const half = width / 2;
	if (center + half > right - margin) return right - margin - (center + half);
	if (center - half < left + margin) return left + margin - (center - half);
	return 0;
}
