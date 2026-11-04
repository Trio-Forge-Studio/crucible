local golden = require("./lib/spline-golden")
local spec = require("./lib/load")
-- Lune's definitions omit Roblox datatypes.
local roblox: any = require("@lune/roblox")

local Vector2 = roblox.Vector2
local Vector3 = roblox.Vector3

local Spline = spec.runtime({ Vector2 = Vector2, Vector3 = Vector3 }).module("maths/spline")

local function near(a: any, b: any, tolerance: number?): boolean
	return (a - b).Magnitude <= (tolerance or 1e-4)
end

local function close(actual: number, expected: number, tolerance: number, label: string)
	assert(math.abs(actual - expected) <= tolerance, `{label}: {actual} differs from {expected}`)
end

-- A plain function, so luau-lsp doesn't refine on Roblox type names it lacks.
local function kind(value: any): string
	return typeof(value)
end

local function finite(value: any): boolean
	if kind(value) == "number" then
		return math.abs(value) < math.huge
	end
	return finite(value.X) and finite(value.Y) and (kind(value) == "Vector2" or finite(value.Z))
end

local function rejects(build: () -> any, label: string)
	assert(not pcall(build), `{label} is rejected`)
end

local function v3(x: number, y: number, z: number?): any
	return Vector3.new(x, y, z or 0)
end

local WIGGLE = { v3(0, 0, 0), v3(4, 1, 0), v3(5, 6, 2), v3(9, 6, -1), v3(12, 0, 0) }

local function circle(count: number, radius: number): { any }
	local points = {}
	for index = 0, count - 1 do
		local angle = 2 * math.pi * index / count
		table.insert(points, v3(radius * math.cos(angle), radius * math.sin(angle), 0))
	end
	return points
end

local function polylineLength(curve: any, steps: number): number
	local total, previous = 0, curve.pointAt(0)
	for step = 1, steps do
		local point = curve.pointAt(step / steps)
		total += (point - previous).Magnitude
		previous = point
	end
	return total
end

local function richardson(coarse: number, fine: number): number
	return (4 * fine - coarse) / 3
end

local function transportAround(loop: any, steps: number): any
	local normal = loop.frameAt(0).normal
	local point, tangent = loop.pointAt(0), loop.tangentAt(0)
	for step = 1, steps do
		local nextPoint, nextTangent = loop.pointAt(step / steps), loop.tangentAt(step / steps)
		local chord = nextPoint - point
		local reflected = normal - chord * (2 * chord:Dot(normal) / chord:Dot(chord))
		local reflectedTangent = tangent - chord * (2 * chord:Dot(tangent) / chord:Dot(chord))
		local second = nextTangent - reflectedTangent
		normal = reflected - second * (2 * second:Dot(reflected) / second:Dot(second))
		point, tangent = nextPoint, nextTangent
	end
	return normal
end

