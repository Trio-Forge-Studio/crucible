# Logging

Write game logs through named loggers with levels, structured context, and custom sinks.

Import logging from the root of `@trioforge/crucible`. The root is safe to import on the server and the client, and logging works at module load, before `boot` runs.

```ts
import { Logging, LogLevel } from "@trioforge/crucible";
import type { Logger, LogRecord, LogSink, SinkSubscription, LogContext, LogProperties } from "@trioforge/crucible";
```

## Create a logger

Call `Logging.getLogger` once per module, at the top level, with an explicit dotted name. The name is the record's `source`, and the default output prints it in brackets.

```ts
import { Logging } from "@trioforge/crucible";

const log = Logging.getLogger("Delivery.Van");
```

A name is 1 to 256 bytes of dot-separated, non-empty segments with no control characters. `getLogger` throws on any other name, such as `""`, `"Van."`, or `"Delivery..Van"`. A throw at the top level stops the module from loading.

## Write a record

A `Logger` has one method per level: `debug`, `info`, `warn`, and `error`. Each takes a template and an optional context table.

```ts
import { Logging } from "@trioforge/crucible";

const log = Logging.getLogger("Delivery.Van");
const van = new Instance("Model");

log.info("Van {van} loaded {count} parcels", { van, count: 3 });
log.warn("Route {route} has no stops", { route: "North" });
```

The renderer replaces each `{key}` with the context value for `key`. `{{` prints `{` and `}}` prints `}`. A `{key}` with no matching context value stays in the message as written.

Context values become the record's `context` when the call passes the level check:

- Strings, numbers, and booleans keep their value.
- An `Instance` becomes its full name from `GetFullName()`, such as `Workspace.Vans.Van1`.
- Any other value becomes `tostring(value)`. A nested table prints as its address, so pass flat values.

The record keeps a frozen copy of the context. Changing the table after the call does not change the record.

Keep the template constant and put the variable parts in the context. The template is the record's `template`, so sinks can group records by it. Text built into the template with `${}` can contain braces that the renderer reads as placeholders.

## Levels and the default output

`LogLevel` has five members in this order: `Debug`, `Info`, `Warning`, `Error`, and `Off`. The method for `Warning` is `warn`. `Off` is only a sink minimum. No record has that level.

`Logging.defaultSink` is the subscription of the built-in sink that writes to the Roblox output. Its minimum is `LogLevel.Debug` in Studio and `LogLevel.Info` in live games. Each line reads `[<logger name>] <message>`.

| Level | Default output |
| --- | --- |
| `Debug` | A plain line through the LogService logger `Game`, with the context attached |
| `Info` | An info line through the logger `Game`, with the context attached |
| `Warning` | A warning line through the logger `Game`, with the context attached |
| `Error` | A red error with a stack traceback that starts at the `log.error` call |

If the LogService loggers fail, the default sink falls back to `print` and `warn` for the rest of the session.

`log.error` does not throw. It writes the error from a separate thread, and the calling code continues. To stop the calling code, call `error()` or `throw` after you log.

To change what the default output shows, change its minimum or disconnect it.

```ts
import { Logging, LogLevel } from "@trioforge/crucible";

Logging.defaultSink.setMinimum(LogLevel.Warning);
```

`Logging.defaultSink.disconnect()` removes the default output for the rest of the session. Custom sinks still receive records.

## Skip work for disabled levels

A call below the minimum of every connected sink returns before it reads the context, renders the message, or creates a record. The context argument itself is still evaluated, because it is an ordinary function argument. If building the context is expensive, check the level first.

```ts
import { Logging, LogLevel } from "@trioforge/crucible";

const log = Logging.getLogger("Delivery.Route");
const stops = new Array<Vector3>();

if (log.isEnabled(LogLevel.Debug)) {
	log.debug("Route stops {stops}", { stops: stops.map((stop) => tostring(stop)).join(", ") });
}
```

`isEnabled(level)` returns `true` when at least one connected sink accepts `level`.

## Add a sink

A sink is an object with a `write` function. `Logging.addSink` registers it and returns a `SinkSubscription`. The minimum defaults to `LogLevel.Info`.

```ts
import { Logging, LogLevel } from "@trioforge/crucible";
import type { LogRecord } from "@trioforge/crucible";

const recent = new Array<LogRecord>();

const subscription = Logging.addSink(
	{
		write: (record) => {
			recent.push(record);
			if (recent.size() > 100) recent.shift();
		},
	},
	LogLevel.Warning,
);

subscription.setMinimum(LogLevel.Error);
subscription.disconnect();
```

Every record goes to each connected sink whose minimum is at or below the record's level, in the order the sinks were added. The call to `log.info` or another level method runs the sinks before it returns.

Sinks receive only game records, from loggers that `getLogger` created. Crucible's own runtime diagnostics go to the Roblox output, never to sinks.

These rules apply to a sink:

- `write` must not yield. A sink that calls `task.wait` or `InvokeServer`, or makes an HTTP or DataStore request, is disconnected on its first record. To do yielding work, pass the record to `task.defer`. The record is frozen, so it is safe to keep.
- If `write` throws or yields, Crucible disconnects that sink, prints a `[Crucible] Log sink failed` error, and still delivers the record to the other sinks.
- The record and its `context` are frozen. A `write` that assigns to them throws, so Crucible disconnects that sink.
- A log call made while a record is being delivered is dropped. A sink cannot log through another logger.
- `write` must be a function property. `addSink` throws when it is missing.

`addSink` with a sink object that is already connected returns its existing subscription and ignores the new minimum. Use `setMinimum` to change it. After `disconnect`, the subscription does nothing, and `addSink` with the same object returns a new subscription. `disconnect` can be called more than once.

`setMinimum` accepts any `LogLevel`. `LogLevel.Off` keeps the sink connected but sends it no records. `setMinimum` and `addSink` throw on a value that is not a `LogLevel`.

## Types

`LogRecord` is the frozen record that sinks receive.

| Field | Type | Value |
| --- | --- | --- |
| `sequence` | `number` | Counts records across all loggers, starting at 1 |
| `time` | `number` | `os.clock()` when the record was created |
| `source` | `string` | The logger name |
| `level` | `LogLevel` | `Debug`, `Info`, `Warning`, or `Error` |
| `template` | `string` | The template as passed |
| `context` | `LogProperties` | The normalized context |
| `message` | `string` | The rendered template |
| `traceback` | `string` or `undefined` | The caller's stack on `Error` records, `undefined` on the others |

- `LogContext` is `Readonly<Record<string, unknown>>`, the context a logger method accepts.
- `LogProperties` is `Readonly<Record<string, string | number | boolean>>`, the normalized context on a record.
- `LogSink` is `{ readonly write: (record: LogRecord) => void }`.
- `SinkSubscription` has `setMinimum(level: LogLevel): void` and `disconnect(): void`.
- `Logger` has `isEnabled(level)` and the four level methods. `isEnabled` does not accept `LogLevel.Off`.
