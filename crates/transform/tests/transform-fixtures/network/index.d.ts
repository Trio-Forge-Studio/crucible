declare const message: unique symbol;
declare const width: unique symbol;

export type f64 = number & { readonly [width]?: "f64" }; // same encoding as plain number
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
export const ClientReady: ClientReady;

export interface Receiver {
	readonly [message]: { readonly kind: "receiver" }; // opaque claim, not callable
}

export type PayloadArgs<P> = [P] extends [undefined] ? [] : [payload: P];
export type AskFailure = "timeout" | "disconnected" | "unavailable" | "handler-error" | "overloaded" | "invalid";
export type AskResult<T> =
	| { readonly ok: true; readonly value: T }
	| { readonly ok: false; readonly reason: AskFailure };
export interface AskOptions {
	readonly timeout?: number; // seconds, numeric literal
}

export function toServer<P>(name: string): ToServer<P>;
export function toClient<P>(name: string): ToClient<P>;
export function askServer<Request, Response>(name: string, options?: AskOptions): AskServer<Request, Response>;

export type ReceiveTarget = ToServer<any> | ToClient<any> | AskServer<any, any> | ClientReady;
export type ReceiveCallback<T extends ReceiveTarget> =
	T extends ToServer<infer P>
		? (player: Player, ...payload: PayloadArgs<P>) => void
		: T extends AskServer<infer Request, infer Response>
			? (player: Player, ...request: PayloadArgs<Request>) => Response
			: T extends ToClient<infer P>
				? (...payload: PayloadArgs<P>) => void
				: (player: Player) => (() => void) | void;
export function receive<T extends ReceiveTarget>(target: T, callback: ReceiveCallback<T>): Receiver;
