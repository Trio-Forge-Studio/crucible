import assert from "node:assert/strict";
import test from "node:test";
import { checkFlow, markerLines, parseHud } from "./evidence.mjs";

test("the HUD text parses into coins, total and last gain", () => {
	assert.deepEqual(parseHud("Coins: 12   Total: 789   Last: +3"), { coins: 12, total: 789, last: 3 });
	assert.deepEqual(parseHud("Coins: 1.5   Total: 1.5   Last: +1.5"), { coins: 1.5, total: 1.5, last: 1.5 });
	assert.equal(parseHud("missing"), undefined);
});

test("a flow passes only when every press adds the multiplier to the coins and the total", () => {
	const good = [
		{ coins: 0, total: 0, last: 0 },
		{ coins: 1, total: 1, last: 1 },
		{ coins: 2, total: 2, last: 1 },
	];
	assert.deepEqual(checkFlow(good, 1), []);
	assert.equal(checkFlow([good[0], { coins: 1, total: 1, last: 2 }], 1).length, 1);
	assert.equal(checkFlow([good[0], { coins: 2, total: 1, last: 1 }], 1).length, 1);
	assert.equal(checkFlow([good[0], { coins: 1, total: 2, last: 1 }], 1).length, 1);
	assert.equal(checkFlow([{ coins: 1, total: 0, last: 0 }], 1).length, 1);
	assert.deepEqual(checkFlow([], 1), ["no HUD reading"]);
});

test("log lines count only after this run's server marker", () => {
	const native = [
		"S1|other|server|marker",
		"Sample.Wallet old collected 1",
		"S1|run-a|server|marker",
		"unrelated engine line",
		"Sample.Wallet Player1 collected 1",
		"S1|run-a|client|marker",
	].join("\n");
	assert.deepEqual(markerLines(native, "run-a"), [
		"S1|run-a|server|marker",
		"Sample.Wallet Player1 collected 1",
		"S1|run-a|client|marker",
	]);
	assert.deepEqual(markerLines(native, "run-b"), []);
});
