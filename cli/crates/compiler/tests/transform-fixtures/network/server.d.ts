import { ToClient, ToServer, AskServer, PayloadArgs, AskResult } from "../network";

export namespace Network {
	function send<P>(player: Player, message: ToClient<P>, ...payload: PayloadArgs<NoInfer<P>>): boolean;
	function broadcast<P>(message: ToClient<P>, ...payload: PayloadArgs<NoInfer<P>>): void;
}
