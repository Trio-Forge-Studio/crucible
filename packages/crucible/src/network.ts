declare const message: unique symbol;
declare const width: unique symbol;

export type f64 = number & { readonly [width]?: "f64" };
export type f32 = number & { readonly [width]?: "f32" };
export type u8 = number & { readonly [width]?: "u8" };
export type u16 = number & { readonly [width]?: "u16" };
export type u32 = number & { readonly [width]?: "u32" };

// Function-valued phantom positions keep every payload type invariant under strictFunctionTypes.
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
export const ClientReady = table.freeze({}) as unknown as ClientReady;

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

// `any` is the only argument every invariant token is assignable to.
export type ReceiveTarget = ToServer<any> | ToClient<any> | AskServer<any, any> | ClientReady;
export type ReceiveCallback<T extends ReceiveTarget> =
	T extends ToServer<infer P>
		? (player: Player, ...payload: PayloadArgs<P>) => void
		: T extends AskServer<infer Request, infer Response>
			? (player: Player, ...request: PayloadArgs<Request>) => Response
			: T extends ToClient<infer P>
				? (...payload: PayloadArgs<P>) => void
				: (player: Player) => (() => void) | void;

const UNTRANSFORMED = "Crucible networking needs the @trioforge/crucible/transformer plugin";

export function toServer<P>(_name: string): ToServer<P> {
	error(UNTRANSFORMED);
}

export function toClient<P>(_name: string): ToClient<P> {
	error(UNTRANSFORMED);
}

export function askServer<Request, Response>(_name: string, _options?: AskOptions): AskServer<Request, Response> {
	error(UNTRANSFORMED);
}

export function receive<T extends ReceiveTarget>(_target: T, _callback: ReceiveCallback<T>): Receiver {
	error(UNTRANSFORMED);
}
