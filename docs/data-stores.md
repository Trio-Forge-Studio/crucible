# Data stores

Keep keyed records in a Roblox DataStore with migrations, unknown-key handling and retry-safe updates.

Import data stores from `@trioforge/crucible/server`. They are server only. A data store holds records that belong to no player session, such as a command log or a guild table. It takes no lease and has no live handles: each call is one DataStore request that yields and can fail.

```ts
import { dataStore, operationId } from "@trioforge/crucible/server";
import type { DataStore, OperationId, StoreFailure, StoreOptions, StoreResult, UpdateResult } from "@trioforge/crucible/server";
import type { Json, JsonRecord, Migration } from "@trioforge/crucible";
```

## Declare a store

A declaration names the DataStore, the record's type and the record a new key starts from. Declare each one as an exported top-level `const` in a server module that boot loads, with the type argument written out.

```ts
import { dataStore } from "@trioforge/crucible/server";

export interface LogEntry {
	readonly id: string;
	readonly name: string;
	readonly at: number;
}

export interface CommandLog {
	readonly entries: readonly LogEntry[];
}

export const CommandLogStore = dataStore<CommandLog>("FD.CommandLog", { defaults: { entries: [] } });
```

`dataStore` is a compiler macro. The transformer plugin checks the declaration and adds the type's descriptor. Without the plugin's data support, a declaration throws "Crucible data needs the @trioforge/crucible/transformer plugin" when its module loads.

The record is an object or a string-keyed record at its root. Wrap a bare string or number in an object so a later field is a default and not a migration.

A name is up to 50 bytes of dot-separated segments that start with a letter and hold letters, digits and underscores. `Crucible` is reserved as the first segment. A name is the DataStore name. A duplicate name, `defaults` that do not match the type, more than 256 data declarations, or a declaration in a module that loads after boot throws when its module loads.

`defaults` are checked against the type and frozen once. A value that cannot be saved, such as a `Vector3`, an Instance or a number with a brand that is not one of Crucible's widths, is a compile error.

## Read a record

`read(key)` yields and returns the stored record, migrated and checked, without writing anything back.

```ts
import { dataStore } from "@trioforge/crucible/server";

interface Party {
	readonly members: readonly number[];
}

const PartyStore = dataStore<Party>("FD.Party", { defaults: { members: [] } });

const result = PartyStore.read("party-1");
if (result.ok) {
	print(result.value === undefined ? "no party" : `${result.value.members.size()} members`);
} else {
	warn(`party read failed: ${result.reason}`);
}
```

A key is 1 to 50 bytes; any other key throws. A key that holds nothing returns `undefined`, never the defaults. A record that cannot be read, or that a newer build saved, returns the reason `"unavailable"`. The value is frozen, and a table in it that your schema does not declare is still there.

## Update a record

`update(key, change, id?)` yields. `change` gets the current record, or the defaults for a key that holds nothing, and returns the next record. It runs inside the DataStore update, so it must be pure, can run more than once, and cannot yield or call another data write. A throw in `change` cancels the write and is thrown from `update`.

```ts
import { dataStore, operationId } from "@trioforge/crucible/server";

interface LogEntry {
	readonly id: string;
	readonly name: string;
}

interface CommandLog {
	readonly entries: readonly LogEntry[];
}

const CommandLogStore = dataStore<CommandLog>("FD.CommandLog", { defaults: { entries: [] } });

export function record(entry: LogEntry): boolean {
	const id = operationId();
	const result = CommandLogStore.update("Recent", (log) => ({ ...log, entries: [...log.entries, entry] }), id);
	return result.ok;
}
```

Create an id once per logical operation and pass it on every retry of that operation. The store remembers the last 256 ids for each key, so a call that retries after a lost reply finds its id and does not run `change` again. A replay returns `replayed: true` and the current stored record, which can include later writers' changes. Every other success has `replayed: false`.

```ts
import { dataStore, operationId } from "@trioforge/crucible/server";

interface Counter {
	readonly total: number;
}

const Counters = dataStore<Counter>("Game.Counters", { defaults: { total: 0 } });

export function add(amount: number): number | undefined {
	const id = operationId();
	for (let attempt = 0; attempt < 3; attempt++) {
		const result = Counters.update("daily", (counter) => ({ ...counter, total: counter.total + amount }), id);
		if (result.ok) return result.value.total;
		if (result.reason !== "timeout" && result.reason !== "uncertain") return undefined;
	}
	return undefined;
}
```

