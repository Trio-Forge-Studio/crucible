import { ToServer, ToClient, AskServer, Receiver, ReceiveTarget, ReceiveCallback } from "../network";
import { Node } from "./schema";
export { n } from "./schema";
export declare function toServer<P>(name: string, payload: Node): ToServer<P>;
export declare function toClient<P>(name: string, payload: Node): ToClient<P>;
export declare function askServer<Q, R>(
	name: string,
	request: Node,
	response: Node,
	timeout: number | undefined,
): AskServer<Q, R>;
export declare function receive<T extends ReceiveTarget>(
	owner: object,
	field: string,
	target: T,
	callback: ReceiveCallback<T>,
): Receiver;
