import { workloads } from "../shared/workloads";

const HttpService = game.GetService("HttpService");
const RunService = game.GetService("RunService");

// Matches the MicroProfiler's frame limit, so a report covers the frames one capture holds.
const WINDOW = 256;

interface Track {
	readonly name: string;
	readonly scope: string;
	readonly checksum: number;
	readonly micros: Array<number>;
	readonly kilobytes: Array<number>;
	mismatches: number;
}

const tracks: Array<Track> = workloads.map((workload) => ({
	name: workload.name,
	scope: `bench:${workload.name}`,
	checksum: workload.run(),
	micros: [],
	kilobytes: [],
	mismatches: 0,
}));

let frames = 0;
RunService.Heartbeat.Connect(() => {
	const slot = frames % WINDOW;
	frames += 1;
	for (let i = 0; i < workloads.size(); i++) {
		const track = tracks[i];
		const heap = gcinfo();
		debug.profilebegin(track.scope);
		const started = os.clock();
		const checksum = workloads[i].run();
		const elapsed = os.clock() - started;
		debug.profileend();
		track.micros[slot] = elapsed * 1e6;
		track.kilobytes[slot] = gcinfo() - heap;
		if (checksum !== track.checksum) track.mismatches += 1;
	}
});

function summarize(values: ReadonlyArray<number>) {
	const sorted = [...values];
	sorted.sort((a, b) => a < b);
	const at = (fraction: number) => sorted[math.clamp(math.ceil(sorted.size() * fraction), 1, sorted.size()) - 1];
	let total = 0;
	for (const value of sorted) total += value;
	return { n: sorted.size(), median: at(0.5), p10: at(0.1), p90: at(0.9), mean: total / sorted.size() };
}

const report = new Instance("BindableFunction");
report.Name = "BenchReport";
report.OnInvoke = () =>
	HttpService.JSONEncode({
		frames,
		workloads: tracks.map((track) => ({
			name: track.name,
			checksum: track.checksum,
			mismatches: track.mismatches,
			micros: summarize(track.micros),
			kilobytes: summarize(track.kilobytes),
		})),
	});
report.Parent = script;
print("bench running");
