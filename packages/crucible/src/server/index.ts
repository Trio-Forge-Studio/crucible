import type { BootOptions } from "../index";
import { boot as bootSide } from "../internal/boot";

if (!game.GetService("RunService").IsServer()) error("@trioforge/crucible/server loaded on the client");

export const serverPlaceholder = "crucible/server";

export function boot(options: BootOptions) {
	bootSide({ name: "server", renders: false, booted: () => {} }, options);
}
