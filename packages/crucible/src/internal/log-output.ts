import type { LogRecord } from "../logging";

/** @internal */
export interface LoggingPort {
	readonly isStudio: () => boolean;
	readonly now: () => number;
	readonly describeInstance: (instance: Instance) => string;
	readonly writeGame: (record: LogRecord) => void;
	readonly writeInternal: (level: "warning" | "error", message: string) => void;
}

let port: LoggingPort | undefined;

/** @internal */
export function installLoggingPort(value: LoggingPort) {
	assert(port === undefined, "Logging port already selected");
	port = value;
}

/** @internal */
export function loggingPort(): LoggingPort {
	port ??= createRobloxLoggingPort();
	return port;
}

function escape(text: string): string {
	return text.gsub("{", "{{")[0].gsub("}", "}}")[0];
}

/** @internal */
export function createRobloxLoggingPort(): LoggingPort {
	let gameLogger: Logger | undefined;
	let crucibleLogger: Logger | undefined;
	let fallback = false;
	const studio = game.GetService("RunService").IsStudio();
	function native(write: () => void, plain: () => void) {
		if (!fallback) {
			const [ok] = pcall(() => {
				const service = game.GetService("LogService");
				gameLogger ??= service.GetLogger("Game");
				crucibleLogger ??= service.GetLogger("Crucible");
				write();
			});
			if (ok) return;
			fallback = true;
		}
		plain();
	}
	return {
		isStudio: () => studio,
		now: () => os.clock(),
		describeInstance: (instance) => instance.GetFullName(),
		writeGame: (record) => {
			const text = `[${record.source}] ${record.message}`;
			if (record.level === 3) {
				task.spawn(() => error(`${text}\n${record.traceback ?? ""}`, 0));
				return;
			}
			native(
				() => {
					const logger = gameLogger!;
					if (record.level === 0) logger.Output(escape(text), record.context);
					else if (record.level === 1) logger.Info(escape(text), record.context);
					else logger.Warn(escape(text), record.context);
				},
				() => {
					if (record.level === 2) warn(text);
					else print(text);
				},
			);
		},
		writeInternal: (level, message) => {
			if (level === "error") task.spawn(() => error(`[Crucible] ${message}`, 0));
			else
				native(
					() => crucibleLogger!.Warn(escape(`[Crucible] ${message}`)),
					() => warn(`[Crucible] ${message}`),
				);
		},
	};
}
