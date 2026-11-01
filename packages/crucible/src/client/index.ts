import type { BootOptions } from "../index";
import type { AskResult, AskServer, PayloadArgs, ToServer } from "../network";
import { boot as bootSide } from "../internal/boot";
import { ClientSession } from "../internal/network/client-session";
import { networkPort } from "../internal/network/port";

if (!game.GetService("RunService").IsClient()) error("@trioforge/crucible/client loaded on the server");

export const BOOTED_ATTRIBUTE = "CrucibleBooted";

let session: ClientSession | undefined;

function active(): ClientSession {
	return session ?? error("Crucible networking is not ready; send from a constructor, a hook, or later", 0);
}

export function boot(options: BootOptions) {
	const network = new ClientSession(networkPort());
	session = network;
	bootSide(
		{
			name: "client",
			renders: true,
			booted: () => {
				const player = game.GetService("Players").LocalPlayer;
				player.SetAttribute(BOOTED_ATTRIBUTE, true);
				return () => player.SetAttribute(BOOTED_ATTRIBUTE, undefined);
			},
			network,
		},
		options,
	);
}

export namespace Network {
	/** True when accepted locally: fired now, or queued until this client is ready. Never delivery. */
	export function send<P>(message: ToServer<P>, ...payload: PayloadArgs<NoInfer<P>>): boolean {
		return active().send(message, (payload as unknown[])[0]);
	}

	/** Yields until the reply, the declared timeout or the session ending. */
	export function ask<Request, Response>(
		message: AskServer<Request, Response>,
		...request: PayloadArgs<NoInfer<Request>>
	): AskResult<Response> {
		return active().ask(message, (request as unknown[])[0]) as AskResult<Response>;
	}
}
