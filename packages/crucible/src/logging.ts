import { LogLevel } from "./log-level";
import { runtimeDiagnostic } from "./internal/diagnostics";
import { loggingPort } from "./internal/log-output";
import { call, CallResult } from "./internal/runner";

export { LogLevel } from "./log-level";

export type LogContext = Readonly<Record<string, unknown>>;
export type LogProperties = Readonly<Record<string, string | number | boolean>>;

export interface LogRecord {
	readonly sequence: number;
	readonly time: number;
	readonly source: string;
	readonly level: Exclude<LogLevel, LogLevel.Off>;
	readonly template: string;
	readonly context: LogProperties;
	readonly message: string;
	readonly traceback?: string;
}

export interface LogSink {
	readonly write: (record: LogRecord) => void;
}

export interface SinkSubscription {
	setMinimum(level: LogLevel): void;
	disconnect(): void;
}

export interface Logger {
	isEnabled(level: Exclude<LogLevel, LogLevel.Off>): boolean;
	debug(template: string, context?: LogContext): void;
	info(template: string, context?: LogContext): void;
	warn(template: string, context?: LogContext): void;
	error(template: string, context?: LogContext): void;
}

interface Registration {
	readonly sink: LogSink;
	readonly subscription: SinkSubscription;
	minimum: LogLevel;
	connected: boolean;
}

const registrations = new Map<LogSink, Registration>();
let snapshot: readonly Registration[] = [];
let minimum = LogLevel.Off;
let sequence = 0;
let emitting = false;

function derive(added?: Registration) {
	const entries = new Array<Registration>();
	minimum = LogLevel.Off;
	// roblox-ts Maps compile to unordered Luau tables. The array owns delivery order.
	for (const registration of snapshot) {
		if (!registration.connected) continue;
		entries.push(registration);
		minimum = math.min(minimum, registration.minimum);
	}
	if (added !== undefined) {
		entries.push(added);
		minimum = math.min(minimum, added.minimum);
	}
	snapshot = table.freeze(entries);
}

function validateMinimum(level: LogLevel) {
	assert(level >= LogLevel.Debug && level <= LogLevel.Off && level % 1 === 0, "Invalid log minimum");
}

function register(sink: LogSink, level: LogLevel): SinkSubscription {
	validateMinimum(level);
	const existing = registrations.get(sink);
	if (existing !== undefined) return existing.subscription;
	assert(typeIs(sink.write, "function"), "Log sink requires write");
	const registration: Registration = {
		sink,
		minimum: level,
		connected: true,
		subscription: {
			setMinimum(value) {
				validateMinimum(value);
				if (!registration.connected) return;
				registration.minimum = value;
				derive();
			},
			disconnect() {
				if (!registration.connected) return;
				registration.connected = false;
				registrations.delete(sink);
				derive();
			},
		},
	};
	registrations.set(sink, registration);
	derive(registration);
	return registration.subscription;
}

function normalize(input?: LogContext): LogProperties {
	const context: Record<string, string | number | boolean> = {};
	if (input !== undefined) {
		for (const [key, value] of pairs(input)) {
			context[key] = typeIs(value, "Instance")
				? loggingPort().describeInstance(value)
				: typeIs(value, "string") || typeIs(value, "number") || typeIs(value, "boolean")
					? value
					: tostring(value);
		}
	}
	return table.freeze(context);
}

function render(template: string, context: LogProperties): string {
	const parts = new Array<string>();
	let index = 1;
	while (index <= template.size()) {
		const char = template.sub(index, index);
		const pair = template.sub(index, index + 1);
		if (pair === "{{" || pair === "}}") {
			parts.push(char);
			index += 2;
		} else if (char === "{") {
			const [close] = template.find("}", index + 1, true);
			if (close !== undefined) {
				const key = template.sub(index + 1, close - 1);
				const value = context[key];
				parts.push(value === undefined ? template.sub(index, close) : tostring(value));
				index = close + 1;
			} else {
				parts.push(char);
				index++;
			}
		} else {
			parts.push(char);
			index++;
		}
	}
	return parts.join("");
}

function emit(source: string, level: Exclude<LogLevel, LogLevel.Off>, template: string, input?: LogContext) {
	if (emitting || level < minimum) return;
	emitting = true;
	const trace = level === LogLevel.Error ? debug.traceback("", 3) : undefined;
	const [result, failure] = call(
		() => {
			const accepted = snapshot.filter((r) => level >= r.minimum);
			const context = normalize(input);
			const record: LogRecord = table.freeze({
				sequence: ++sequence,
				time: loggingPort().now(),
				source,
				level,
				template,
				context,
				message: render(template, context),
				traceback: trace,
			});
			for (const registration of accepted) {
				if (!registration.connected) continue;
				const [status, err] = call(registration.sink.write, record, undefined);
				if (status !== CallResult.Ok) {
					registration.subscription.disconnect();
					runtimeDiagnostic("error", `Log sink failed: ${tostring(err)}`);
				}
			}
		},
		undefined,
		undefined,
	);
	if (result !== CallResult.Ok) runtimeDiagnostic("error", `Logging failed: ${tostring(failure)}`);
	emitting = false;
}

export class Logging {
	private constructor() {}

	static readonly defaultSink = register(
		{ write: (record) => loggingPort().writeGame(record) },
		loggingPort().isStudio() ? LogLevel.Debug : LogLevel.Info,
	);

	static getLogger(source: string): Logger {
		assert(source.size() > 0 && source.size() <= 256 && source.find("%c")[0] === undefined, "Invalid logger name");
		for (const segment of source.split(".")) assert(segment.size() > 0, "Invalid logger name segment");
		return {
			isEnabled(level) {
				return level >= minimum;
			},
			debug(template, context) {
				emit(source, LogLevel.Debug, template, context);
			},
			info(template, context) {
				emit(source, LogLevel.Info, template, context);
			},
			warn(template, context) {
				emit(source, LogLevel.Warning, template, context);
			},
			error(template, context) {
				emit(source, LogLevel.Error, template, context);
			},
		};
	}

	static addSink(sink: LogSink, minimum = LogLevel.Info): SinkSubscription {
		return register(sink, minimum);
	}
}
