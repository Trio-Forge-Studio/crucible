import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import test from "node:test";
import config from "../../vite.config.ts";
import { createPlaceName, selectStudio } from "./session.mjs";

const root = path.resolve(import.meta.dirname, "../..");

test("F1 every tracked Luau file uses exactly one correct Selene standard", () => {
	const files = execFileSync("git", ["ls-files", "*.luau"], { cwd: root, encoding: "utf8" }).trim().split(/\r?\n/);
	for (const file of files) {
		const commands = Object.entries(config.staged)
			.filter(
				([glob]) =>
					glob.endsWith(".luau") && path.matchesGlob(glob.includes("/") ? file : path.basename(file), glob),
			)
			.map(([, value]) => value);
		assert.deepEqual(
			commands,
			[["stylua", file.startsWith("probes/") ? "selene --config probes/engine/selene.toml" : "selene"]],
			file,
		);
	}
});

test("F2 row 1b includes both moves and their settle windows without tag signals", () => {
	const readme = fs.readFileSync(new URL("README.md", import.meta.url), "utf8");
	const row = readme.split("\n").find((line) => line.startsWith("| 1b."));
	for (const [label, start, end] of [
		["deferred", 23, 39],
		["immediate", 20, 33],
	]) {
		assert.ok(row.includes(`(results/${label}-short.log) L${start}-${end}`));
		const lines = fs
			.readFileSync(new URL(`results/${label}-short.log`, import.meta.url), "utf8")
			.split(/\r?\n/)
			.slice(start - 1, end);
		const moves = lines.slice(lines.findIndex((line) => line.includes("|1b|before ServerStorage")));
		assert.ok(moves.some((line) => line.includes("|1b|after workspace")));
		assert.equal(moves.filter((line) => /\|1b\|(added|removed)/.test(line)).length, 0);
		assert.ok(lines.at(-1).includes(label === "deferred" ? "|f13|" : "|3.client|frame=3"));
	}
});

test("F3 each launch has a unique place name even within one driver process", () => {
	const first = createPlaceName("deferred-short");
	assert.ok(first.startsWith(`s1-deferred-short-${process.pid}-`));
	assert.notEqual(createPlaceName("deferred-short"), first);
});

test("F3 session selection ignores other places and waits for its own", () => {
	const name = createPlaceName("deferred-short");
	const other = { name: "s1-deferred-short.rbxl", id: "other" };
	assert.equal(selectStudio([other], name), undefined);
	assert.equal(selectStudio([other, { name, id: "owned" }], name), "owned");
});

test("F3 ambiguous Studio names fail before a session can be driven", () => {
	const name = createPlaceName("deferred-short");
	assert.throws(
		() =>
			selectStudio(
				[
					{ name, id: "one" },
					{ name, id: "two" },
				],
				name,
			),
		/Multiple Studios match owned place/,
	);
});
