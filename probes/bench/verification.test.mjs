import test from "node:test";
import assert from "node:assert/strict";
import { aggregate, renderTable } from "./aggregate.mjs";

const run = (variant, rep, micros, checksum = 42, extra = {}) => ({
	variant,
	rep,
	loadBefore: 10,
	loadAfter: 20,
	clock: {
		workloads: [
			{
				name: "loop",
				checksum,
				mismatches: 0,
				micros: { median: micros },
				kilobytes: { median: 0.5 },
			},
		],
	},
	libmp: { workloads: { loop: { micros: { median: micros + 1 } } } },
	...extra,
});

test("the summary takes the median of the run medians and compares each variant with rbxtsc", () => {
	const summary = aggregate([
		run("rbxtsc", 1, 100),
		run("rbxtsc", 2, 300),
		run("rbxtsc", 3, 200),
		run("compiler", 1, 400),
		run("compiler", 2, 401),
		run("compiler", 3, 399),
	]);
	const rbxtsc = summary.rows.find((r) => r.variant === "rbxtsc");
	const compiler = summary.rows.find((r) => r.variant === "compiler");
	assert.deepEqual(rbxtsc.clockMicros, { median: 200, range: [100, 300] });
	assert.deepEqual(rbxtsc.libmpMicros, { median: 201, range: [101, 301] });
	assert.equal(compiler.vsRbxtsc, 401 / 201);
	assert.deepEqual(summary.cpuLoadPercent, { median: 10, range: [10, 20] });
	assert.deepEqual(summary.disagreements, []);
});

test("variants that compute different results are reported", () => {
	const summary = aggregate([run("rbxtsc", 1, 100, 42), run("compiler", 1, 100, 43)]);
	assert.equal(summary.disagreements.length, 1);
	assert.match(renderTable(summary), /Checksum disagreements: loop\./);
});

test("a failed run is listed and left out of the medians", () => {
	const summary = aggregate([run("rbxtsc", 1, 100), { variant: "rbxtsc", rep: 2, error: "Error: boom" }]);
	assert.equal(summary.rows[0].runs, 1);
	assert.deepEqual(summary.failed, [{ variant: "rbxtsc", rep: 2, error: "Error: boom" }]);
});