local tests: { [string]: () -> () } = {
	["Catmull-Rom lands exactly on every point for each alpha"] = function()
		for _, alpha in { 0, 0.5, 1 } do
			local curve = Spline.catmullRom(WIGGLE, { alpha = alpha })
			for index, point in WIGGLE do
				assert(curve.pointAt((index - 1) / 4) == point, `alpha {alpha} lands on point {index}`)
			end
		end
		local uniform = Spline.catmullRom(WIGGLE, { alpha = 0 })
		local chordal = Spline.catmullRom(WIGGLE, { alpha = 1 })
		assert(not near(uniform.pointAt(0.125), chordal.pointAt(0.125), 1e-3), "alpha changes the shape")
		assert(
			Spline.catmullRom(WIGGLE).pointAt(0.3) == Spline.catmullRom(WIGGLE, { alpha = 0.5 }).pointAt(0.3),
			"the default alpha is centripetal"
		)
		local segment = Spline.catmullRom({ v3(0, 0, 0), v3(10, 0, 0) })
		assert(near(segment.pointAt(0.5), v3(5, 0, 0)), "two points make a straight segment")
		local loop = Spline.catmullRom({ v3(0, 0, 0), v3(10, 0, 0), v3(10, 10, 0), v3(0, 10, 0) }, { closed = true })
		for index, point in { v3(0, 0, 0), v3(10, 0, 0), v3(10, 10, 0), v3(0, 10, 0) } do
			assert(loop.pointAt((index - 1) / 4) == point, `a loop lands on point {index}`)
		end
	end,
	["cubic Bezier lands on its anchors and leaves along its handles"] = function()
		local controls =
			{ v3(0, 0, 0), v3(0, 4, 0), v3(6, 4, 0), v3(6, 0, 0), v3(6, -3, 0), v3(10, -3, 0), v3(10, 0, 0) }
		local curve = Spline.cubicBezier(controls)
		assert(curve.pointAt(0) == controls[1] and curve.pointAt(0.5) == controls[4], "starts and joins on anchors")
		assert(curve.pointAt(1) == controls[7], "ends on the last anchor")
		assert(near(curve.tangentAt(0), v3(0, 1, 0)), "leaves along the first handle")
		assert(near(curve.tangentAt(0.5), v3(0, -1, 0)), "a join takes the outgoing handle")
		assert(near(curve.tangentAt(1), v3(0, 1, 0)), "arrives along the last handle")
		local first = (controls[1] + controls[2] * 3 + controls[3] * 3 + controls[4]) / 8
		assert(near(curve.pointAt(0.25), first, 1e-5), "the midpoint of a span is (p0 + 3p1 + 3p2 + p3) / 8")
		local loop = Spline.cubicBezier({ v3(0, 0, 0), v3(5, -5, 0), v3(5, 5, 0) }, { closed = true })
		assert(loop.pointAt(1) == v3(0, 0, 0), "a closed Bezier wraps to its first anchor")
		local expected = (v3(0, 0, 0) + v3(5, -5, 0) * 3 + v3(5, 5, 0) * 3 + v3(0, 0, 0)) / 8
		assert(near(loop.pointAt(0.5), expected, 1e-5), "a closed span's fourth control is the first anchor")
	end,
	["a uniform B-spline matches the hand-computed cubic"] = function()
		local curve = Spline.uniformBSpline({ v3(0, 0, 0), v3(1, 2, 0), v3(3, 2, 0), v3(4, 0, 0) })
		assert(near(curve.pointAt(0), v3(7 / 6, 10 / 6, 0), 1e-5), "starts at (p0 + 4p1 + p2) / 6")
		assert(near(curve.pointAt(0.5), v3(2, 23 / 12, 0), 1e-5), "the middle weighs 1, 23, 23, 1 over 48")
		assert(near(curve.pointAt(1), v3(17 / 6, 10 / 6, 0), 1e-5), "ends at (p1 + 4p2 + p3) / 6")
		local repeated = Spline.uniformBSpline({ v3(0, 0, 0), v3(1, 2, 0), v3(1, 2, 0), v3(3, 2, 0), v3(4, 0, 0) })
		assert(not near(repeated.pointAt(0.5), curve.pointAt(0.5), 1e-3), "a repeated control still pulls")
	end,
	["Vector2 points give Vector2 results that match the Vector3 curve"] = function()
		local flat = { Vector2.new(0, 0), Vector2.new(4, 1), Vector2.new(5, 6), Vector2.new(9, 6) }
		local lifted = { v3(0, 0, 0), v3(4, 1, 0), v3(5, 6, 0), v3(9, 6, 0) }
		local pairs = {
			{ Spline.catmullRom(flat), Spline.catmullRom(lifted) },
			{ Spline.cubicBezier(flat), Spline.cubicBezier(lifted) },
			{ Spline.uniformBSpline(flat), Spline.uniformBSpline(lifted) },
		}
		for index, pair in pairs do
			local planar, spatial = pair[1], pair[2]
			local point, frame, found = planar.pointAt(0.4), planar.frameAt(0.4), planar.closest(Vector2.new(3, 3))
			assert(kind(point) == "Vector2" and kind(planar.tangentAt(0.4)) == "Vector2", `kind {index} point`)
			assert(kind(frame.point) == "Vector2" and kind(frame.normal) == "Vector2", `kind {index} frame`)
			assert(kind(found.point) == "Vector2", `kind {index} closest point`)
			local other = spatial.pointAt(0.4)
			assert(point.X == other.X and point.Y == other.Y, `kind {index} matches the Vector3 curve`)
			close(planar.length(), spatial.length(), 1e-6, `kind {index} length`)
			local tangent = frame.tangent
			assert(near(frame.normal, Vector2.new(-tangent.Y, tangent.X), 1e-6), `kind {index} normal is the left side`)
		end
	end,
	["constructors reject invalid input"] = function()
		rejects(function()
			return Spline.catmullRom({})
		end, "no points")
		rejects(function()
			return Spline.catmullRom({ v3(0, 0 / 0, 0) })
		end, "a NaN point")
		rejects(function()
			return Spline.uniformBSpline({ v3(0, 0, 0), v3(1, 0, 0), v3(math.huge, 0, 0), v3(3, 0, 0) })
		end, "an infinite point")
		rejects(function()
			return Spline.catmullRom({ v3(0, 0, 0), Vector2.new(1, 0) })
		end, "mixed dimensions")
		rejects(function()
			return Spline.catmullRom({ Vector2.new(0, 0), v3(1, 0, 0) })
		end, "mixed dimensions led by a Vector2")
		rejects(function()
			return Spline.cubicBezier({ v3(0, 0, 0), v3(1, 0, 0), v3(2, 0, 0), v3(3, 0, 0), v3(4, 0, 0) })
		end, "an open Bezier of five controls")
		rejects(function()
			return Spline.cubicBezier({ v3(0, 0, 0), v3(1, 0, 0), v3(2, 0, 0), v3(3, 0, 0) }, { closed = true })
		end, "a closed Bezier of four controls")
		rejects(function()
			return Spline.uniformBSpline({ v3(0, 0, 0), v3(1, 0, 0), v3(2, 0, 0) })
		end, "an open B-spline of three controls")
		rejects(function()
			return Spline.uniformBSpline({ v3(0, 0, 0), v3(1, 0, 0) }, { closed = true })
		end, "a closed B-spline of two controls")
		for _, alpha in { -0.1, 1.5, 0 / 0 } do
			rejects(function()
				return Spline.catmullRom(WIGGLE, { alpha = alpha })
			end, `alpha {alpha}`)
		end
		rejects(function()
			return Spline.catmullRom({ v3(0, 0, 0), v3(1, 0, 0), v3(0, 0, 0) }, { closed = true })
		end, "a closed Catmull-Rom with two distinct points")
		rejects(function()
			return Spline.catmullRom({ Vector2.new(0, 0), Vector2.new(1, 0) }, { initialNormal = v3(0, 1, 0) })
		end, "an initial normal on a Vector2 curve")
		local curve = Spline.catmullRom(WIGGLE)
		rejects(function()
			return curve.pointAt(0 / 0)
		end, "a NaN parameter")
		rejects(function()
			return curve.parameterAtDistance(math.huge)
		end, "an infinite distance")
	end,
	["exact repeats of a Catmull-Rom point coalesce"] = function()
		local plain = Spline.catmullRom({ v3(0, 0, 0), v3(5, 2, 0), v3(9, 0, 3) })
		local repeated = Spline.catmullRom({ v3(0, 0, 0), v3(0, 0, 0), v3(5, 2, 0), v3(9, 0, 3), v3(9, 0, 3) })
		local square = { v3(0, 0, 0), v3(10, 0, 0), v3(10, 10, 0), v3(0, 10, 0) }
		local loop = Spline.catmullRom(square, { closed = true })
		local repeatedLoop = Spline.catmullRom(
			{ square[1], square[2], square[3], square[4], square[1] },
			{ closed = true }
		)
		for step = 0, 10 do
			assert(plain.pointAt(step / 10) == repeated.pointAt(step / 10), `the open curve matches at {step}`)
			assert(loop.pointAt(step / 10) == repeatedLoop.pointAt(step / 10), `the loop matches at {step}`)
		end
	end,
	["a single point is a constant curve"] = function()
		local held = v3(3, 4, 5)
		local constants: { any } =
			{ Spline.catmullRom({ held }), Spline.catmullRom({ held, held, held }, { closed = true }) }
		for _, curve in constants do
			assert(curve.pointAt(0) == held and curve.pointAt(0.7) == held and curve.pointAt(1) == held, "holds")
			assert(curve.tangentAt(0.5) == nil and curve.frameAt(0.5) == nil, "has no direction")
			assert(curve.length() == 0 and curve.parameterAtDistance(5) == 0, "has no length")
			local found = curve.closest(v3(3, 4, 8))
			assert(found.point == held and found.parameter == 0 and found.distance == 0, "projects onto the point")
			close(found.separation, 3, 1e-6, "gap")
		end
	end,
	["closed loops wrap continuously across the seam"] = function()
		local ring = circle(6, 10)
		local loops: { [string]: any } = {
			catmullRom = Spline.catmullRom(ring, { closed = true }),
			uniformBSpline = Spline.uniformBSpline(ring, { closed = true }),
			cubicBezier = Spline.cubicBezier(
				{ v3(10, 0, 0), v3(10, 6, 0), v3(-10, 6, 0), v3(-10, 0, 0), v3(-10, -6, 0), v3(10, -6, 0) },
				{ closed = true }
			),
		}
		for name, loop in loops do
			assert(loop.pointAt(1) == loop.pointAt(0), `{name} closes exactly`)
			assert(near(loop.pointAt(1 - 1e-6), loop.pointAt(0), 1e-3), `{name} approaches the seam`)
			assert(loop.tangentAt(1 - 1e-6):Dot(loop.tangentAt(0)) > 0.999, `{name} keeps its direction`)
			assert(near(loop.pointAt(1.25), loop.pointAt(0.25), 1e-5), `{name} wraps past 1`)
			assert(near(loop.pointAt(-0.25), loop.pointAt(0.75), 1e-5), `{name} wraps below 0`)
		end
	end,
	["coincident points never produce NaN"] = function()
		local a, b = v3(1, 1, 1), v3(6, 2, 0)
		local curves = {
			Spline.catmullRom({ a, a, b, b, a, a }),
			Spline.catmullRom({ a, b, a, b }, { alpha = 1 }),
			Spline.cubicBezier({ a, a, b, b }),
			Spline.cubicBezier({ a, a, a, a }),
			Spline.uniformBSpline({ a, a, a, b, b, b }),
			Spline.uniformBSpline({ a, a, a, a }),
		}
		for index, curve in curves do
			assert(finite(curve.length()), `curve {index} length`)
			for step = 0, 20 do
				local parameter = step / 20
				assert(finite(curve.pointAt(parameter)), `curve {index} point at {parameter}`)
				local tangent = curve.tangentAt(parameter)
				assert(tangent == nil or finite(tangent), `curve {index} tangent at {parameter}`)
				local frame = curve.frameAt(parameter)
				assert(frame == nil or finite(frame.normal), `curve {index} frame at {parameter}`)
				assert(finite(curve.parameterAtDistance(parameter * curve.length())), `curve {index} inversion`)
			end
			local found = curve.closest(v3(3, 3, 3))
			assert(finite(found.point) and finite(found.separation) and finite(found.distance), `curve {index} closest`)
		end
		local bezier = curves[3]
		assert(bezier.tangentAt(0) == nil and bezier.frameAt(1) == nil, "a handle on its anchor has no direction")
	end,
	["straight and circular lengths are accurate"] = function()
		close(Spline.catmullRom({ v3(0, 0, 0), v3(10, 0, 0) }).length(), 10, 1e-4, "a two-point segment")
		local uneven = Spline.cubicBezier({ v3(0, 0, 0), v3(0.5, 0, 0), v3(1, 0, 0), v3(10, 0, 0) })
		close(uneven.length(), 10, 1e-4, "a straight Bezier with bunched handles")
		local line = Spline.uniformBSpline({ v3(0, 0, 0), v3(3, 4, 0), v3(6, 8, 0), v3(9, 12, 0), v3(12, 16, 0) })
		close(line.length(), 10, 1e-4, "a straight B-spline")
		local loop = Spline.catmullRom(circle(16, 10), { closed = true })
		local coarse, fine = polylineLength(loop, 2000), polylineLength(loop, 4000)
		close(loop.length(), richardson(coarse, fine), 2e-4, "a closed circle approximation")
		close(loop.length(), 2 * math.pi * 10, 0.05, "close to the true circle")
	end,
	["even distances give even chords"] = function()
		local uneven = Spline.cubicBezier({ v3(0, 0, 0), v3(0.5, 0, 0), v3(1, 0, 0), v3(10, 0, 0) })
		assert(math.abs(uneven.pointAt(0.5).X - 5) > 3, "parameter and distance differ on this curve")
		for step = 0, 10 do
			local point = uneven.pointAt(uneven.parameterAtDistance(step))
			close(point.X, step, 2e-4, `{step} studs along`)
		end
		local bent = Spline.catmullRom(WIGGLE)
		local total, previous, low, high = bent.length(), bent.pointAt(0), math.huge, 0
		for step = 1, 200 do
			local point = bent.pointAt(bent.parameterAtDistance(step / 200 * total))
			local chord = (point - previous).Magnitude
			low, high, previous = math.min(low, chord), math.max(high, chord), point
		end
		assert(high - low < 0.005 * high, `even chords, got {low} to {high}`)
	end,
	["distance and parameter round trip"] = function()
		local curve = Spline.catmullRom(WIGGLE)
		local total = curve.length()
		for step = 0, 50 do
			local distance = step / 50 * total
			close(curve.distanceAtParameter(curve.parameterAtDistance(distance)), distance, 1e-4, `distance {distance}`)
			local parameter = step / 50
			close(
				curve.parameterAtDistance(curve.distanceAtParameter(parameter)),
				parameter,
				1e-6,
				`parameter {parameter}`
			)
		end
		local previous = -1
		for step = 0, 100 do
			local parameter = curve.parameterAtDistance(step / 100 * total)
			assert(parameter > previous, "parameters grow with distance")
			previous = parameter
		end
	end,
	["open curves clamp and closed curves wrap distance"] = function()
		local open = Spline.catmullRom(WIGGLE)
		local total = open.length()
		assert(open.parameterAtDistance(-5) == 0 and open.parameterAtDistance(total + 5) == 1, "open distances clamp")
		assert(open.distanceAtParameter(-1) == 0 and open.distanceAtParameter(2) == total, "open parameters clamp")
		local loop = Spline.catmullRom(circle(7, 10), { closed = true })
		local around = loop.length()
		close(loop.parameterAtDistance(around + 12), loop.parameterAtDistance(12), 1e-6, "past the end")
		close(loop.parameterAtDistance(-12), loop.parameterAtDistance(around - 12), 1e-6, "before the start")
		close(loop.distanceAtParameter(1.25), loop.distanceAtParameter(0.25), 1e-6, "parameters past 1")
		assert(loop.parameterAtDistance(around) == 0, "a full lap returns to the start")
	end,
	["frames stay orthonormal and match the curve"] = function()
		local helix = {}
		for index = 0, 12 do
			local angle = index * 0.8
			table.insert(helix, v3(8 * math.cos(angle), index * 1.5, 8 * math.sin(angle)))
		end
		local curve = Spline.catmullRom(helix)
		for step = 0, 200 do
			local parameter = step / 200
			local frame = curve.frameAt(parameter)
			close(frame.tangent.Magnitude, 1, 1e-5, "unit tangent")
			close(frame.normal.Magnitude, 1, 1e-5, "unit normal")
			close(frame.tangent:Dot(frame.normal), 0, 1e-5, "perpendicular")
			assert(frame.point == curve.pointAt(parameter), "the frame sits on the curve")
			assert(near(frame.tangent, curve.tangentAt(parameter), 1e-6), "the frame follows the tangent")
		end
		local shuffled = curve.frameAt(0.61)
		curve.frameAt(0.1)
		assert(near(curve.frameAt(0.61).normal, shuffled.normal, 1e-6), "queries in any order agree")
		local rolled = Spline.catmullRom(helix, { initialNormal = v3(1, 0, 0) })
		assert(not near(rolled.frameAt(0.5).normal, curve.frameAt(0.5).normal, 0.1), "the initial normal sets the roll")
	end,
	["frames do not flip through an inflection"] = function()
		local curve = Spline.cubicBezier({ v3(0, 0, 0), v3(10, 10, 0), v3(10, -10, 0), v3(20, 0, 0) })
		local function curvatureSide(parameter: number): any
			local before, after = curve.tangentAt(parameter - 1e-3), curve.tangentAt(parameter + 1e-3)
			return (after - before).Unit
		end
		assert(curvatureSide(0.2):Dot(curvatureSide(0.8)) < -0.5, "the Frenet normal flips across the inflection")
		local previous = curve.frameAt(0).normal
		for step = 1, 400 do
			local normal = curve.frameAt(step / 400).normal
			assert(normal:Dot(previous) > 0.99, `the normal stays continuous at step {step}`)
			previous = normal
		end
		assert(curve.frameAt(0.2).normal:Dot(curve.frameAt(0.8).normal) > 0.5, "the normal keeps its side")
	end,
	["a closed loop's frame matches across the seam"] = function()
		local points = { v3(10, 0, 0), v3(0, 10, 0), v3(-10, 0, 0), v3(0, 0, 10), v3(0, -10, 5) }
		local loop = Spline.catmullRom(points, { closed = true })
		local normal = transportAround(loop, 4000)
		assert(normal:Dot(loop.frameAt(0).normal) < 0.9, "uncorrected transport misses the start")
		local start, finish = loop.frameAt(0), loop.frameAt(1)
		assert(near(start.normal, finish.normal, 1e-6), "parameters 0 and 1 share a frame")
		assert(near(loop.frameAt(1 - 1e-5).normal, start.normal, 1e-3), "the frame closes without a jump")
		close(loop.frameAt(0.5).tangent:Dot(loop.frameAt(0.5).normal), 0, 1e-5, "the corrected frame stays orthonormal")
	end,
	["closest picks the near arm of a hairpin"] = function()
		local curve = Spline.catmullRom({ v3(0, 0, 0), v3(20, 0, 0), v3(20, 2, 0), v3(0, 2, 0) })
		for _, query in { v3(4, 1.3, 0), v3(12, 1.2, 0.5), v3(3, 0.8, 0) } do
			local best, bestParameter = math.huge, 0
			for step = 0, 30000 do
				local gap = (curve.pointAt(step / 30000) - query).Magnitude
				if gap < best then
					best, bestParameter = gap, step / 30000
				end
			end
			local found = curve.closest(query)
			close(found.separation, best, 1e-4, "the global gap")
			close(found.parameter, bestParameter, 1e-3, "the global parameter")
			close(found.separation, (found.point - query).Magnitude, 1e-6, "the gap is the separation")
			close(found.distance, curve.distanceAtParameter(found.parameter), 1e-6, "the distance is along the curve")
		end
		assert(curve.closest(v3(4, 1.3, 0)).point.Y > 1, "the upper arm wins")
	end,
	["closest reaches just past a joint"] = function()
		local loop = Spline.catmullRom(circle(60, 100), { closed = true })
		for joint = 0, 59, 3 do
			local parameter = joint / 60 + 1e-4
			local frame = loop.frameAt(parameter)
			-- The normal is perpendicular to the tangent, so its foot on the loop is the closest point.
			local found = loop.closest(frame.point + frame.normal * 15)
			close(found.parameter, parameter, 2e-5, `past joint {joint}`)
			close(found.separation, 15, 2e-5, `gap past joint {joint}`)
		end
	end,
	["a point on the curve projects onto itself"] = function()
		local curve = Spline.catmullRom(WIGGLE)
		for step = 0, 20 do
			local parameter = step / 20
			local found = curve.closest(curve.pointAt(parameter))
			assert(found.separation < 1e-4, `gap at {parameter} is {found.separation}`)
			close(found.parameter, parameter, 1e-4, "parameter")
			assert(near(found.point, curve.pointAt(found.parameter), 1e-6), "the point is evaluated at the parameter")
		end
		local loop = Spline.catmullRom(circle(5, 10), { closed = true })
		assert(loop.closest(loop.pointAt(0)).parameter == 0, "the seam projects to parameter 0")
	end,
	["prepare builds the tables without changing answers"] = function()
		local points = { v3(10, 0, 0), v3(0, 10, 0), v3(-10, 0, 0), v3(0, 0, 10), v3(0, -10, 5) }
		local lazy = Spline.catmullRom(points, { closed = true })
		local ready = Spline.catmullRom(points, { closed = true })
		ready.prepare()
		ready.prepare()
		assert(ready.length() == lazy.length(), "the same length")
		for step = 0, 20 do
			local parameter = step / 20
			assert(ready.frameAt(parameter).normal == lazy.frameAt(parameter).normal, `the same frame at {parameter}`)
			assert(ready.parameterAtDistance(step) == lazy.parameterAtDistance(step), `the same inversion at {step}`)
		end
		local query = v3(3, 4, 5)
		assert(ready.closest(query).point == lazy.closest(query).point, "the same projection")
		Spline.catmullRom({ Vector2.new(0, 0), Vector2.new(4, 3) }).prepare()
		Spline.catmullRom({ v3(1, 2, 3) }).prepare()
	end,
	["a curve frozen before its first query still answers"] = function()
		local curve = table.freeze(Spline.catmullRom({ v3(0, 0, 0), v3(10, 0, 0), v3(10, 5, 5) }, { closed = true }))
		local holder = table.freeze({ curve = curve })
		assert(holder.curve.length() > 0, "length builds its table")
		assert(holder.curve.frameAt(0.3) ~= nil, "frames build their table")
		assert(holder.curve.closest(v3(5, 1, 0)).separation < 2, "projection measures distance")
		table.freeze(Spline.cubicBezier({ v3(0, 0, 0), v3(1, 2, 0), v3(3, 2, 0), v3(4, 0, 0) })).prepare()
		local evaluate = holder.curve.pointAt
		assert(evaluate(0) == v3(0, 0, 0), "a detached property still works")
	end,
	["the documented examples print what they say"] = function()
		local path = Spline.catmullRom({ v3(0, 0, 0), v3(10, 5, 0), v3(20, 0, 0) })
		assert(path.pointAt(0.5).Y == 5, "the quickstart")
		local flat = Spline.catmullRom({ Vector2.new(0, 0), Vector2.new(4, 8), Vector2.new(12, 0) }, { alpha = 0.5 })
		assert(flat.pointAt(0.5).Y == 8, "Catmull-Rom")
		local bezier = Spline.cubicBezier({ v3(0, 0, 0), v3(0, 4, 0), v3(6, 4, 0), v3(6, 0, 0) })
		assert(bezier.pointAt(0.5).Y == 3, "cubic Bezier")
		local bspline = Spline.uniformBSpline({ v3(0, 0, 0), v3(3, 0, 0), v3(6, 0, 0), v3(9, 0, 0) })
		assert(bspline.pointAt(0).X == 3 and bspline.pointAt(1).X == 6, "uniform B-spline")
		local even = Spline.cubicBezier({ v3(0, 0, 0), v3(0.5, 0, 0), v3(1, 0, 0), v3(10, 0, 0) })
		local parameter = even.parameterAtDistance(5 * 1)
		assert(math.abs(even.pointAt(parameter).X - 5) < 0.001, "an even speed")
		assert(math.abs(even.distanceAtParameter(parameter) - 5) < 0.001, "the distance at that parameter")
		local track = Spline.catmullRom({ v3(0, 0, 0), v3(40, 0, 10), v3(80, 5, 0) })
		track.prepare()
		assert(track.length() > 80, "prepared tables")
		local line = Spline.catmullRom({ v3(0, 0, 0), v3(10, 0, 0) })
		local found = line.closest(v3(4, 3, 0))
		assert(math.abs(found.point.X - 4) < 0.001 and math.abs(found.separation - 3) < 0.001, "closest")
		local ahead = line.pointAt(line.parameterAtDistance(found.distance + 2))
		assert(math.abs(ahead.X - 6) < 0.001, "a point ahead on the curve")
		local loop = Spline.catmullRom({ v3(0, 0, 0), v3(10, 0, 0), v3(10, 10, 0), v3(0, 10, 0) }, { closed = true })
		assert(loop.pointAt(1) == loop.pointAt(0), "a loop closes")
		assert(loop.pointAt(-0.25) == loop.pointAt(0.75), "a loop wraps")
		assert(loop.parameterAtDistance(loop.length()) == 0, "a full lap returns to 0")
	end,
	["a degree-elevated quadratic matches its reference and lands exactly"] = function()
		local start = Vector3.new(-7, 2, 4)
		local handle = Vector3.new(3, 18, -5)
		local target = Vector3.new(17, -3, 11)
		local curve = Spline.cubicBezier({
			start,
			start + (handle - start) * (2 / 3),
			target + (handle - target) * (2 / 3),
			target,
		})
		assert(curve.pointAt(0) == start, "starts exactly")
		assert(curve.pointAt(1) == target, "lands exactly")
		for step = 0, 100 do
			local t = step / 100
			local expected = start * ((1 - t) ^ 2) + handle * (2 * (1 - t) * t) + target * (t ^ 2)
			assert((curve.pointAt(t) - expected).Magnitude < 1e-5, `matches the quadratic at {t}`)
		end
	end,
}

