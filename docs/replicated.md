# Replicated state

Declare game state once in shared code, write it from one server Subsystem, and read it on every client.

Import the declarations and `syncStatus` from the root of `@trioforge/crucible`. Import `publish` from `@trioforge/crucible/server`. The client entry adds nothing: clients only read.

```ts
import { replicated, replicatedPerPlayer, syncStatus } from "@trioforge/crucible";
import type { Replicated, ReplicatedPerPlayer, SyncStatus } from "@trioforge/crucible";
import { publish } from "@trioforge/crucible/server";
import type { Publication } from "@trioforge/crucible/server";
```

## Declare a value

`replicated<T>(name, initial)` declares a value every client receives. `replicatedPerPlayer<T>(name, initial)` declares a value each player receives alone.

```ts
import { replicated, replicatedPerPlayer } from "@trioforge/crucible";

export type JobPhase = "Waiting" | "Running" | "Done";

export const Phase = replicated<JobPhase | undefined>("Job.Phase", undefined);
export const Coins = replicatedPerPlayer<number>("Data.Coins", 0);
```

Both are transformer macros. Write each one as the initializer of an exported top-level `const` in a shared module, with an explicit type argument and a string literal name. The build rejects any other form.

- The name is the value's identity on the wire and in diagnostics. It follows the message name rules: 1 to 64 bytes of dot-separated segments, each `[A-Za-z][A-Za-z0-9_]*`. The first segment cannot be `Crucible`. Each name belongs to one declaration in the game.
- The type must describe plain data: booleans, numbers, strings, `Vector3`, `Color3`, `CFrame`, objects, arrays, tuples, string-keyed maps and unions the networking lowering accepts. Instances, `unknown`, functions and numeric map keys are build errors. Replicate a stable id instead of an Instance.
- An optional field, or a map value that admits `undefined`, is an absent key.

A declaration is a plain function. Call it to read the value. Inside an effect or a computed, the read tracks.

## Publish a value

`publish(declaration)` takes the write side of a value every client receives. Call it while a server Subsystem is constructed, as a field initializer. It returns a `State`.

```ts
import { replicated, Subsystem } from "@trioforge/crucible";
import { publish } from "@trioforge/crucible/server";

export const Score = replicated<number>("Job.Score", 0);

export class JobService extends Subsystem {
	private readonly score = publish(Score);

	addPoints(points: number) {
		this.score.update((current) => current + points);
	}
}
```

- One Subsystem publishes each declaration. A second `publish` of the same declaration fails boot and names both Subsystems.
- `publish` anywhere other than a server Subsystem's construction throws.
- `set` and `update` check the value against the declared type before they store it. Fields the type does not declare are dropped and never sent. A value that cannot replicate throws at the write and names the declaration and the path, never the value: `Data.Profile.settings.music: expected boolean, got number`.
- Every table in a published value is frozen. Code that later mutates one of those tables in place throws at that line.
- Every server reader sees the new value at once. Clients see it in the next Heartbeat's frame. Several writes between two Heartbeats send only the last value.

`publish(declaration, project)` supplies a per-player value as a projection. It returns an opaque `Publication`.

```ts
import { replicatedPerPlayer, state, Subsystem } from "@trioforge/crucible";
import { publish } from "@trioforge/crucible/server";

export const Coins = replicatedPerPlayer<number | undefined>("Data.Coins", undefined);

const balances = state<ReadonlyMap<number, number>>(new Map());

export class WalletService extends Subsystem {
	private readonly coins = publish(Coins, (player) => balances().get(player.UserId));
}
```

For each player whose client is ready, the projection runs like a `computed`: it reruns when a value it read changes. Each player receives only their own value. The projection must not write state, create effects or yield. It returns `undefined` until its data exists. A projection that throws, or returns a value that cannot replicate, marks that player's value failed until its next accepted value.

## Read a value

On a client, a declaration reads the last value the server sent, or `initial` before the first frame. A per-player declaration reads the local player's value. Clients cannot write a declaration. The first frame reaches a client before any message a game `ClientReady` callback sends it, so a handler for such a message already reads the published values.

On the server, a public declaration reads the authoritative value. Calling a per-player declaration on the server throws, because it has no single value. Read the sources its projection reads instead.

`syncStatus(declaration)` returns the declaration's `SyncStatus` and tracks like a value:

| Status | Client | Server, public declaration |
| --- | --- | --- |
| `"pending"` | No frame has carried its value yet. | No Subsystem publishes it. |
| `"ready"` | It holds the value the server sent. | Its publisher is live. |
| `"failed"` | Its publisher ended, its projection failed for this player, or this client's sync stopped. The last value is kept. | Its publisher ended. |

```ts
import { computed, replicated, syncStatus } from "@trioforge/crucible";

export const Phase = replicated<string | undefined>("Job.Phase", undefined);

export const loading = computed(() => syncStatus(Phase) === "pending");
```

A per-player declaration that failed becomes `"ready"` again when its next value arrives. A client whose sync stopped stays `"failed"` until the player rejoins.

## What reaches a client

- The server sends each ready player one frame per Heartbeat at most, holding every value that changed since the last frame that player accepted. A frame is atomic: an effect on the client never sees part of it.
- A value equal to the one the player already holds is not resent.
- A declaration nobody publishes is never sent. Its clients keep `initial` with status `"pending"`.
- When a publishing Subsystem faults, its declarations become `"failed"` for every client, and writes through its handle throw.
- A frame over the sync budgets, or a client that reports a broken frame, stops that player's sync and kicks the player with "Connection problem, please rejoin." No other player is affected.

Sync is state, not an event log. Send a message when a client needs every transition.
