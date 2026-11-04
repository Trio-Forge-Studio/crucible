export type CurveVector = Vector2 | Vector3;

export interface CurveFrame<V extends CurveVector> {
	readonly point: V;
	readonly tangent: V;
	readonly normal: V;
}

export interface CurveProjection<V extends CurveVector> {
	readonly parameter: number;
	readonly point: V;
	readonly distance: number;
	readonly separation: number;
}

export interface CurveOptions<V extends CurveVector> {
	readonly closed?: boolean;
	readonly initialNormal?: V extends Vector3 ? Vector3 : never;
}

export interface CatmullRomOptions<V extends CurveVector> extends CurveOptions<V> {
	readonly alpha?: number;
}

// Arrow properties compile to dot calls, so Luau callers write curve.pointAt(t) and can detach them.
export interface Curve<V extends CurveVector> {
	readonly pointAt: (parameter: number) => V;
	readonly tangentAt: (parameter: number) => V | undefined;
	readonly frameAt: (parameter: number) => CurveFrame<V> | undefined;
	readonly length: () => number;
	readonly parameterAtDistance: (distance: number) => number;
	readonly distanceAtParameter: (parameter: number) => number;
	readonly closest: (query: V) => CurveProjection<V>;
	readonly prepare: () => void;
}

export function catmullRom(points: readonly Vector2[], options?: CatmullRomOptions<Vector2>): Curve<Vector2>;
export function catmullRom(points: readonly Vector3[], options?: CatmullRomOptions<Vector3>): Curve<Vector3>;
export function catmullRom(points: readonly CurveVector[], options: CatmullRomOptions<CurveVector> = {}) {
	const alpha = options.alpha ?? 0.5;
	if (!(alpha >= 0 && alpha <= 1)) error(`a Catmull-Rom alpha must be from 0 to 1, got ${alpha}`);
	return build(points, options, (lifted, closed) => catmullRomSpans(lifted, closed, alpha));
}

export function cubicBezier(points: readonly Vector2[], options?: CurveOptions<Vector2>): Curve<Vector2>;
export function cubicBezier(points: readonly Vector3[], options?: CurveOptions<Vector3>): Curve<Vector3>;
export function cubicBezier(points: readonly CurveVector[], options: CurveOptions<CurveVector> = {}) {
	return build(points, options, bezierSpans);
}

export function uniformBSpline(points: readonly Vector2[], options?: CurveOptions<Vector2>): Curve<Vector2>;
export function uniformBSpline(points: readonly Vector3[], options?: CurveOptions<Vector3>): Curve<Vector3>;
export function uniformBSpline(points: readonly CurveVector[], options: CurveOptions<CurveVector> = {}) {
	return build(points, options, bSplineSpans);
}

type Span = readonly [Vector3, Vector3, Vector3, Vector3];
type Chain = readonly [Span, ...Span[]];
// A span's squared speed as a polynomial in u, highest power first.
type Quartic = readonly [number, number, number, number, number];

interface Geometry {
	readonly spans: Chain;
	readonly closed: boolean;
	readonly planar: boolean;
	readonly initialNormal?: Vector3;
	readonly tolerance: number;
}

interface ArcSpan {
	readonly stops: readonly number[];
	readonly distances: readonly number[];
	readonly speeds: readonly number[];
}

interface ArcTable {
	readonly quartics: readonly Quartic[];
	readonly spans: readonly ArcSpan[];
	readonly ends: readonly number[];
	readonly length: number;
}

interface Station {
	readonly index: number;
	readonly u: number;
	readonly point: Vector3;
	readonly tangent: Vector3;
	readonly normal: Vector3;
}

interface FrameTable {
	readonly positions: readonly number[];
	readonly stations: readonly Station[];
	readonly seamRoll: number;
	readonly quartics: readonly Quartic[];
	readonly distances: readonly number[];
}

interface Cluster {
	readonly low: Vector3;
	readonly high: Vector3;
	readonly index: number;
	readonly children?: readonly [Cluster, Cluster];
}

interface Branch {
	readonly bound: number;
	readonly cluster: Cluster;
}

interface Piece {
	readonly cluster?: undefined;
	readonly index: number;
	readonly controls: Span;
	readonly from: number;
	readonly to: number;
	readonly bound: number;
}

