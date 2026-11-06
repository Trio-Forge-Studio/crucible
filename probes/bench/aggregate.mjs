const median = (values) => {
	const sorted = [...values].sort((a, b) => a - b);
	return sorted[Math.ceil(sorted.length / 2) - 1];
};
const range = (values) => [Math.min(...values), Math.max(...values)];

// Each run reports one median per workload. The summary takes the median of those run medians, and the range across
// runs is the spread a difference between variants has to clear.
export function aggregate(records) {
	const ok = records.filter((r) => !r.error && r.clock && r.libmp);
	const rows = [];
	const checksums = new Map();
	for (const record of ok) {
		for (const workload of record.clock.workloads) {
			const key = `${workload.name}\u0000${record.variant}`;
			let row = rows.find((r) => r.key === key);
			if (!row) {
				row = {
					key,
					workload: workload.name,
					variant: record.variant,
					runs: 0,
					libmp: [],
					clock: [],
					kb: [],
					collected: false,
				};
				rows.push(row);
			}
			row.runs += 1;
			row.clock.push(workload.micros.median);
			row.kb.push(workload.kilobytes.median);
			// The heap shrinks across a call when a garbage collection step runs inside it, so its delta then says
			// nothing about what the call allocated.
			if (workload.kilobytes.p10 < 0) row.collected = true;
			const scope = record.libmp.workloads[workload.name];
			if (scope) row.libmp.push(scope.micros.median);
			const seen = checksums.get(workload.name) ?? new Map();
			seen.set(record.variant, [...(seen.get(record.variant) ?? []), workload.checksum]);
			if (workload.mismatches > 0) seen.set("mismatch", true);
			checksums.set(workload.name, seen);
		}
	}
	const summary = rows.map((row) => ({
		workload: row.workload,
		variant: row.variant,
		runs: row.runs,
		libmpMicros: row.libmp.length ? { median: median(row.libmp), range: range(row.libmp) } : null,
		clockMicros: { median: median(row.clock), range: range(row.clock) },
		kilobytesPerCall: row.collected ? null : median(row.kb),
	}));
	for (const row of summary) {
		const base = summary.find((r) => r.workload === row.workload && r.variant === "rbxtsc");
		const value = row.libmpMicros?.median;
		row.vsRbxtsc = base?.libmpMicros && value ? value / base.libmpMicros.median : null;
	}
	const disagreements = [...checksums].flatMap(([workload, seen]) => {
		const values = new Set([...seen].filter(([k]) => k !== "mismatch").flatMap(([, v]) => v));
		return values.size > 1 || seen.get("mismatch") ? [{ workload, checksums: Object.fromEntries(seen) }] : [];
	});
	const loads = records.flatMap((r) => [r.loadBefore, r.loadAfter]).filter((v) => typeof v === "number");
	return {
		runs: records.length,
		failed: records.filter((r) => r.error).map((r) => ({ variant: r.variant, rep: r.rep, error: r.error })),
		cpuLoadPercent: loads.length ? { median: median(loads), range: range(loads) } : null,
		disagreements,
		rows: summary,
	};
}

const micros = (m) => (m ? `${m.median.toFixed(1)} (${m.range[0].toFixed(1)} to ${m.range[1].toFixed(1)})` : "n/a");

export function renderTable(summary) {
	const lines = [
		"| Workload | Variant | Runs | LibMP µs per call | os.clock µs per call | KB per call | vs rbxtsc |",
		"|---|---|---|---|---|---|---|",
	];
	for (const row of summary.rows)
		lines.push(
			`| ${row.workload} | ${row.variant} | ${row.runs} | ${micros(row.libmpMicros)} | ${micros(row.clockMicros)} | ${row.kilobytesPerCall === null ? "GC inside calls" : row.kilobytesPerCall.toFixed(2)} | ${row.vsRbxtsc ? `${row.vsRbxtsc.toFixed(2)}x` : "n/a"} |`,
		);
	const load = summary.cpuLoadPercent;
	lines.push(
		"",
		`Runs ${summary.runs}, failed ${summary.failed.length}. CPU load ${load ? `median ${load.median}%, ${load.range[0]}% to ${load.range[1]}%` : "unknown"}.`,
		`Checksum disagreements: ${summary.disagreements.length ? summary.disagreements.map((d) => d.workload).join(", ") : "none"}.`,
		"",
	);
	return lines.join("\n");
}
