# Spline

Build Catmull-Rom, cubic Bezier and uniform B-spline curves from Vector2 or Vector3 points.

Use a curve to sample positions, travel by distance, orient objects and find the closest point on a path.

Import it from `@trioforge/crucible/maths/spline`. The subpath exports `catmullRom`, `cubicBezier` and `uniformBSpline`, and the types `Curve`, `CurveFrame`, `CurveProjection`, `CurveOptions`, `CatmullRomOptions` and `CurveVector`. It has no dependencies, and the root, server and client entries never load it.

This example builds a path through three points and samples it at the middle point.

```ts
import { catmullRom } from "@trioforge/crucible/maths/spline";

const path = catmullRom([new Vector3(0, 0, 0), new Vector3(10, 5, 0), new Vector3(20, 0, 0)]);
print(path.pointAt(0.5).Y);
// → 5
```

Parameters run from 0 to 1, and each span gets an equal share of that range, so equal parameter steps don't always cover equal distances. An open curve clamps parameters and distances at its ends.

Every curve function is an arrow property, so Luau callers use `.`, as in `path.pointAt(0.5)` and `path.length()`. You can also pass these functions around on their own without binding them to the curve.

## Choose a curve

### Catmull-Rom

`catmullRom` passes through every point you give it. `alpha` sets the knot spacing. The default, `0.5`, is centripetal. Use `0` for uniform spacing or `1` for chordal spacing.

```ts
import { catmullRom } from "@trioforge/crucible/maths/spline";

const path = catmullRom([new Vector2(0, 0), new Vector2(4, 8), new Vector2(12, 0)], { alpha: 0.5 });
print(path.pointAt(0.5).Y);
// → 8
```

Consecutive copies of the same point count as one, and a single point makes a constant curve with zero length.

### Cubic Bezier

Each `cubicBezier` span is an anchor, two handles, and another anchor. Neighboring spans share the anchor between them, so an open curve needs `3n + 1` controls.

```ts
import { cubicBezier } from "@trioforge/crucible/maths/spline";

const path = cubicBezier([new Vector3(0, 0, 0), new Vector3(0, 4, 0), new Vector3(6, 4, 0), new Vector3(6, 0, 0)]);
print(path.pointAt(0.5).Y);
// → 3
```

### Uniform B-spline

`uniformBSpline` approximates its controls. An open curve needs at least four controls and usually doesn't reach its endpoints.

```ts
import { uniformBSpline } from "@trioforge/crucible/maths/spline";

const path = uniformBSpline([new Vector3(0, 0, 0), new Vector3(3, 0, 0), new Vector3(6, 0, 0), new Vector3(9, 0, 0)]);
print(path.pointAt(0).X);
// → 3
print(path.pointAt(1).X);
// → 6
```

## Move at an even speed

To move at a steady speed, turn the elapsed time into a distance in studs and pass it to `parameterAtDistance` before you sample. The curve builds its distance table the first time you ask and keeps it.

```ts
import { cubicBezier } from "@trioforge/crucible/maths/spline";

const path = cubicBezier([new Vector3(0, 0, 0), new Vector3(0.5, 0, 0), new Vector3(1, 0, 0), new Vector3(10, 0, 0)]);
const speed = 5;
const elapsed = 1;
const parameter = path.parameterAtDistance(speed * elapsed);
const position = path.pointAt(parameter);
print(math.abs(position.X - 5) < 0.001);
// → true
print(math.abs(path.distanceAtParameter(parameter) - 5) < 0.001);
// → true
```

Run this conversion with the current elapsed time on every update. `length()` returns the total distance along the curve.

## Orient along a path

`frameAt` returns a point, a unit tangent, and a unit normal. On `Vector3` curves the frames twist as little as possible along the path. The first normal points toward world up unless you set `initialNormal` to choose the starting roll. If the path starts straight up or down, or `initialNormal` is zero or parallel to the tangent, the curve uses the world axis least aligned with the tangent instead.

This matrix points the object's local negative Z axis along the tangent and its local Y axis along the normal.

```ts
import { catmullRom } from "@trioforge/crucible/maths/spline";

const path = catmullRom([new Vector3(0, 0, 0), new Vector3(10, 0, 0)], { initialNormal: new Vector3(0, 1, 0) });
const frame = path.frameAt(0.5);
if (frame !== undefined) {
	const right = frame.tangent.Cross(frame.normal);
	const pose = CFrame.fromMatrix(frame.point, right, frame.normal, frame.tangent.mul(-1));
	print(pose.LookVector.Dot(frame.tangent) > 0.999);
	// → true
}
```

`tangentAt` and `frameAt` return `undefined` at stationary points, where the curve has no direction. On a `Vector2` curve, the normal is the tangent turned a quarter to the left. On a `Vector3` curve, the first frame query builds the frame lookup table.

## Build tables ahead

Distances, spatial frames, and `closest` each build a lookup table the first time you use them. On a long curve, building them can take a moment, so a first query in the middle of a cutscene can hitch. Call `prepare()` while the game loads to build every table up front. It doesn't change any answer.

```ts
import { catmullRom } from "@trioforge/crucible/maths/spline";

const track = catmullRom([new Vector3(0, 0, 0), new Vector3(40, 0, 10), new Vector3(80, 5, 0)]);
track.prepare();
print(track.length() > 80);
// → true
```

## Snap to a path

`closest` searches the whole curve. In its result, `distance` is how far along the curve the closest point is, and `separation` is the straight-line gap between that point and the one you passed.

```ts
import { catmullRom } from "@trioforge/crucible/maths/spline";

const path = catmullRom([new Vector3(0, 0, 0), new Vector3(10, 0, 0)]);
const found = path.closest(new Vector3(4, 3, 0));
print(math.abs(found.point.X - 4) < 0.001);
// → true
print(math.abs(found.separation - 3) < 0.001);
// → true
const ahead = path.pointAt(path.parameterAtDistance(found.distance + 2));
print(math.abs(ahead.X - 6) < 0.001);
// → true
```

The result, a `CurveProjection`, also has the `parameter` at the closest point.

## Close a loop

Set `closed` to make parameters and distances wrap around. Don't repeat the first point at the end. Catmull-Rom drops a repeated last point, but Bezier and B-spline curves count it as one more control, which either changes the shape or fails the control count. A closed Catmull-Rom needs at least three points after consecutive copies count as one, unless every point is the same. A closed B-spline needs at least three controls. A closed Bezier needs `3n` controls, with the last two handles leading back to the first anchor.

```ts
import { catmullRom } from "@trioforge/crucible/maths/spline";

const loop = catmullRom(
	[new Vector3(0, 0, 0), new Vector3(10, 0, 0), new Vector3(10, 10, 0), new Vector3(0, 10, 0)],
	{ closed: true },
);
print(loop.pointAt(1) === loop.pointAt(0));
// → true
print(loop.pointAt(-0.25) === loop.pointAt(0.75));
// → true
print(loop.parameterAtDistance(loop.length()));
// → 0
```

On a closed `Vector3` curve, the frames spread their roll correction along the loop so they meet at the seam. On a Bezier, the handles decide whether the tangent is continuous across a join.

## Types and validation

A curve keeps the dimension of its points, and `initialNormal` applies only to `Vector3` curves.

The constructors reject empty inputs, mixed dimensions, non-finite points, wrong control counts, and an `alpha` outside 0 to 1. Queries reject a non-finite parameter, distance, or point.

## Moving from `@trioforge/spline`

The subpath is the code of the retired `@trioforge/spline` package, unchanged. Change the import to `@trioforge/crucible/maths/spline` and every curve answers as before.