interface Candidate {
	readonly index: number;
	readonly u: number;
	readonly separation: number;
}

const UP = new Vector3(0, 1, 0);
const AXES = [new Vector3(1, 0, 0), UP, new Vector3(0, 0, 1)] as const;
const LENGTH_DEPTH = 40;
const FRAME_DEPTH = 10;
const FRAME_TURN = math.cos(math.rad(5));
const SEARCH_BUDGET = 20000;
const GAUSS_INNER = math.sqrt(5 - 2 * math.sqrt(10 / 7)) / 3;
const GAUSS_OUTER = math.sqrt(5 + 2 * math.sqrt(10 / 7)) / 3;
const GAUSS = [
	[0, 128 / 225],
	[GAUSS_INNER, (322 + 13 * math.sqrt(70)) / 900],
	[-GAUSS_INNER, (322 + 13 * math.sqrt(70)) / 900],
	[GAUSS_OUTER, (322 - 13 * math.sqrt(70)) / 900],
	[-GAUSS_OUTER, (322 - 13 * math.sqrt(70)) / 900],
] as const;

function finite(value: number) {
	return math.abs(value) < math.huge;
}

function liftPlanar(point: CurveVector): Vector3 {
	if (!typeIs(point, "Vector2")) error("a Vector2 curve takes only Vector2 points");
	if (!finite(point.X) || !finite(point.Y)) error("curve points must be finite");
	return new Vector3(point.X, point.Y, 0);
}

function lowerPlanar(point: Vector3): Vector2 {
	return new Vector2(point.X, point.Y);
}

function liftSpatial(point: CurveVector): Vector3 {
	if (!typeIs(point, "Vector3")) error("a Vector3 curve takes only Vector3 points");
	if (!finite(point.X) || !finite(point.Y) || !finite(point.Z)) error("curve points must be finite");
	return point;
}

function keepSpatial(point: Vector3): Vector3 {
	return point;
}

function build(
	points: readonly CurveVector[],
	options: CurveOptions<CurveVector>,
	toSpans: (points: readonly Vector3[], closed: boolean) => Chain,
): Curve<Vector2> | Curve<Vector3> {
	if (points.size() === 0) error("a curve needs at least one point");
	const closed = options.closed ?? false;
	const initialNormal = options.initialNormal;
	if (typeIs(points[0], "Vector2")) {
		if (initialNormal !== undefined) error("initialNormal applies only to Vector3 curves");
		return createCurve(shape(toSpans(points.map(liftPlanar), closed), closed, true), liftPlanar, lowerPlanar);
	}
	const seed = initialNormal === undefined ? undefined : liftSpatial(initialNormal);
	return createCurve(shape(toSpans(points.map(liftSpatial), closed), closed, false, seed), liftSpatial, keepSpatial);
}

function shape(spans: Chain, closed: boolean, planar: boolean, initialNormal?: Vector3): Geometry {
	let low = spans[0][0];
	let high = low;
	for (const span of spans) {
		for (const control of span) {
			if (!finite(control.X) || !finite(control.Y) || !finite(control.Z)) error("the curve's controls overflow");
			low = low.Min(control);
			high = high.Max(control);
		}
	}
	return { spans, closed, planar, initialNormal, tolerance: math.max(1e-4, 1e-5 * high.sub(low).Magnitude) };
}

// One axis of a span's velocity as a polynomial in u, constant term first.
function hodograph(a: number, b: number, c: number, d: number): LuaTuple<[number, number, number]> {
	return $tuple(3 * (b - a), 6 * (c - 2 * b + a), 3 * (d - 3 * c + 3 * b - a));
}

// Expanded from components in doubles, so arc length integrates without Vector3 arithmetic.
function squaredSpeed([a, b, c, d]: Span): Quartic {
	const [x0, x1, x2] = hodograph(a.X, b.X, c.X, d.X);
	const [y0, y1, y2] = hodograph(a.Y, b.Y, c.Y, d.Y);
	const [z0, z1, z2] = hodograph(a.Z, b.Z, c.Z, d.Z);
	return [
		x2 * x2 + y2 * y2 + z2 * z2,
		2 * (x2 * x1 + y2 * y1 + z2 * z1),
		x1 * x1 + y1 * y1 + z1 * z1 + 2 * (x2 * x0 + y2 * y0 + z2 * z0),
		2 * (x1 * x0 + y1 * y0 + z1 * z0),
		x0 * x0 + y0 * y0 + z0 * z0,
	];
}

