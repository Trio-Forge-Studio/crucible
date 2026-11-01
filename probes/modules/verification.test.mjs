import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { createPlaceName, selectStudio } from "./session.mjs";

const trace = fs.readFileSync(process.env.S0_TRACE || new URL("results/deferred-short.log", import.meta.url), "utf8");
const lines = trace.trim().split(/\r?\n/);
const streaming = fs.readFileSync(
	process.env.S0_STREAM_TRACE || new URL("results/deferred-short-r2.log", import.meta.url),
	"utf8",
);
const streamingLines = streaming.trim().split(/\r?\n/);

test("NET 67 and 68 a previously visible streamed-out part can retain its Instance slot", () => {
	const observations = [
		"|1a-transition|visible=Workspace.PreviouslyVisiblePart",
		"|1a-transition|client confirmed visible; moving character beyond target radius",
		"|1a-transition|server part still present=true",
		"|1a-transition|previously visible part removed locally",
		"|1a-transition|client confirmed stream-out; server part present=true",
		"|1a-transition|FireClient ok=true error=nil",
		"|1a-transition|received local=nil part=PreviouslyVisiblePart keep=intact number=42",
	];
	let previous = -1;
	for (const observation of observations) {
		const index = streaming.indexOf(observation);
		assert.ok(index > previous, `missing or out-of-order observation ${observation}`);
		previous = index;
	}
});

test("NET 67 missing Instance slots preserve sibling fields in both directions", () => {
	assert.ok(trace.includes("local-far=nil part=nil keep=intact number=42"));
	assert.ok(trace.includes("|1b|part=nil keep=intact"));
	assert.ok(trace.includes("FireClient ok=true error=nil"));
});
test("NET 12 and 13 catalog attributes are visible on discovery and intact", () => {
	assert.ok(trace.includes("WaitForChild published=true"));
	for (const size of [867, 66559]) assert.ok(trace.includes(`size=${size} type=string length=${size} intact=true`));
});
test("NET 48 and 52 all 200 handler starts preserve order while the first yields", () => {
	const starts = lines.filter((line) => line.includes("|3order|start=")).slice(0, 200);
	assert.deepEqual(
		starts.map((line) => Number(line.split("start=")[1])),
		Array.from({ length: 200 }, (_, i) => i + 1),
	);
	const end1 = lines.findIndex((line) => line.endsWith("|3order|end=1"));
	assert.ok(end1 > lines.findIndex((line) => line.endsWith("|3order|start=200")));
});
test("RX 31 each client interval records deferred Heartbeat, PreRender and signal callbacks", () => {
	const firstRun = lines.slice(
		0,
		lines.findIndex((line) => line.includes("|3kick|handler before Kick")),
	);
	let complete = 0;
	for (let frame = 1; frame <= 35; frame++) {
		const rows = firstRun.filter((line) => line.includes("S0|client|") && line.includes(`|f${frame}|4|`));
		if (!rows.some((line) => line.endsWith("|PreRender"))) continue;
		const phases = rows.map((line) => line.split("|4|")[1]);
		assert.deepEqual(phases, [
			"Heartbeat",
			"defer-Heartbeat",
			"signal",
			"defer-signal",
			"PreRender",
			"defer-PreRender",
			"PreAnimation",
			"PreSimulation",
			"PostSimulation",
		]);
		complete++;
	}
	assert.ok(complete >= 30, `only ${complete} complete render intervals`);
	let compared = 0;
	for (let frame = 5; frame <= 35; frame++) {
		const rows = firstRun.filter((line) => line.includes("S0|server|") && line.includes(`|f${frame}|`));
		const handler = rows.findIndex((line) => line.includes("|4remote|handler="));
		if (handler < 0) continue;
		const deferred = rows.findIndex((line) => line.includes("|4remote|defer="));
		assert.ok(handler >= 0 && deferred > handler);
		const heartbeat = rows.findIndex((line) => line.endsWith("|4|Heartbeat"));
		const preSimulation = rows.findIndex((line) => line.endsWith("|4|PreSimulation"));
		assert.ok(heartbeat >= 0 && handler > heartbeat && preSimulation > deferred);
		compared++;
	}
	assert.ok(compared >= 30);
	for (let frame = 1; frame <= 35; frame++) {
		assert.ok(trace.includes(`|4remote|handler=${frame}`));
		assert.ok(trace.includes(`|4remote|defer=${frame}`));
	}
});
test("DATA 58 receipt environment failure and DATA 2 dotted override have evidence", () => {
	assert.ok(trace.includes("gameId=0 placeId=0 ok=false result=HTTP 404 (Not Found)"));
	assert.ok(trace.includes("S0.Dotted ok=true result=nil"));
});
test("NET 35, 52 and 77 kick traces preserve the observed callback lifetime and Kick-to-removal delay", () => {
	for (const kind of ["startup", "handler"]) {
		assert.ok(trace.includes(`|3kick|${kind} before Kick`));
		assert.ok(trace.includes(`|3kick|${kind} after Kick`));
		assert.ok(trace.includes(`|3kick|FAIL received ${kind}`));
	}
	const afterKick = lines.findIndex((line) => line.endsWith("|3kick|handler after Kick"));
	const removal = lines.findIndex((line, index) => index > afterKick && line.includes("|3kick|PlayerRemoving"));
	const returned = lines.findIndex((line) => line.endsWith("|3kick|handler return"));
	assert.ok(afterKick >= 0 && removal > afterKick && returned > removal);
	const clock = (line) => Number(line.split("S0|server|")[1].split("|")[0]);
	for (const traceLines of [lines, streamingLines]) {
		const kicks = traceLines.flatMap((line, index) =>
			/\|3kick\|(startup|handler) after Kick$/.test(line) ? [index] : [],
		);
		assert.equal(kicks.length, 2);
		for (const kick of kicks) {
			const removed = traceLines.findIndex(
				(line, index) => index > kick && line.includes("|3kick|PlayerRemoving"),
			);
			assert.ok(removed > kick, `no PlayerRemoving after ${traceLines[kick]}`);
			const delay = clock(traceLines[removed]) - clock(traceLines[kick]);
			assert.ok(delay >= 1, `PlayerRemoving ${delay.toFixed(3)} s after ${traceLines[kick]}`);
		}
	}
});
test("DATA 97 and 100 parked reads retain the engine error", () => {
	assert.ok(trace.includes("|6fetch|ok=false result=ConfigService: Failed to load config"));
});
test("owned sessions have unique names and reject ambiguous selection", () => {
	const name = createPlaceName("deferred-short");
	assert.ok(name.startsWith(`s0-deferred-short-${process.pid}-`));
	assert.notEqual(name, createPlaceName("deferred-short"));
	assert.equal(selectStudio([{ name: "other", id: "other" }], name), undefined);
	assert.equal(selectStudio([{ name, id: "own" }], name), "own");
	assert.throws(
		() =>
			selectStudio(
				[
					{ name, id: "a" },
					{ name, id: "b" },
				],
				name,
			),
		/Multiple Studios/,
	);
});
