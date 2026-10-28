import assert from "node:assert/strict";
import { test } from "node:test";
import { ownsProcess, selectStudio, createPlaceName } from "./session.mjs";
import { retryStartup } from "./startup.mjs";
import fs from "node:fs";
import path from "node:path";

test("pnpm check includes the probe format lint and regression commands", () => {
	const { scripts } = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, "../../package.json"), "utf8"));
	assert.match(scripts.check, /stylua --check [^&]*\bprobes\b/);
	assert(scripts.check.includes("selene --config probes/engine/selene.toml probes"));
	assert(scripts.check.includes("node --test probes/engine/verification.test.mjs"));
	assert.match(scripts.fmt, /stylua [^&]*\bprobes\b/);
});

test("owned PID selection requires exact place and Studio executable despite PID reuse", () => {
	const place = "C:\\places\\owned.rbxl";
	assert(ownsProcess(`"C:\\Roblox\\RobloxStudioBeta.exe" "${place}"`, place));
	for (const command of [
		undefined,
		`"C:\\other.exe" "${place}"`,
		`"C:\\Roblox\\RobloxStudioBeta.exe" "${place}.other"`,
	])
		assert.equal(ownsProcess(command, place), false);
	const ownedName = createPlaceName("deferred-short");
	const foreignName = createPlaceName("deferred-short");
	assert.equal(
		selectStudio(
			[
				{ name: foreignName, id: "foreign" },
				{ name: ownedName, id: "owned" },
			],
			ownedName,
		),
		"owned",
	);
	assert.throws(() => selectStudio([{ name: "owned.rbxl" }, { name: "owned.rbxl" }], "owned.rbxl"));
	assert.notEqual(createPlaceName("same"), createPlaceName("same"));
});

test("both Studio modes retain binding render Error traceback shutdown and counted cap evidence", () => {
	for (const mode of ["deferred", "immediate"]) {
		const results = path.join(import.meta.dirname, "results", `${mode}-short`);
		const trace = fs.readFileSync(`${results}.log`, "utf8");
		const native = fs.readFileSync(`${results}-native.log`, "utf8");
		const session = JSON.parse(fs.readFileSync(`${results}-session.json`, "utf8"));
		for (const marker of [
			"tag added bound identity=studio-83 speed=17",
			"tag removed unbound",
			"tag readded new generation",
			"Parent=nil unbound tag retained",
			"reparent bound new generation",
			"BOOTED_ATTRIBUTE=true on LocalPlayer",
			"client observations complete",
			"measurements complete",
		])
			assert(
				trace.includes(
					`S2 ${marker.startsWith("client") || marker.startsWith("measurements") ? "" : "observed "}${marker}`,
				),
				`${mode} ${marker}`,
			);
		assert.equal(trace.match(/S2 observed render order Bind200,Crucible201,PreRender/g)?.length, 5);
		assert.match(
			native,
			/Error \[FLog::CreatorError\].*Renderer faulted in a frame hook:.*S2 intentional frame fault\r?\n[^\n]+S2ClientObserver:\d+\r?\n[^\n]+crucible-object:\d+/,
		);
		for (const hook of ["controller", "store"]) {
			const line = trace.split("\n").find((line) => line.includes(`S2 ${hook} shutdown`));
			assert(line);
			const seconds = (Date.parse(line.split(",")[0]) - Date.parse(session.stopRequestedAt)) / 1000;
			assert(seconds >= 0 && seconds < 25, `${mode} ${hook} within deadline`);
		}
		assert.equal(trace.match(/S2 measure pool .*errors=0/g)?.length, 5);
		assert.equal(trace.match(/S2 measure MAX_BATCHES .*starts=1024 expectedFailures=1/g)?.length, 5);
		assert.equal(trace.match(/S2 measure MAX_CHAINED_REQUESTS .*requests=65539 expectedFailures=1/g)?.length, 5);
		assert.equal(session.processRemaining, false);
		assert.equal(typeof session.driverPid, "number");
		assert.equal(trace.match(/S2 profile /g)?.length, 3);
	}
});

test("startup crashes wait sixty seconds and stop after three launches while other failures never retry", async () => {
	const attempts = [],
		waits = [];
	assert.equal(
		await retryStartup(
			async (attempt) => {
				attempts.push(attempt);
				return { code: 1, startupCrash: true };
			},
			async (ms) => waits.push(ms),
		),
		1,
	);
	assert.deepEqual(attempts, [1, 2, 3]);
	assert.deepEqual(waits, [60000, 60000]);
	assert.equal(
		await retryStartup(
			async () => ({ code: 0, startupCrash: false }),
			async () => assert.fail(),
		),
		0,
	);
	assert.equal(
		await retryStartup(
			async () => ({ code: 2, startupCrash: false }),
			async () => assert.fail(),
		),
		2,
	);
});