function catmullRomSpans(input: readonly Vector3[], closed: boolean, alpha: number): Chain {
	const points = input.filter((point, index) => index === 0 || point !== input[index - 1]);
	if (closed && points.size() > 1 && points[points.size() - 1] === points[0]) points.pop();
	const count = points.size();
	if (count === 1) return [[points[0], points[0], points[0], points[0]]];
	if (closed && count < 3) error("a closed Catmull-Rom curve needs three distinct points");
	const at = (index: number) => {
		if (closed) return points[(index + count) % count];
		if (index < 0) return points[0].mul(2).sub(points[1]);
		if (index >= count) return points[count - 1].mul(2).sub(points[count - 2]);
		return points[index];
	};
	const last = closed ? count - 1 : count - 2;
	// gaps[k] spaces at(k - 1) and at(k). Neighboring spans share them.
	const gaps: number[] = [];
	for (const index of $range(-1, last + 1)) gaps.push(at(index + 1).sub(at(index)).Magnitude ** alpha);
	const span = (index: number): Span => {
		const [p0, p1, p2, p3] = [at(index - 1), at(index), at(index + 1), at(index + 2)];
		const [d01, d12, d23] = [gaps[index], gaps[index + 1], gaps[index + 2]];
		const m1 = p1
			.sub(p0)
			.div(d01)
			.sub(p2.sub(p0).div(d01 + d12))
			.add(p2.sub(p1).div(d12));
		const m2 = p2
			.sub(p1)
			.div(d12)
			.sub(p3.sub(p1).div(d12 + d23))
			.add(p3.sub(p2).div(d23));
		return [p1, p1.add(m1.mul(d12 / 3)), p2.sub(m2.mul(d12 / 3)), p2];
	};
	const spans: [Span, ...Span[]] = [span(0)];
	for (const index of $range(1, last)) spans.push(span(index));
	return spans;
}

function bezierSpans(points: readonly Vector3[], closed: boolean): Chain {
	const count = points.size();
	if (closed ? count % 3 !== 0 : count < 4 || count % 3 !== 1) {
		error(`a ${closed ? "closed cubic Bezier needs 3n" : "cubic Bezier needs 3n + 1"} points, got ${count}`);
	}
	const span = (start: number): Span => [
		points[start],
		points[start + 1],
		points[start + 2],
		points[(start + 3) % count],
	];
	const spans: [Span, ...Span[]] = [span(0)];
	for (const index of $range(1, math.floor(count / 3) - 1)) spans.push(span(index * 3));
	return spans;
}

function bSplineSpans(points: readonly Vector3[], closed: boolean): Chain {
	const count = points.size();
	const least = closed ? 3 : 4;
	if (count < least) error(`a ${closed ? "closed " : ""}uniform B-spline needs ${least} points, got ${count}`);
	const at = (index: number) => points[closed ? (index + count - 1) % count : index];
	const thirdOrSame = (near: Vector3, far: Vector3) => (near === far ? near : near.mul(2).add(far).div(3));
	const jointOrSame = (a: Vector3, b: Vector3, c: Vector3) =>
		a === b && b === c ? b : a.add(b.mul(4)).add(c).div(6);
	const span = (index: number): Span => {
		const [q0, q1, q2, q3] = [at(index), at(index + 1), at(index + 2), at(index + 3)];
		return [jointOrSame(q0, q1, q2), thirdOrSame(q1, q2), thirdOrSame(q2, q1), jointOrSame(q1, q2, q3)];
	};
	const spans: [Span, ...Span[]] = [span(0)];
	for (const index of $range(1, (closed ? count : count - 3) - 1)) spans.push(span(index));
	return spans;
}

function locate(geometry: Geometry, parameter: number): LuaTuple<[number, number]> {
	const count = geometry.spans.size();
	const wrapped = geometry.closed ? parameter - math.floor(parameter) : math.clamp(parameter, 0, 1);
	const scaled = wrapped * count;
	const index = math.min(math.floor(scaled), count - 1);
	return $tuple(index, scaled - index);
}

