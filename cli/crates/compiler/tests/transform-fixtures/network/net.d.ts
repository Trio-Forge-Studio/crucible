// v2 declaration block, verbatim from networking-api.md "Declarations" (both facades in one file for probing).
declare const message: unique symbol;
declare const width: unique symbol;

export type f64 = number & { readonly [width]?: "f64" };
export type f32 = number & { readonly [width]?: "f32" };
export type u8 = number & { readonly [width]?: "u8" };
export type u16 = number & { readonly [width]?: "u16" };
export type u32 = number & { readonly [width]?: "u32" };

export interface ToServer<P> {
	readonly [message]: { readonly kind: "to-server"; readonly payload: (value: P) => P };
}
export interface ToClient<P> {
	readonly [message]: { readonly kind: "to-client"; readonly payload: (value: P) => P };
}
export interface AskServer<Request, Response> {
	readonly [message]: {
		readonly kind: "ask-server";
		readonly request: (value: Request) => Request;
		readonly response: (value: Response) => Response;
	};
}
export interface ClientReady {
	readonly [message]: { readonly kind: "client-ready" };
}
export declare const ClientReady: ClientReady;

export interface Receiver {
	readonly [message]: { readonly kind: "receiver" };
}

export type PayloadArgs<P> = [P] extends [undefined] ? [] : [payload: P];
export type AskFailure = "timeout" | "disconnected" | "unavailable" | "handler-error" | "overloaded" | "invalid";
export type AskResult<T> =
	| { readonly ok: true; readonly value: T }
	| { readonly ok: false; readonly reason: AskFailure };
export interface AskOptions {
	readonly timeout?: number;
}

export declare function toServer<P>(name: string): ToServer<P>;
export declare function toClient<P>(name: string): ToClient<P>;
export declare function askServer<Request, Response>(name: string, options?: AskOptions): AskServer<Request, Response>;

export type ReceiveTarget = ToServer<any> | ToClient<any> | AskServer<any, any> | ClientReady;
export type ReceiveCallback<T extends ReceiveTarget> =
	T extends ToServer<infer P>
		? (player: Player, ...payload: PayloadArgs<P>) => void
		: T extends AskServer<infer Request, infer Response>
			? (player: Player, ...request: PayloadArgs<Request>) => Response
			: T extends ToClient<infer P>
				? (...payload: PayloadArgs<P>) => void
				: (player: Player) => (() => void) | void;
export declare function receive<T extends ReceiveTarget>(target: T, callback: ReceiveCallback<T>): Receiver;

// @trioforge/crucible/server
export declare namespace ServerNetwork {
	function send<P>(player: Player, message: ToClient<P>, ...payload: PayloadArgs<NoInfer<P>>): boolean;
	function broadcast<P>(message: ToClient<P>, ...payload: PayloadArgs<NoInfer<P>>): void;
}
// @trioforge/crucible/client
export declare namespace ClientNetwork {
	function send<P>(message: ToServer<P>, ...payload: PayloadArgs<NoInfer<P>>): boolean;
	function ask<Request, Response>(
		message: AskServer<Request, Response>,
		...request: PayloadArgs<NoInfer<Request>>
	): AskResult<Response>;
}

// v1 for comparison: no NoInfer, and the four receive overloads in v1 order.
export declare namespace V1Server {
	function send<P>(player: Player, message: ToClient<P>, ...payload: PayloadArgs<P>): boolean;
	function broadcast<P>(message: ToClient<P>, ...payload: PayloadArgs<P>): void;
}
export declare namespace V1Client {
	function send<P>(message: ToServer<P>, ...payload: PayloadArgs<P>): boolean;
	function ask<Request, Response>(
		message: AskServer<Request, Response>,
		...request: PayloadArgs<Request>
	): AskResult<Response>;
}
export declare function receiveV1<P>(
	message: ToServer<P>,
	callback: (player: Player, ...payload: PayloadArgs<P>) => void,
): Receiver;
export declare function receiveV1<Request, Response>(
	message: AskServer<Request, Response>,
	callback: (player: Player, ...request: PayloadArgs<Request>) => Response,
): Receiver;
export declare function receiveV1(target: ClientReady, callback: (player: Player) => (() => void) | void): Receiver;
export declare function receiveV1<P>(message: ToClient<P>, callback: (...payload: PayloadArgs<P>) => void): Receiver;
