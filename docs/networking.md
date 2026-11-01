# Networking

Send typed messages between the server and its clients through declared messages, `receive` fields, and `Network`.

Declare messages and receivers with the root of `@trioforge/crucible`, which is safe on both sides. Send with the `Network` namespace of the side entry: `@trioforge/crucible/server` on the server and `@trioforge/crucible/client` on the client. Declarations and `receive` need the `@trioforge/crucible/transformer` plugin. Without it, each one throws "Crucible networking needs the @trioforge/crucible/transformer plugin" when its module loads.

```ts
import { askServer, ClientReady, receive, toClient, toServer } from "@trioforge/crucible";
import type { AskFailure, AskResult, AskServer, PayloadArgs, ToClient, ToServer } from "@trioforge/crucible";
import type { f32, f64, u8, u16, u32 } from "@trioforge/crucible";
```

## Declare messages

Declare each message as an exported top-level `const` in a shared module, with an explicit payload type. The name is the message's identity on the wire, so two declarations cannot share a name.

```ts
import { askServer, toClient, toServer } from "@trioforge/crucible";

export const DoorToggle = toServer<{ readonly door: Model; readonly open: boolean }>("Doors.Toggle");
export const DoorbellRang = toClient<{ readonly button: BasePart | undefined; readonly houseId: string }>(
	"Doorbell.Rang",
);
export const StandUp = toServer<undefined>("Seating.Stand");
export const Profile = askServer<{ readonly userId: number }, { readonly name: string } | undefined>(
	"PlayerCard.Profile",
	{ timeout: 30 },
);
```

Each declaration returns a token that carries the direction and the payload type:

- `toServer<P>(name)` returns a `ToServer<P>`. A client sends it and one server Subsystem receives it.
- `toClient<P>(name)` returns a `ToClient<P>`. The server sends it and one client Subsystem receives it.
- `askServer<Request, Response>(name, options?)` returns an `AskServer<Request, Response>`. A client asks it and one server Subsystem answers it.

A name is 1 to 64 bytes of dot-separated segments, each a letter followed by letters, digits, or underscores. The first segment `Crucible` is reserved. `askServer` options take only `timeout`, a number of seconds in (0, 120]. The default timeout is 10 seconds.

A payload of exactly `undefined` takes no argument and sends no bytes. `PayloadArgs<P>` is the argument list a token takes: `[]` for `undefined`, `[payload: P]` otherwise.

Payloads can use booleans, `number`, strings, literal types, `Vector3`, `Color3`, `CFrame`, Instance classes, objects, arrays, fixed tuples, `Map`, string-keyed records, unions, and `unknown`. A plain `number` travels as a 64-bit float. To send fewer bytes, use the width types:

| Type | Bytes | Values |
| --- | --- | --- |
| `f64` | 8 | Finite numbers, the same as `number` |
| `f32` | 4 | Finite numbers that stay finite as 32-bit floats |
| `u8` | 1 | Integers from 0 to 255 |
| `u16` | 2 | Integers from 0 to 65,535 |
| `u32` | 4 | Integers from 0 to 4,294,967,295 |

Strings must be valid UTF-8 of at most 65,536 bytes. An Instance in a server-to-client payload can stream out on the client, so declare it `T | undefined`, except a `Player`.

## Receive messages

A Subsystem receives a message through a `readonly` field initialized with `receive`. Each message has at most one receiver on each side. To reach behaviours, route from that one receiver, for example with `Door.get(instance)`.

```ts
import { ClientReady, receive, Subsystem, toServer } from "@trioforge/crucible";

const DoorToggle = toServer<{ readonly door: Model; readonly open: boolean }>("Doors.Toggle");

export class DoorService extends Subsystem {
	private readonly toggle = receive(DoorToggle, (player, { door, open }) => {
		print(player.Name, door.Name, open);
	});

	private readonly ready = receive(ClientReady, (player) => {
		print(`${player.Name} is ready`);
	});
}
```

The callback's shape follows the target:

| Target | Callback | Side |
| --- | --- | --- |
| `ToServer<P>` | `(player, ...payload) => void` | Server |
| `AskServer<Request, Response>` | `(player, ...request) => Response` | Server |
| `ToClient<P>` | `(...payload) => void` | Client |
| `ClientReady` | `(player) => (() => void) \| void` | Server |

`player` is the Player the engine reports as the sender. It never comes from the payload.

Each callback runs on its own thread, owned by the Subsystem, so it may yield. Yielding never delays the next packet's callback. A receiver works from the end of boot until its Subsystem faults. After that, its messages have no receiver.

A callback that throws is reported and does not fault the Subsystem. An event callback must return nothing. A request callback must not return a Promise. A request callback that throws or returns a value its response type rejects answers the ask with `handler-error`. A callback still running 30 seconds after it started is reported, and it keeps running.

## Run code when a client is ready

`ClientReady` runs once for each player, after that player's client booted and the server sent everything it queued for that player. Any number of Subsystems can receive it, one field each, and they run in construction order.