function bernstein([a, b, c, d]: Span, u: number): Vector3 {
	const v = 1 - u;
	return a
		.mul(v * v * v)
		.add(b.mul(3 * v * v * u))
		.add(c.mul(3 * v * u * u))
		.add(d.mul(u * u * u));
}

function velocity([a, b, c, d]: Span, u: number): Vector3 {
	const v = 1 - u;
	return b
		.sub(a)
		.mul(3 * v * v)
		.add(c.sub(b).mul(6 * v * u))
		.add(d.sub(c).mul(3 * u * u));
}

function acceleration([a, b, c, d]: Span, u: number): Vector3 {
	return c
		.sub(b.mul(2))
		.add(a)
		.mul(6 * (1 - u))
		.add(
			d
				.sub(c.mul(2))
				.add(b)
				.mul(6 * u),
		);
}

function direction(vector: Vector3): Vector3 | undefined {
	const size = vector.Magnitude;
	return size > 0 ? vector.div(size) : undefined;
}

// Rounding can take a near-zero square slightly negative.
function speed([u4, u3, u2, u1, u0]: Quartic, u: number): number {
	return math.sqrt(math.max((((u4 * u + u3) * u + u2) * u + u1) * u + u0, 0));
}

function travel([u4, u3, u2, u1, u0]: Quartic, from: number, to: number): number {
	const half = (to - from) / 2;
	const middle = (from + to) / 2;
	let sum = 0;
	for (const [node, weight] of GAUSS) {
		const u = middle + half * node;
		sum += weight * math.sqrt(math.max((((u4 * u + u3) * u + u2) * u + u1) * u + u0, 0));
	}
	return sum * half;
}

function measure(geometry: Geometry): ArcTable {
	const count = geometry.spans.size();
	let length = 0;
	const quartics = geometry.spans.map(squaredSpeed);
	const spans = quartics.map((quartic): ArcSpan => {
		const stops = [0];
		const distances = [length];
		const speeds = [speed(quartic, 0)];
		const visit = (from: number, to: number, whole: number, depth: number) => {
			const middle = (from + to) / 2;
			const left = travel(quartic, from, middle);
			const right = travel(quartic, middle, to);
			const allowed = math.max((geometry.tolerance * (to - from)) / count, 1e-6 * whole);
			if (depth >= 2 && math.abs(left + right - whole) <= allowed) {
				length += left + right;
				stops.push(to);
				distances.push(length);
				speeds.push(speed(quartic, to));
				return;
			}
			if (depth >= LENGTH_DEPTH) error("a curve span is too irregular to measure");
			visit(from, middle, left, depth + 1);
			visit(middle, to, right, depth + 1);
		};
		visit(0, 1, travel(quartic, 0, 1), 0);
		return { stops, distances, speeds };
	});
	return { quartics, spans, ends: spans.map(({ distances }) => distances[distances.size() - 1]), length };
}

function lastAtMost(values: readonly number[], value: number): number {
	let low = 0;
	let high = values.size() - 1;
	while (low < high) {
		const middle = math.ceil((low + high) / 2);
		if (values[middle] <= value) low = middle;
		else high = middle - 1;
	}
	return low;
}

function firstAtLeast(values: readonly number[], value: number, from = 0): number {
	let low = from;
	let high = values.size() - 1;
	while (low < high) {
		const middle = math.floor((low + high) / 2);
		if (values[middle] >= value) high = middle;
		else low = middle + 1;
	}
	return low;
}

function distanceAt(arc: ArcTable, index: number, u: number): number {
	const { stops, distances } = arc.spans[index];
	const station = lastAtMost(stops, u);
	if (stops[station] === u) return distances[station];
	return distances[station] + travel(arc.quartics[index], stops[station], u);
}

// Inverts distance over one stop with a cubic Hermite in the stop's share of distance, using du/ds = 1 / speed.
function hermiteGuess(low: number, high: number, run: number, share: number, startSpeed: number, endSpeed: number) {
	const linear = low + (high - low) * share;
	if (!(startSpeed > 0 && endSpeed > 0)) return linear;
	const square = share * share;
	const cube = square * share;
	const guess =
		low * (2 * cube - 3 * square + 1) +
		(run / startSpeed) * (cube - 2 * square + share) +
		high * (3 * square - 2 * cube) +
		(run / endSpeed) * (cube - square);
	return guess > low && guess < high ? guess : linear;
}

