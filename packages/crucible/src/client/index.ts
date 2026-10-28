import type { BootOptions } from "../index";
import { boot as bootSide } from "../internal/boot";

if (!game.GetService("RunService").IsClient()) error("@trioforge/crucible/client loaded on the server");

export const BOOTED_ATTRIBUTE = "CrucibleBooted";

export function boot(options: BootOptions) {
	bootSide(
		{
			name: "client",
			renders: true,
			booted: () => game.GetService("Players").LocalPlayer.SetAttribute(BOOTED_ATTRIBUTE, true),
		},
		options,
	);
}
