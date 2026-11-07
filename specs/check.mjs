import { configs, ensureJar, findJava, runTlc } from "./tlc.mjs";

await ensureJar();
const java = findJava();
const only = process.argv.slice(2);
const selected = configs().filter(
	(check) => only.length === 0 || only.some((name) => `${check.model}/${check.cfg}`.includes(name)),
);
if (selected.length === 0) {
	console.log(`no configuration matches ${only.join(", ")}`);
	process.exit(1);
}
const begun = Date.now();
let failed = 0;
for (const check of selected) {
	const label = `${check.model}/${check.cfg}`;
	const run = runTlc(java, check.dir, check.cfg, check.module, label);
	const ok = run.result === check.expect;
	const shown = run.result === "pass" ? "pass" : `violation ${run.result}`;
	console.log(
		`${ok ? "ok  " : "FAIL"} ${label}: ${shown}, ${run.distinct} distinct states, ${run.seconds.toFixed(1)}s`,
	);
	if (!ok) {
		failed += 1;
		const expected = check.expect === "pass" ? "pass" : `violation ${check.expect}`;
		console.log(`     expected ${expected}\n${run.output}`);
	}
}
console.log(`${selected.length} configurations, ${failed} failed, ${((Date.now() - begun) / 1000).toFixed(1)}s`);
process.exit(failed === 0 ? 0 : 1);