function parameterAt(geometry: Geometry, arc: ArcTable, distance: number): number {
	const total = arc.length;
	if (geometry.closed) distance -= math.floor(distance / total) * total;
	if (!(distance > 0 && distance < total)) return geometry.closed || distance <= 0 ? 0 : 1;
	const count = geometry.spans.size();
	const index = firstAtLeast(arc.ends, distance);
	const quartic = arc.quartics[index];
	const { stops, distances, speeds } = arc.spans[index];
	const station = firstAtLeast(distances, distance, 1) - 1;
	const from = stops[station];
	const start = distances[station];
	const run = distances[station + 1] - start;
	let low = from;
	let high = stops[station + 1];
	let u =
		run > 0 ? hermiteGuess(low, high, run, (distance - start) / run, speeds[station], speeds[station + 1]) : low;
	for (let step = 0; step < 40 && high - low > 1e-12; step++) {
		const miss = start + travel(quartic, from, u) - distance;
		if (math.abs(miss) <= geometry.tolerance * 1e-2) break;
		if (miss > 0) high = u;
		else low = u;
		const pace = speed(quartic, u);
		const stepped = pace > 0 ? u - miss / pace : low;
		u = stepped > low && stepped < high ? stepped : (low + high) / 2;
	}
	return (index + u) / count;
}

function perpendicular(vector: Vector3, tangent: Vector3): Vector3 | undefined {
	const flat = vector.sub(tangent.mul(tangent.Dot(vector)));
	const size = flat.Magnitude;
	return size > 1e-4 * vector.Magnitude ? flat.div(size) : undefined;
}

function seed(tangent: Vector3, preferred = UP): Vector3 {
	const normal = perpendicular(preferred, tangent);
	if (normal !== undefined) return normal;
	let axis = AXES[0];
	for (const candidate of AXES) if (math.abs(candidate.Dot(tangent)) < math.abs(axis.Dot(tangent))) axis = candidate;
	return axis.sub(tangent.mul(tangent.Dot(axis))).Unit;
}

function minimalRotation(from: Vector3, to: Vector3, normal: Vector3): Vector3 {
	const sum = from.add(to);
	const size = sum.Dot(sum);
	if (size <= 1e-12) return normal;
	const flipped = normal.sub(from.mul(2 * from.Dot(normal)));
	return flipped.sub(sum.mul((2 * sum.Dot(flipped)) / size));
}

function transport(from: Station, point: Vector3, tangent: Vector3, tolerance: number): Vector3 {
	const chord = point.sub(from.point);
	const c1 = chord.Dot(chord);
	let normal: Vector3;
	if (c1 <= (tolerance * 1e-3) ** 2) {
		normal = minimalRotation(from.tangent, tangent, from.normal);
	} else {
		const reflectedNormal = from.normal.sub(chord.mul((2 * chord.Dot(from.normal)) / c1));
		const reflectedTangent = from.tangent.sub(chord.mul((2 * chord.Dot(from.tangent)) / c1));
		const second = tangent.sub(reflectedTangent);
		const c2 = second.Dot(second);
		normal = c2 > 0 ? reflectedNormal.sub(second.mul((2 * second.Dot(reflectedNormal)) / c2)) : reflectedNormal;
	}
	return perpendicular(normal, tangent) ?? seed(tangent, from.normal);
}

// Returns the stops that have a tangent, with that tangent, which the frame table reuses.
function frameStops(span: Span): LuaTuple<[number[], Vector3[]]> {
	const stops: number[] = [];
	const tangents: Vector3[] = [];
	const keep = (u: number, tangent: Vector3 | undefined) => {
		if (tangent === undefined) return;
		stops.push(u);
		tangents.push(tangent);
	};
	const start = direction(velocity(span, 0));
	keep(0, start);
	const turns = (a: Vector3 | undefined, b: Vector3 | undefined) =>
		a === undefined || b === undefined ? a !== b : a.Dot(b) < FRAME_TURN;
	const visit = (
		from: number,
		fromTangent: Vector3 | undefined,
		to: number,
		toTangent: Vector3 | undefined,
		depth: number,
	) => {
		const middle = (from + to) / 2;
		const middleTangent = direction(velocity(span, middle));
		if (
			depth < FRAME_DEPTH &&
			(depth < 2 || turns(fromTangent, middleTangent) || turns(middleTangent, toTangent))
		) {
			visit(from, fromTangent, middle, middleTangent, depth + 1);
			visit(middle, middleTangent, to, toTangent, depth + 1);
		} else {
			keep(to, toTangent);
		}
	};
	visit(0, start, 1, direction(velocity(span, 1)), 0);
	return $tuple(stops, tangents);
}

