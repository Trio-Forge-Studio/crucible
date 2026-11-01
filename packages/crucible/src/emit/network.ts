import type { AskServer, Receiver, ToClient, ToServer } from "../network";
import { define } from "../internal/network/catalog";
import { claim } from "../internal/network/claims";
import type { Node } from "../internal/network/schema";

export function toServer<P>(name: string, payload: Node): ToServer<P> {
	return define({ kind: "to-server", name, input: payload }) as unknown as ToServer<P>;
}

export function toClient<P>(name: string, payload: Node): ToClient<P> {
	return define({ kind: "to-client", name, input: payload }) as unknown as ToClient<P>;
}

export function askServer<Request, Response>(
	name: string,
	request: Node,
	response: Node,
	timeout: number | undefined,
): AskServer<Request, Response> {
	return define({ kind: "ask-server", name, input: request, output: response, timeout }) as unknown as AskServer<
		Request,
		Response
	>;
}

export function receive(owner: object, field: string, target: unknown, callback: Callback): Receiver {
	return claim(owner, field, target, callback);
}