A `ClientReady` callback must not yield. It may return a cleanup function, which runs once when the player leaves, the player's connection fails, or the Subsystem faults.

```ts
import { ClientReady, receive, Subsystem } from "@trioforge/crucible";

export class Presence extends Subsystem {
	private readonly online = new Set<Player>();

	private readonly ready = receive(ClientReady, (player) => {
		this.online.add(player);
		return () => this.online.delete(player);
	});
}
```

To load data for a player, spawn a thread inside the callback and return a cleanup that cancels it.

## Send from the client

`Network.send(message, ...payload)` sends a `ToServer` message. `Network.ask(message, ...request)` sends an `AskServer` request and yields until the reply, the declared timeout, or the session ending.

```ts
import { askServer, toServer } from "@trioforge/crucible";
import { Network } from "@trioforge/crucible/client";

const DoorToggle = toServer<{ readonly door: Model; readonly open: boolean }>("Doors.Toggle");
const Profile = askServer<{ readonly userId: number }, string>("PlayerCard.Profile");

export function toggle(door: Model, open: boolean) {
	Network.send(DoorToggle, { door, open });
}

export function nameOf(userId: number): string | undefined {
	const result = Network.ask(Profile, { userId });
	if (result.ok) return result.value;
	warn(`profile request failed: ${result.reason}`);
	return undefined;
}
```

`Network.ask` returns an `AskResult<Response>`. A success is `{ ok: true, value }`, and `undefined` and `false` are successful values. A failure is `{ ok: false, reason }`, where `reason` is an `AskFailure`:

| Reason | Meaning |
| --- | --- |
| `timeout` | The declared timeout passed, counted from the call. |
| `disconnected` | The session failed or closed. |
| `unavailable` | The server has no live receiver, or the server's catalog lacks the message. |
| `handler-error` | The server callback threw, returned a Promise, or returned a value the response type rejects. |
| `overloaded` | 64 asks are already pending, or the server is running 64 callbacks for this player. |
| `invalid` | The request or the reply did not decode, for example because an Instance it names is gone. |

A timeout does not mean the server did nothing. Call `Network.ask` where yielding is allowed, such as `start` or a spawned thread.

## Send from the server

`Network.send(player, message, ...payload)` sends a `ToClient` message to one player. `Network.broadcast(message, ...payload)` sends it to every player in the game now. Players who join later do not receive it.

```ts
import { toClient } from "@trioforge/crucible";
import { Network } from "@trioforge/crucible/server";

const DoorbellRang = toClient<{ readonly button: BasePart | undefined; readonly houseId: string }>("Doorbell.Rang");

export function ring(button: BasePart, houseId: string) {
	Network.broadcast(DoorbellRang, { button, houseId });
}

export function ringFor(player: Player, houseId: string): boolean {
	return Network.send(player, DoorbellRang, { button: undefined, houseId });
}
```

## When a send works

Sends work from a Subsystem's constructor, `awake`, and `start`, and at any time after boot. A send during module load throws. After a failed boot, `Network.send` returns `false` and `Network.ask` returns `disconnected`.

Each call checks and encodes its payload before it returns, so changing the table afterwards changes nothing that is sent. A payload that does not fit its type throws at the call, naming the message, the property path, and the reason, such as `Doors.Toggle payload.door: expected declared Instance class`.

`true` from `Network.send` means Crucible accepted the message. It never means the message arrived. Until a client is ready, both sides hold its messages in order and send them when the client finishes booting. `false` means the message was not accepted:

- On the server, the player is not in the game, the player's connection failed, or this send overflowed the player's queue.
- On the client, the session failed or closed, or the server's catalog lacks the message.

Messages to one player arrive in the order they were sent, and their callbacks start in that order.

## What happens when something goes wrong

A packet from a client that does not decode is dropped and reported once per player per message. A packet that names an Instance the server cannot see is dropped without a report, and an ask with one answers `invalid`. A client's bad packet never kicks the player.

The connection fails for good, and the server kicks the player with "Connection problem, please rejoin.", in these cases:

- A queue held for a client that is not ready yet would exceed 256 messages or 1 MiB.
- A message arrives while the server is already running 64 callbacks for that player.
- The client receives a server message it cannot decode, or any other packet that breaks the protocol.

At shutdown the server keeps sending through every `onShutdown`, then closes every connection without a kick.

## Limits

| Limit | Value |
| --- | --- |
| Messages declared in one place | 1,024 |
| Queued messages per player before ready | 256 |
| Queued bytes per player before ready | 1,048,576, plus 64 for each Instance |
| Bytes per packet | 262,144 |
| Instances per packet | 256 |
| Callbacks running per player | 64 |
| Pending asks per client | 64 |
| Callback lifetime before a report | 30 seconds |
| Ask timeout | 10 seconds by default, at most 120 |
| Client wait for the server at boot | 30 seconds |
