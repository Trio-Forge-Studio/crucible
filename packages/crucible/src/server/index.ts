import type { BootOptions } from "../index";
import { boot as bootSide } from "../internal/boot";
import { installConfigRuntime } from "../internal/configs/runtime";

if (!game.GetService("RunService").IsServer()) error("@trioforge/crucible/server loaded on the client");

installConfigRuntime();

export { config, playerConfig, configReady, refreshConfigs } from "../configs";
export type { Config, PlayerConfig } from "../configs";

export function boot(options: BootOptions) {
	bootSide({ name: "server", renders: false, booted: () => {} }, options);
}
