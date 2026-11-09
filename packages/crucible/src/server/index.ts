import type { BootOptions } from "../index";
import type { PayloadArgs, ToClient } from "../network";
import { boot as bootSide } from "../internal/boot";
import { installConfigRuntime } from "../internal/configs/runtime";
import { networkPort } from "../internal/network/port";
import { ServerSession } from "../internal/network/server-session";
import { installServerSync } from "../internal/sync/server-sync";
import "../internal/data/record-bindings";
import { installDataRuntime } from "../internal/data/runtime";

if (!game.GetService("RunService").IsServer()) error("@trioforge/crucible/server loaded on the client");

let session: ServerSession | undefined;

export const BOOTED_ATTRIBUTE = "CrucibleBooted";

export { config, playerConfig, configReady, refreshConfigs } from "../configs";
export type { Config, PlayerConfig } from "../configs";
export { publish } from "./publish";
export type { Publication, Published } from "./publish";
export { dataStore, operationId, profileMessage } from "./data";
export type { DataStore, StoreOptions, UpdateResult } from "./data";
export type { OperationId, StoreFailure, StoreResult } from "../data";
export type {
	ProfileMessage,
	ProfileClaim,
	LoadReason,
	ReleaseReason,
	Purchase,
	AfterGrant,
	QuarantinedMessage,
	SaveResult,
	SaveFailure,
} from "../data";

function active(): ServerSession {
	return session ?? error("Crucible networking is not ready; send from a constructor, a hook, or later", 0);
}

installConfigRuntime();
installDataRuntime();

export function boot(options: BootOptions) {
	const network = new ServerSession(networkPort());
	session = network;
	installServerSync((player, _frame, packet) => active().sendEncoded(player, packet));
	bootSide(
		{
			name: "server",
			renders: false,
			booted: () => {
				const service = game.GetService("ServerScriptService");
				service.SetAttribute(BOOTED_ATTRIBUTE, true);
				return () => service.SetAttribute(BOOTED_ATTRIBUTE, undefined);
			},
			network,
		},
		options,
	);
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