function travelBetween(quartics: readonly Quartic[], from: number, fromU: number, to: number, toU: number) {
	if (from === to) return travel(quartics[from], fromU, toU);
	let sum = travel(quartics[from], fromU, 1) + travel(quartics[to], 0, toU);
	for (const index of $range(from + 1, to - 1)) sum += travel(quartics[index], 0, 1);
	return sum;
}

function transportFrames(geometry: Geometry): FrameTable {
	const positions: number[] = [];
	const stations: Station[] = [];
	geometry.spans.forEach((span, index) => {
		const [stops, tangents] = frameStops(span);
		stops.forEach((u, position) => {
			const tangent = tangents[position];
			const point = bernstein(span, u);
			const normal =
				stations.size() === 0
					? seed(tangent, geometry.initialNormal)
					: transport(stations[stations.size() - 1], point, tangent, geometry.tolerance);
			positions.push(index + u);
			stations.push({ index, u, point, tangent, normal });
		});
	});
	const count = geometry.spans.size();
	const first = stations[0];
	const last = stations[stations.size() - 1];
	if (!geometry.closed || stations.size() === 0 || positions[0] !== 0 || positions[positions.size() - 1] !== count) {
		return { positions, stations, seamRoll: 0, quartics: [], distances: [] };
	}
	const arrived = minimalRotation(last.tangent, first.tangent, last.normal);
	const seamRoll = math.atan2(first.tangent.Dot(arrived.Cross(first.normal)), arrived.Dot(first.normal));
	const quartics = geometry.spans.map(squaredSpeed);
	const distances = [0];
	for (const index of $range(1, stations.size() - 1)) {
		const [from, to] = [stations[index - 1], stations[index]];
		distances.push(distances[index - 1] + travelBetween(quartics, from.index, from.u, to.index, to.u));
	}
	return { positions, stations, seamRoll, quartics, distances };
}

function spatialNormal(
	geometry: Geometry,
	frames: FrameTable,
	index: number,
	u: number,
	point: Vector3,
	tangent: Vector3,
): Vector3 {
	if (frames.stations.size() === 0) return seed(tangent, geometry.initialNormal);
	const found = lastAtMost(frames.positions, index + u);
	const station = frames.stations[found];
	const normal = transport(station, point, tangent, geometry.tolerance);
	if (frames.seamRoll === 0) return normal;
	// The seam correction spreads by distance from the first station to the last.
	const { quartics, distances } = frames;
	const distance = distances[found] + travelBetween(quartics, station.index, station.u, index, u);
	return roll(normal, tangent, (frames.seamRoll * distance) / distances[distances.size() - 1]);
}

function roll(normal: Vector3, axis: Vector3, angle: number): Vector3 {
	return normal.mul(math.cos(angle)).add(axis.Cross(normal).mul(math.sin(angle)));
}

function halve([a, b, c, d]: Span): LuaTuple<[Span, Span]> {
	const ab = a.add(b).mul(0.5);
	const bc = b.add(c).mul(0.5);
	const cd = c.add(d).mul(0.5);
	const abc = ab.add(bc).mul(0.5);
	const bcd = bc.add(cd).mul(0.5);
	const middle = abc.add(bcd).mul(0.5);
	return $tuple([a, ab, abc, middle], [middle, bcd, cd, d]);
}

function segmentGap(point: Vector3, a: Vector3, d: Vector3): number {
	const chord = d.sub(a);
	const offset = point.sub(a);
	const along = chord.Dot(chord);
	if (along <= 0) return offset.Magnitude;
	return offset.sub(chord.mul(math.clamp(offset.Dot(chord) / along, 0, 1))).Magnitude;
}

