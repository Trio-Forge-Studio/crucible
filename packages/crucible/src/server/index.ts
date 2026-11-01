import type { BootOptions } from "../index";
import type { PayloadArgs, ToClient } from "../network";
import { boot as bootSide } from "../internal/boot";
import { installConfigRuntime } from "../internal/configs/runtime";
import { networkPort } from "../internal/network/port";
import { ServerSession } from "../internal/network/server-session";

if (!game.GetService("RunService").IsServer()) error("@trioforge/crucible/server loaded on the client");

installConfigRuntime();

export { config, playerConfig, configReady, refreshConfigs } from "../configs";
export type { Config, PlayerConfig } from "../configs";

let session: ServerSession | undefined;

function active(): ServerSession {
	return session ?? error("Crucible networking is not ready; send from a constructor, a hook, or later", 0);
}

export function boot(options: BootOptions) {
	const network = new ServerSession(networkPort());
	session = network;
	bootSide({ name: "server", renders: false, booted: () => {}, network }, options);
}

export namespace Network {
	/** True when accepted locally: fired now, or queued before the player's client is ready. Never delivery. */
	export function send<P>(player: Player, message: ToClient<P>, ...payload: PayloadArgs<NoInfer<P>>): boolean {
		return active().send(player, message, (payload as unknown[])[0]);
	}

	/** Encodes once and delivers the same packet to every player in the game now. */
	export function broadcast<P>(message: ToClient<P>, ...payload: PayloadArgs<NoInfer<P>>): void {
		active().broadcast(message, (payload as unknown[])[0]);
	}
}
