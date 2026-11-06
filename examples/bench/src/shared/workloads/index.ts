import { construct } from "./construct";
import { loop } from "./loop";
import { map } from "./map";
import { numeric } from "./numeric";
import { pairsStep } from "./pairs";
import { vector } from "./vector";

export interface Workload {
	readonly name: string;
	readonly run: () => number;
}

export const workloads: ReadonlyArray<Workload> = [
	{ name: "loop", run: loop },
	{ name: "construct", run: construct },
	{ name: "map", run: map },
	{ name: "vector", run: vector },
	{ name: "pairs", run: pairsStep },
	{ name: "numeric", run: numeric },
];