function boxGap(low: Vector3, high: Vector3, query: Vector3): number {
	return query.sub(query.Max(low).Min(high)).Magnitude;
}

// Both bounds hold by the convex hull property. The chord bound shrinks with the square of a piece's size,
// the box bound only linearly, but the box is tighter on wide pieces such as a loop's whole span.
function gapBound(controls: Span, query: Vector3): number {
	const [a, b, c, d] = controls;
	const reach = math.max(segmentGap(b, a, d), segmentGap(c, a, d));
	return math.max(boxGap(a.Min(b, c, d), a.Max(b, c, d), query), segmentGap(query, a, d) - reach);
}

// Boxes over runs of neighboring spans, so a query skips spans far from it.
function cluster(spans: Chain, first: number, last: number): Cluster {
	if (first === last) {
		const [a, b, c, d] = spans[first];
		return { low: a.Min(b, c, d), high: a.Max(b, c, d), index: first };
	}
	const middle = math.floor((first + last) / 2);
	const left = cluster(spans, first, middle);
	const right = cluster(spans, middle + 1, last);
	return { low: left.low.Min(right.low), high: left.high.Max(right.high), index: first, children: [left, right] };
}

function pushEntry<T extends { readonly bound: number }>(heap: T[], entry: T) {
	let position = heap.size();
	heap.push(entry);
	while (position > 0) {
		const parent = math.floor((position - 1) / 2);
		if (heap[parent].bound <= entry.bound) break;
		heap[position] = heap[parent];
		position = parent;
	}
	heap[position] = entry;
}

function popEntry<T extends { readonly bound: number }>(heap: T[]): T | undefined {
	const top = heap[0];
	const last = heap.pop();
	const size = heap.size();
	if (last === undefined || size === 0) return last;
	let position = 0;
	for (let child = 1; child < size; child = 2 * position + 1) {
		if (child + 1 < size && heap[child + 1].bound < heap[child].bound) child += 1;
		if (heap[child].bound >= last.bound) break;
		heap[position] = heap[child];
		position = child;
	}
	heap[position] = last;
	return top;
}

function nearest(geometry: Geometry, tree: Cluster, query: Vector3): Candidate {
	const tolerance = geometry.tolerance;
	let best: Candidate = { index: 0, u: 0, separation: geometry.spans[0][0].sub(query).Magnitude };
	const offer = (index: number, u: number, point: Vector3) => {
		const separation = point.sub(query).Magnitude;
		if (separation < best.separation || (separation === best.separation && index + u < best.index + best.u))
			best = { index, u, separation };
	};
	const heap: (Branch | Piece)[] = [{ bound: boxGap(tree.low, tree.high, query), cluster: tree }];
	for (let step = 0; ; step++) {
		const entry = popEntry(heap);
		if (entry === undefined || entry.bound >= best.separation - tolerance) break;
		if (step >= SEARCH_BUDGET) error("the closest point search did not converge");
		if (entry.cluster !== undefined) {
			const { index, children } = entry.cluster;
			if (children !== undefined) {
				for (const child of children) {
					const bound = boxGap(child.low, child.high, query);
					if (bound < best.separation - tolerance) pushEntry(heap, { bound, cluster: child });
				}
				continue;
			}
			const span = geometry.spans[index];
			offer(index, 0, span[0]);
			offer(index, 1, span[3]);
			const bound = gapBound(span, query);
			if (bound < best.separation - tolerance) pushEntry(heap, { index, controls: span, from: 0, to: 1, bound });
			continue;
		}
		const { index, controls, from, to } = entry;
		const [a, , , d] = controls;
		// The chord's foot lands near the piece's closest point, so the best gap tightens quickly.
		const chord = d.sub(a);
		const along = chord.Dot(chord);
		if (along > 0) {
			const u = from + (to - from) * math.clamp(query.sub(a).Dot(chord) / along, 0, 1);
			offer(index, u, bernstein(geometry.spans[index], u));
		}
		const middle = (from + to) / 2;
		const [left, right] = halve(controls);
		offer(index, middle, left[3]);
		const limit = best.separation - tolerance;
		const leftBound = gapBound(left, query);
		if (leftBound < limit) pushEntry(heap, { index, controls: left, from, to: middle, bound: leftBound });
		const rightBound = gapBound(right, query);
		if (rightBound < limit) pushEntry(heap, { index, controls: right, from: middle, to, bound: rightBound });
	}
	return polish(geometry, query, best);
}

