import { ToClient, ToServer, AskServer, PayloadArgs, AskResult } from "../network";

export namespace Network {
	function send<P>(message: ToServer<P>, ...payload: PayloadArgs<NoInfer<P>>): boolean;
	function ask<Request, Response>(
		message: AskServer<Request, Response>,
		...request: PayloadArgs<NoInfer<Request>>
	): AskResult<Response>; // yields
}
