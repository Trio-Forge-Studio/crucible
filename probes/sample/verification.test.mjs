import assert from "node:assert/strict";
import test from "node:test";
import { checkFlow, markerLines, parseHud } from "./evidence.mjs";
import { chunks, parseRbxmx, steps } from "./rbxmx.mjs";

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
	assert.deepEqual(
		checkFlow(
			[
				{ coins: 7, total: 7, last: 0 },
				{ coins: 9, total: 9, last: 2 },
			],
			2,
			{ fresh: false },
		),
		[],
	);
	assert.equal(
		checkFlow(
			[
				{ coins: 7, total: 7, last: 0 },
				{ coins: 9, total: 9, last: 2 },
			],
			2,
		).length,
		1,
	);
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

const MODEL = `<roblox version="4">
  <Item class="Folder" referent="0"><Properties><string name="Name">S1Import</string></Properties>
    <Item class="Folder" referent="1"><Properties><string name="Name">ReplicatedStorage</string></Properties>
      <Item class="ModuleScript" referent="2"><Properties><string name="Name">a&amp;b</string>
        <string name="Source"><![CDATA[return "]]" .. 1 < 2
]]></string></Properties></Item>
      <Item class="Script" referent="3"><Properties><string name="Name">main</string><token name="RunContext">1</token>
        <string name="Source"><![CDATA[print(1)]]></string></Properties></Item>
    </Item>
  </Item>
</roblox>`;

test("a Rojo model parses into nodes and emits creation steps for each descendant", () => {
	const root = parseRbxmx(MODEL);
	assert.equal(root.name, "S1Import");
	const folder = root.children[0];
	assert.deepEqual(
		folder.children.map((child) => [child.class, child.name]),
		[
			["ModuleScript", "a&b"],
			["Script", "main"],
		],
	);
	assert.equal(folder.children[0].source, 'return "]]" .. 1 < 2\n');
	assert.equal(folder.children[1].runContext, 1);
	const list = steps(folder, ["ReplicatedStorage"]);
	assert.equal(list.length, 2);
	const [chunk] = chunks(list, 100000);
	assert.match(chunk, /Instance\.new\("ModuleScript"\)/);
	assert.match(chunk, /i\.Name = "a&b"/);
	assert.match(chunk, /\[=\[\n/);
	assert.match(chunk, /Enum\.RunContext:FromValue\(1\)/);
	assert.equal(chunks(list, 1).length, 2);
});
