import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { checked, configs, ensureJar, findJava, root, runTlc, tools } from "./tlc.mjs";

const [model, ...only] = process.argv.slice(2);
if (!model) {
	console.log("usage: node specs/mutate.mjs <model> [mutation id ...]");
	process.exit(2);
}
const dir = join(root, model);
const mutations = JSON.parse(readFileSync(join(dir, "mutations.json"), "utf8"));
const bases = new Map(
	configs()
		.filter((c) => c.model === model)
		.map((c) => [c.cfg, c]),
);

function fail(message) {
	console.log(message);
	process.exit(1);
}

function mutateModel(text, edits, id) {
	for (const [from, to] of edits) {
		const count = text.split(from).length - 1;
		if (count !== 1) fail(`${id}: an edit must match exactly once, matched ${count} times: ${from}`);
		text = text.replace(from, () => to);
	}
	return text;
}

function mutateConfig(base, set, only, id) {
	let lines = base.text.split("\n");
	for (const [name, value] of Object.entries(set ?? {})) {
		const at = lines.findIndex((line) => new RegExp(`^\\s+${name}\\s*(=|<-)`).test(line));
		if (at < 0) fail(`${id}: ${base.cfg} sets no constant ${name}`);
		lines[at] = `    ${name} ${value}`;
	}
	if (!only) return lines.join("\n");
	const sections = checked(base.text);
	const kind = sections.INVARIANT.includes(only) ? "INVARIANT" : sections.PROPERTY.includes(only) ? "PROPERTY" : null;
	if (!kind) fail(`${id}: ${base.cfg} does not check ${only}`);
	const kept = [];
	let skipping = false;
	for (const line of lines) {
		if (/^(INVARIANTS?|PROPERT(Y|IES))\b/.test(line)) skipping = true;
		else if (/^[A-Z_]+\b/.test(line)) skipping = false;
		if (!skipping) kept.push(line);
	}
	const at = kept.findIndex((line) => line.startsWith("CHECK_DEADLOCK"));
	kept.splice(at, 0, `${kind} ${only}`);
	return kept.join("\n");
}

const coveredBy = new Map();
for (const m of mutations) {
	const base = bases.get(m.config);
	if (!base) fail(`${m.id}: no configuration ${m.config}`);
	const names = m.only ? [m.only] : [...checked(base.text).INVARIANT, ...checked(base.text).PROPERTY];
	const caught = m.expect !== "liveness" ? [m.expect] : m.only ? [m.only] : names.filter((n) => /^Converges/.test(n));
	for (const name of caught) coveredBy.set(name, [...(coveredBy.get(name) ?? []), m.id]);
}
const required = new Set();
for (const base of bases.values()) {
	if (base.expect !== "pass") continue;
	for (const name of [...checked(base.text).INVARIANT, ...checked(base.text).PROPERTY]) required.add(name);
}
const uncovered = [...required].filter((name) => !coveredBy.has(name));

await ensureJar();
const java = findJava();
const selected = mutations.filter((m) => only.length === 0 || only.includes(m.id));
if (selected.length === 0) fail(`no mutation matches ${only.join(", ")}`);
let failed = 0;
for (const m of selected) {
	const base = bases.get(m.config);
	const work = join(tools, "mutants", model, m.id);
	rmSync(work, { recursive: true, force: true });
	mkdirSync(work, { recursive: true });
	cpSync(dir, work, { recursive: true, filter: (path) => !path.endsWith(".json") });
	const spec = join(work, base.module.replace(/^MC/, ""));
	writeFileSync(spec, mutateModel(readFileSync(spec, "utf8"), m.edits ?? [], m.id));
	writeFileSync(join(work, m.config), mutateConfig(base, m.set, m.only, m.id));
	const run = runTlc(java, work, m.config, base.module, `mutants/${model}/${m.id}`);
	rmSync(work, { recursive: true, force: true });
	const ok = run.result === m.expect;
	const scope = m.only ? `${m.config}, ${m.only} only` : m.config;
	console.log(
		`${ok ? "ok  " : "FAIL"} ${m.id} (${scope}): ${run.result}, ${run.distinct} distinct states, ${run.seconds.toFixed(1)}s`,
	);
	if (!ok) {
		failed += 1;
		console.log(`     expected ${m.expect}\n${run.output.slice(-4000)}`);
	}
}
if (only.length === 0 && uncovered.length > 0) {
	console.log(`no mutation catches ${uncovered.join(", ")}`);
	failed += 1;
}
console.log(`${selected.length} mutations, ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