An update with no change still records its id. A record you return unchanged stores no new payload, and on a key that holds nothing it stores only the id, so the key still reads as `undefined`. A record saved at an older schema version is migrated and written even when `change` returns it unchanged.

An id that is older than an hour, or older than an id the store already evicted, is refused as `"expired"` before anything changes. When a key's 256 ids are all younger than 60 seconds, a new id is refused as `"overloaded"` and nothing is written. Only `operationId()` makes an id; a plain string does not type-check.

## Results

Every call returns a result you narrow on `ok`.

| Reason | Meaning |
| --- | --- |
| `"timeout"` | The call never reached DataStore within 30 seconds, so nothing was sent. |
| `"uncertain"` | The call was sent and no answer arrived. Retry the same id. |
| `"overloaded"` | The server's request queue is full, or the key's id ring is too young to evict. |
| `"closing"` | The server is shutting down and no longer sends generic requests. |
| `"unavailable"` | The record could not be read, a newer build saved it, or the storage error is not retryable. |
| `"expired"` | The id is too old. |

A server that is shutting down still sends generic requests until 1.5 seconds before core's shutdown deadline, so a final flush from an `onShutdown` hook is accepted.

## Change a schema

A record's version is the number of its migrations. Pass `migrations` after `defaults`; each step upgrades a record from one version to the next and runs only on records saved before it. Steps are append-only: removing or reordering one changes what every older record becomes.

```ts
import { dataStore } from "@trioforge/crucible/server";
import type { JsonRecord, Migration } from "@trioforge/crucible";

interface LogEntry {
	readonly id: string;
	readonly name: string;
	readonly ok: boolean;
}

interface CommandLog {
	readonly entries: readonly LogEntry[];
}

const addOk: Migration = (old) => {
	const entries = (old.entries ?? []) as readonly JsonRecord[];
	return { entries: entries.map((entry) => ({ ...entry, ok: true })) };
};

export const CommandLogStore = dataStore<CommandLog>("FD.CommandLog", {
	defaults: { entries: [] },
	migrations: [addOk],
});
```

A step gets a private copy of the stored record, unknown keys included, as plain JSON, and returns a patch:

- An object merges into the stored object, recursively.
- Any other value, an array included, replaces the stored one.
- An omitted key is kept. A step cannot delete a key, and a rename writes the new key and leaves the old one in place.

A step that throws, yields or returns something that is not a record makes the record unreadable, and the data is untouched. The log names the store and the step as `migrations[<index>]`, once per store and kind of failure.

The migrated value and its version are written in one request, so a migrated record is never stored under its old version, and a crash after migrating in memory reruns the same pure steps on the same input. Steps must be pure.

After the steps, the type is checked against the record. A value of the wrong shape, a non-finite number, a string over 65,536 bytes or invalid UTF-8, an array with holes, a table nested deeper than 28 levels, or more than 100,000 values makes the record unreadable. Nothing is coerced or reset. A key the type requires and the record lacks takes the default only along object fields from the root. Inside an array element, a map value or any other variant it is unreadable, and the log names the path. A missing optional key stays absent, so clearing one is durable.

Many changes need no step, so a rolling deploy can run two builds at one version. An optional field, a required field with a default on an object path from the root, a removed field, and a new store need none. A change inside an array element or a map value, a change of type, and a new variant or literal need a step. A build refuses a record saved at a newer version, so an older server never rewrites data a newer migration shaped: `read` and `update` return `"unavailable"`, and `update` logs that a newer build wrote the record.

## Keep what you do not know

A key your type does not declare stays in the record, at any depth, though the type does not show it. A spread copies it along: `{ ...entry, ok: true }` keeps every unknown key. A table you build from scratch holds only what you put in it, so `{ id: entry.id, ok: true }` drops them. Spread; do not rebuild.

When code rebuilds a map or an array from its own catalog, it deletes the entries a newer build added. Keep the entries you do not recognize and change only the ones you do.

## Size

The record is at most 1,000,000 bytes encoded. A result over that throws from `update`, naming the store and the key, and nothing is written. Together with its id ring and header, a record at that size fits one DataStore value.
