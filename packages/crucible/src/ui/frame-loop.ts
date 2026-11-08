const RunService = game.GetService("RunService");

export type FrameListener = (now: number, delta: number) => void;

function loop(signal: RBXScriptSignal<(delta: number) => void>) {
	const listeners = new Set<FrameListener>();
	let order: FrameListener[] | undefined;
	let connection: RBXScriptConnection | undefined;
	return (listener: FrameListener): (() => void) => {
		listeners.add(listener);
		order = undefined;
		connection ??= signal.Connect((delta) => {
			order ??= [...listeners];
			const now = os.clock();
			for (const callback of order) {
				if (listeners.has(callback)) callback(now, delta);
			}
		});
		return () => {
			if (!listeners.delete(listener)) return;
			order = undefined;
			if (listeners.isEmpty()) {
				connection?.Disconnect();
				connection = undefined;
			}
		};
	};
}

export const heartbeat = loop(RunService.Heartbeat);
export const preRender = loop(RunService.PreRender);
