import { setTimeout as delay } from "node:timers/promises";
import { parseHud } from "./evidence.mjs";

const HUD_CODE = `
local gui = game:GetService("Players").LocalPlayer:FindFirstChild("PlayerGui")
local screen = gui and gui:FindFirstChild("CoinHud")
local label = screen and screen:FindFirstChild("Coins")
return label and label.Text or "missing"
`;

export const markerPattern = (runId) => new RegExp(`S1\\|${runId}\\|client\\|marker`);

// Drives an owned play test: prints a marker from each side, waits for the HUD, then presses E and reads the HUD.
// `ctx.luau(datamodel, code)` and `ctx.call(tool, args)` talk to the session the caller proved is its own.
export async function playFlow(ctx, { runId, presses, expectedGain }) {
	const hud = async () => parseHud(await ctx.luau("Client", HUD_CODE));
	await ctx.luau("Server", `print("S1|${runId}|server|marker")`);
	await ctx.luau("Client", `print("S1|${runId}|client|marker")`);
	if (!(await ctx.waitFor(markerPattern(runId), 30000))) throw Error("Client marker missing");
	const readings = [];
	const record = (step, reading) => readings.push({ at: new Date().toISOString(), step, ...reading });
	const loaded = Date.now() + 90000;
	let first;
	while (Date.now() < loaded) {
		first = await hud();
		if (first) break;
		await delay(1000);
	}
	if (!first) throw Error("HUD never appeared");
	record("loaded", first);
	let previous = first;
	for (let press = 1; press <= presses; press++) {
		await ctx.call("user_keyboard_input", {
			datamodel_type: "Client",
			actions: [{ action: "keyPress", key_code: "E" }],
		});
		const wait = Date.now() + 15000;
		let reading;
		while (Date.now() < wait) {
			reading = await hud();
			if (reading && reading.coins === previous.coins + expectedGain) break;
			await delay(500);
		}
		if (!reading) throw Error(`HUD missing after press ${press}`);
		record(`press-${press}`, reading);
		previous = reading;
	}
	return readings;
}
