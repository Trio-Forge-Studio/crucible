const HUD = /Coins: (\d+(?:\.\d+)?)\s+Total: (\d+(?:\.\d+)?)\s+Last: \+(\d+(?:\.\d+)?)/;

export function parseHud(text) {
	const match = HUD.exec(text ?? "");
	if (!match) return undefined;
	return { coins: Number(match[1]), total: Number(match[2]), last: Number(match[3]) };
}

export function markerLines(native, runId) {
	const lines = native.split(/\r?\n/);
	const first = lines.findIndex((line) => line.includes(`S1|${runId}|server|marker`));
	if (first < 0) return [];
	return lines
		.slice(first)
		.filter((line) => line.includes("S1|") || line.includes("Sample.") || line.includes("S2 "));
}

// Each step must raise the owner's coins and the total by the gain the client reports, in order.
export function checkFlow(steps, multiplier) {
	const failures = [];
	let previous = steps[0];
	if (!previous) return ["no HUD reading"];
	if (previous.coins !== 0 || previous.total !== 0) failures.push(`start is ${JSON.stringify(previous)}`);
	for (const step of steps.slice(1)) {
		if (step.last !== multiplier) failures.push(`last gain ${step.last} is not ${multiplier}`);
		if (step.coins !== previous.coins + multiplier) failures.push(`coins ${previous.coins} -> ${step.coins}`);
		if (step.total !== previous.total + multiplier) failures.push(`total ${previous.total} -> ${step.total}`);
		previous = step;
	}
	return failures;
}