-- Each recorded case is a curve built from the published @trioforge/spline 0.2.0 and its answers to the same queries.
-- Float results match within 1e-9 of the larger of 1 and the recorded value. The curve math is IEEE arithmetic, but
-- frames and initial normals go through math.sin, math.cos and math.atan, which a platform may round differently.
local function within(actual: number, expected: number, label: string)
	local tolerance = 1e-9 * math.max(1, math.abs(expected))
	if not (math.abs(actual - expected) <= tolerance) then
		error(`{label}: expected {string.format("%.17g", expected)}, got {string.format("%.17g", actual)}`, 3)
	end
end

local function components(dimension: number, value: any, expected: { number }, label: string)
	assert(kind(value) == (if dimension == 2 then "Vector2" else "Vector3"), `{label} keeps the point type`)
	within(value.X, expected[1], `{label}.X`)
	within(value.Y, expected[2], `{label}.Y`)
	if dimension == 3 then
		within(value.Z, expected[3], `{label}.Z`)
	end
end

local function build(case: any): any
	local points = {}
	for _, point in case.points do
		table.insert(
			points,
			if case.dim == 2 then Vector2.new(point[1], point[2]) else v3(point[1], point[2], point[3])
		)
	end
	local options = table.clone(case.options)
	if options.initialNormal then
		options.initialNormal = v3(table.unpack(options.initialNormal))
	end
	return Spline[case.kind](points, if next(options) == nil then nil else options)