// The search stops within tolerance, which can leave the best point on a joint while the true closest point
// lies just inside the neighboring span, where Newton's clamp to one span can't reach.
function polish(geometry: Geometry, query: Vector3, found: Candidate): Candidate {
	const best = refineClosest(geometry.spans[found.index], query, found);
	const count = geometry.spans.size();
	const step = best.u === 1 ? 1 : best.u === 0 ? -1 : 0;
	const neighbor = best.index + step;
	if (step === 0 || (!geometry.closed && (neighbor < 0 || neighbor >= count))) return best;
	const index = (neighbor + count) % count;
	const across = refineClosest(geometry.spans[index], query, {
		index,
		u: step > 0 ? 0 : 1,
		separation: best.separation,
	});
	return across.separation < best.separation ? across : best;
}

function refineClosest(span: Span, query: Vector3, found: Candidate): Candidate {
	let best = found;
	let u = best.u;
	for (let step = 0; step < 8; step++) {
		const offset = bernstein(span, u).sub(query);
		const motion = velocity(span, u);
		const slope = motion.Dot(motion) + offset.Dot(acceleration(span, u));
		if (!(slope > 0)) break;
		u = math.clamp(u - offset.Dot(motion) / slope, 0, 1);
		const separation = bernstein(span, u).sub(query).Magnitude;
		if (!(separation < best.separation)) break;
		best = { index: best.index, u, separation };
	}
	return best;
}

function createCurve<V extends CurveVector>(
	geometry: Geometry,
	lift: (point: V) => Vector3,
	lower: (point: Vector3) => V,
): Curve<V> {
	let arc: ArcTable | undefined;
	let frames: FrameTable | undefined;
	let tree: Cluster | undefined;
	const measured = () => (arc ??= measure(geometry));
	const framed = () => (frames ??= transportFrames(geometry));
	const clustered = () => (tree ??= cluster(geometry.spans, 0, geometry.spans.size() - 1));
	const locateChecked = (parameter: number) => {
		if (!finite(parameter)) error("curve parameters must be finite");
		return locate(geometry, parameter);
	};
	return {
		pointAt: (parameter) => {
			const [index, u] = locateChecked(parameter);
			return lower(bernstein(geometry.spans[index], u));
		},
		tangentAt: (parameter) => {
			const [index, u] = locateChecked(parameter);
			const tangent = direction(velocity(geometry.spans[index], u));
			return tangent === undefined ? undefined : lower(tangent);
		},
		frameAt: (parameter) => {
			const [index, u] = locateChecked(parameter);
			const span = geometry.spans[index];
			const tangent = direction(velocity(span, u));
			if (tangent === undefined) return undefined;
			const point = bernstein(span, u);
			const normal = geometry.planar
				? new Vector3(-tangent.Y, tangent.X, 0)
				: spatialNormal(geometry, framed(), index, u, point, tangent);
			return { point: lower(point), tangent: lower(tangent), normal: lower(normal) };
		},
		length: () => measured().length,
		parameterAtDistance: (distance) => {
			if (!finite(distance)) error("curve distances must be finite");
			const lengths = measured();
			return lengths.length > 0 ? parameterAt(geometry, lengths, distance) : 0;
		},
		distanceAtParameter: (parameter) => {
			const [index, u] = locateChecked(parameter);
			return distanceAt(measured(), index, u);
		},
		closest: (query) => {
			const target = lift(query);
			const found = nearest(geometry, clustered(), target);
			const count = geometry.spans.size();
			const parameter = geometry.closed && found.index + found.u >= count ? 0 : (found.index + found.u) / count;
			const [index, u] = locate(geometry, parameter);
			const point = bernstein(geometry.spans[index], u);
			return {
				parameter,
				point: lower(point),
				distance: distanceAt(measured(), index, u),
				separation: point.sub(target).Magnitude,
			};
		},
		prepare: () => {
			measured();
			clustered();
			if (!geometry.planar) framed();
		},
	};
}
