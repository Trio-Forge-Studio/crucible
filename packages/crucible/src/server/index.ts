import type { BootOptions } from "../index";
import type { PayloadArgs, ToClient } from "../network";
import { boot as bootSide } from "../internal/boot";
import { installConfigRuntime } from "../internal/configs/runtime";
import { networkPort } from "../internal/network/port";
import { ServerSession } from "../internal/network/server-session";
import { SyncFrameMessage } from "../internal/sync/frame";
import { installServerSync } from "../internal/sync/server-sync";

if (!game.GetService("RunService").IsServer()) error("@trioforge/crucible/server loaded on the client");

let session: ServerSession | undefined;

export { config, playerConfig, configReady, refreshConfigs } from "../configs";
export type { Config, PlayerConfig } from "../configs";
export { publish } from "./publish";
export type { Publication } from "./publish";

function active(): ServerSession {
	return session ?? error("Crucible networking is not ready; send from a constructor, a hook, or later", 0);
}

installConfigRuntime();

export function boot(options: BootOptions) {
	const network = new ServerSession(networkPort());
	session = network;
	installServerSync((player, frame) => active().send(player, SyncFrameMessage, frame));
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