end

local function query(case: any, point: { number }): any
	return if case.dim == 2 then Vector2.new(point[1], point[2]) else v3(point[1], point[2], point[3])
end

for _, case in golden do
	tests[`recorded answers: {case.name}`] = function()
		local curve = build(case)
		local dimension = case.dim
		within(curve.length(), case.length, "length")
		for index, parameter in case.parameters do
			local sample = case.samples[index]
			local label = `t = {parameter}`
			components(dimension, curve.pointAt(parameter), sample.point, `point at {label}`)
			local tangent = curve.tangentAt(parameter)
			if sample.tangent == false then
				assert(tangent == nil, `no tangent at {label}`)
			else
				components(dimension, tangent, sample.tangent, `tangent at {label}`)
			end
			local frame = curve.frameAt(parameter)
			if sample.frame == false then
				assert(frame == nil, `no frame at {label}`)
			else
				components(dimension, frame.point, sample.frame.point, `frame point at {label}`)
				components(dimension, frame.tangent, sample.frame.tangent, `frame tangent at {label}`)
				components(dimension, frame.normal, sample.frame.normal, `frame normal at {label}`)
			end
			within(curve.distanceAtParameter(parameter), sample.distance, `distance at {label}`)
		end
		local length = curve.length()
		for _, record in case.distances do
			local distance = record.fraction * length
			within(
				curve.parameterAtDistance(distance),
				record.parameter,
				`parameter at {record.fraction} of the length`
			)
		end
		for _, record in case.queries do
			local found = curve.closest(query(case, record.query))
			local label = `closest to {table.concat(record.query, ", ")}`
			within(found.parameter, record.parameter, `{label}: parameter`)
			components(dimension, found.point, record.point, `{label}: point`)
			within(found.distance, record.distance, `{label}: distance`)
			within(found.separation, record.separation, `{label}: separation`)
		end
	end
end

return tests
