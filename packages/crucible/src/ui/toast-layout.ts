import { glideStep } from "./pan-zoom-state";

export type ToastPlacement = "TopLeft" | "TopCenter" | "TopRight" | "BottomLeft" | "BottomCenter" | "BottomRight";
export type ToastLayout = "List" | "Deck";
export interface Placement {
	readonly anchor: Vector2;
	readonly stack: 1 | -1;
	readonly swipe: Vector2;
}
export interface ToastTarget {
	readonly position: Vector2;
	readonly scale: number;
	readonly depth: number;
	readonly collapsedHeight: number | undefined;
}
export interface ToastPose {
	readonly position: Vector2;
	readonly scale: number;
}
export interface StackOptions {
	readonly layout: ToastLayout;
	readonly expanded: boolean;
	readonly gap: number;
	readonly peek: number;
	readonly placement: Placement;
}
export interface SwipeSample {
	readonly at: number;
	readonly travel: number;
}

const placements: Record<ToastPlacement, Placement> = {
	TopLeft: { anchor: new Vector2(0, 0), stack: 1, swipe: new Vector2(-1, 0) },
	TopCenter: { anchor: new Vector2(0.5, 0), stack: 1, swipe: new Vector2(0, -1) },
	TopRight: { anchor: new Vector2(1, 0), stack: 1, swipe: new Vector2(1, 0) },
	BottomLeft: { anchor: new Vector2(0, 1), stack: -1, swipe: new Vector2(-1, 0) },
	BottomCenter: { anchor: new Vector2(0.5, 1), stack: -1, swipe: new Vector2(0, 1) },
	BottomRight: { anchor: new Vector2(1, 1), stack: -1, swipe: new Vector2(1, 0) },
};
export function placementOf(placement: ToastPlacement): Placement {
	return placements[placement];
}

export const deckShrink = 0.05;
export const swipeSlop = 8;
export const swipeDistance = 48;
export const swipeSpeed = 600;
const swipeWindow = 0.1;
const swipeMargin = 24;

// Sizes are natural logical sizes, newest toast first. A UIScale shrinks a card around its AnchorPoint, so a card
// behind the front sits where its far edge peeks out past the front card's, and shifts sideways to stay centred.
export function stackTargets(sizes: readonly Vector2[], options: StackOptions): ToastTarget[] {
	const { placement } = options;
	const front = sizes[0]?.Y ?? 0;
	let offset = 0;
	return sizes.map((size, depth) => {
		if (options.layout === "List" || options.expanded || depth === 0) {
			const position = new Vector2(0, placement.stack * offset);
			offset += size.Y + options.gap;
			return { position, scale: 1, depth, collapsedHeight: undefined };
		}
		const scale = 1 - depth * deckShrink;
		const x = (0.5 - placement.anchor.X) * size.X * (1 - scale);
		const y = front + depth * options.peek - front * scale;
		return { position: new Vector2(x, placement.stack * y), scale, depth, collapsedHeight: front };
	});
}

export function stackBounds(
	sizes: readonly Vector2[],
	targets: readonly ToastTarget[],
	placement: Placement,
): { readonly min: Vector2; readonly max: Vector2 } | undefined {
	let min: Vector2 | undefined;
	let max: Vector2 | undefined;
	for (let index = 0; index < targets.size(); index++) {
		const target = targets[index];
		const drawn = new Vector2(sizes[index].X, target.collapsedHeight ?? sizes[index].Y).mul(target.scale);
		const low = target.position.sub(drawn.mul(placement.anchor));
		const high = low.add(drawn);
		min = min ? min.Min(low) : low;
		max = max ? max.Max(high) : high;
	}
	return min && max ? { min, max } : undefined;
}

export function glidePose(pose: ToastPose, target: ToastPose, delta: number): ToastPose {
	const remaining = target.scale - pose.scale;
	return {
		position: glideStep(pose.position, target.position, delta),
		scale: math.abs(remaining) < 1e-3 ? target.scale : pose.scale + remaining * (1 - math.exp(-9 * delta)),
	};
}

// Toward the edge the toast follows the finger. Back toward the screen it moves at most 12 pixels.
export function swipeOffset(travel: number): number {
	return travel >= 0 ? travel : -12 * (1 - 1 / (1 - travel / 40));
}
export function swipeProgress(travel: number): number {
	return math.clamp(travel / swipeDistance, 0, 1);
}
export function swipeVelocity(samples: readonly SwipeSample[], now: number): number {
	const recent = samples.filter((sample) => now - sample.at <= swipeWindow);
	const first = recent[0];
	const last = recent[recent.size() - 1];
	if (!first || !last || last.at <= first.at) return 0;
	return (last.travel - first.travel) / (last.at - first.at);
}
export function swipeDismisses(travel: number, velocity: number): boolean {
	return travel >= swipeDistance || (travel > swipeSlop && velocity >= swipeSpeed);
}
export function swipeExit(placement: Placement, size: Vector2): Vector2 {
	const axis = math.abs(placement.swipe.X) * size.X + math.abs(placement.swipe.Y) * size.Y;
	return placement.swipe.mul(axis + swipeMargin);
}
