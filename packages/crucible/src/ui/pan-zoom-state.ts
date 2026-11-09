export interface PanZoomView {
	readonly pan: Vector2;
	readonly zoom: number;
}
export interface PanZoomInset {
	readonly topLeft: Vector2;
	readonly bottomRight: Vector2;
}

export function containFit(viewport: Vector2, content: Vector2): number {
	return math.min(viewport.X / content.X, viewport.Y / content.Y);
}

// Per axis, content wider than the viewport pans until its edge meets the viewport's edge. Narrower content stays centred.
export function coverPan(pan: Vector2, content: Vector2, scale: number, half: Vector2): Vector2 {
	const limit = content.div(2).sub(half.div(scale)).Max(Vector2.zero);
	return pan.Max(limit.mul(-1)).Min(limit);
}

// The anchor is an offset from the viewport centre, in logical pixels, that stays over the same content point.
export function zoomAround(view: PanZoomView, zoom: number, anchor: Vector2, fit: number): PanZoomView {
	return { pan: view.pan.add(anchor.div(fit * zoom)).sub(anchor.div(fit * view.zoom)), zoom };
}

export function revealPan(
	pan: Vector2,
	offset: Vector2,
	scale: number,
	half: Vector2,
	inset: PanZoomInset,
): Vector2 | undefined {
	const low = inset.topLeft.sub(half);
	const high = half.sub(inset.bottomRight);
	// A viewport smaller than its insets has no safe area, so the point settles midway between them.
	const middle = low.add(high).div(2);
	const wanted = offset.Max(low.Min(middle)).Min(high.Max(middle));
	return wanted === offset ? undefined : pan.add(wanted.sub(offset).div(scale));
}

export function glideStep(pan: Vector2, target: Vector2, delta: number): Vector2 {
	const remaining = target.sub(pan);
	return remaining.Magnitude < 0.5 ? target : pan.add(remaining.mul(1 - math.exp(-9 * delta)));
}

export function stickVector(position: Vector2, deadzone: number): Vector2 {
	const magnitude = position.Magnitude;
	if (magnitude <= deadzone) return Vector2.zero;
	return position.Unit.mul((math.min(magnitude, 1) - deadzone) / (1 - deadzone));
}
