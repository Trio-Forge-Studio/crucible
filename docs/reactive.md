# Reactive state

Keep game state in Charm atoms and run effects when it changes, with lifetimes tied to your objects.

Values come from `@rbxts/charm` 0.11: `atom`, `computed`, `untracked` and `batch`. Crucible keeps the lifetimes: `effect`, `watch`, `scope`, `onCleanup` and `own`, imported from the root of `@trioforge/crucible`. The root is safe to import on the server and the client. Reactive state is per Luau VM: server and client values are separate.

```ts
import { atom, batch, computed, untracked } from "@rbxts/charm";
import { effect, onCleanup, own, scope, watch } from "@trioforge/crucible";
import type { Ownable } from "@trioforge/crucible";
```

## Install Charm

`@rbxts/charm` is a required peer dependency of `@trioforge/crucible`, at exactly `0.11.1`. Crucible does not re-export it, so the game lists it in its own `dependencies`, as a game made with `crucible create` does.

The peer is exact because Crucible reads and edits Charm's internal nodes, which no Charm API covers. When the runtime loads, it checks that Charm behaves as it assumes, and it fails with `Crucible needs @rbxts/charm 0.11.1, and the installed Charm <what differs>` when it does not. A Crucible release that supports a newer Charm moves the pin.

The check writes an atom and needs its effects to run before the write returns. Inside a batch or an effect, Charm holds them until the batch or the flush ends, so the check cannot run there. Require `@trioforge/crucible` outside both, as a script's top level does. A first load inside one fails with `Crucible checks @rbxts/charm as it first loads, and Charm ran no effect for the check's write`, and the next load outside checks again.

A place must hold one copy of Charm. Charm keeps its graph in module variables, so an atom from a second copy is invisible to Crucible's effects. The Rojo project `crucible create` writes maps the game's `node_modules/@rbxts` folder once, and the runtime finds Charm there.

Leave `Charm.flags.trackInnerEffects` on, as it is by default. Crucible relies on it to end the effects a run created when the run's effect reruns.

## Values

`atom(initial)` makes a writable value. Read it by calling it with no argument. Write it by calling it with the new value, or with a function that takes the current value and returns the new one.

```ts
import { atom } from "@rbxts/charm";

const coins = atom(0);
coins(10);
coins((current) => current + 5);
print(coins()); // 15
```

- An atom needs no owner. Use it as a field initializer, a local, or a module constant.
- A write equal to the stored value does nothing. Mutating a table and storing the same table changes nothing, so store a new table. In Studio, Charm freezes the tables it stores.
- An atom called with a function calls it. To store a function, write `handler(() => callback)`.

`computed(read)` makes a cached value derived from others, `untracked(read)` reads without tracking, and `batch(run)` holds the effects that the writes inside `run` reach until it returns. Charm's documentation describes the rest of its API.

An atom, a computed and a replicated declaration are plain functions of no arguments. A getter that reads them, such as `() => this.open() && !this.locked()`, tracks the same way.

A profile handle reads the same way, with `handle()`, but it is a callable table, not a function. Charm's `computed`, `listen` and `subscribe`, and `useAtom` from `vide-charm`, fail at run time when they are given one, so pass them `() => handle()`. The types do not catch it, because a handle's call signature matches `() => T`. Crucible's `effect` and `watch` call their source themselves, so `watch(handle, onChange)` works.

```ts
import { computed } from "@rbxts/charm";
import { profileStore } from "@trioforge/crucible";

const PlayerData = profileStore("Game.Computed", { template: { coins: 0 } });

export function isRich(player: Player) {
	const coins = PlayerData(player).coins;
	return computed(() => coins() >= 100);
}
```

### Keep computeds pure

Crucible no longer checks what a `computed` does. Its `read` can write an atom, a published value or profile data, or hand a profile off, and nothing throws. The write happens whenever a read finds the computed stale, which depends on who reads it and when. Keep `read` to deriving a value, and write from an effect, a hook or a handler instead.

Do not create an `effect`, `watch`, `scope`, `onCleanup` or `own` inside a computed either. It attaches to whichever code's read ran the computed, and that changes from one run to the next.

## Effects

`effect(body)` keeps something outside Charm's graph in step with the values it reads. The body may return a cleanup. `effect` returns a dispose that ends the effect early. The dispose is idempotent.

```ts
import { atom } from "@rbxts/charm";
import { effect } from "@trioforge/crucible";

export function track(label: TextLabel) {
	const score = atom(0);
	effect(() => {
		label.Text = `Score: ${score()}`;
	});
	effect(() => {
		const connection = label.GetPropertyChangedSignal("Visible").Connect(() => score(0));
		return () => connection.Disconnect();
	});
	return score;
}
```

- The body runs once inside the `effect` call, unless boot is open, as described in "Boot".
- It runs again inside every write that changes what its last run read. Inside a batch, it runs once when the outermost batch ends, with the final values.
- Dependencies are rebuilt each run, so a branch that stops reading a value stops depending on it.
- Before each rerun, the scopes the last run created end, then its cleanups run, newest first, starting with the cleanup the body returned. The effects the last run created end when the new run finishes. When the effect itself ends, all of them end.
- A body that throws, yields, or returns anything other than nothing or a function fails its run. A failed first run makes `effect` throw, and nothing is created. A failed later run throws to the code that wrote, and the effect keeps its place: the next write that changes what it read runs it again.
- The effects that a first run's writes reach run after it, as the `effect` call returns. When one of them throws, `effect` throws that error and nothing is created. When the first run itself failed, `effect` throws the first run's error, and the other is reported as `Crucible: an effect failed after an effect's first run failed: <error>`.

`effect` needs an owner, described below. Without one it throws `effect() needs an owner: create it in a Subsystem or behaviour, or inside scope()`.

## Watching one value

`watch(source, onChange)` calls `onChange(value, previous)` when `source()` changes. It returns a dispose, like `effect`.

```ts
import { atom } from "@rbxts/charm";
import { scope, watch } from "@trioforge/crucible";

const phase = atom<"waiting" | "running">("waiting");
const stop = scope(() => {
	watch(phase, (value, previous) => print(`${previous} -> ${value}`));
});
phase("running"); // prints "waiting -> running"
stop();
```

- `source` is read once at the call, untracked, as the baseline. If it throws there, `watch` rethrows and creates nothing.
- A watch is an effect that reads `source`. It calls `onChange` on the runs where the value is not `rawequal` to the last value seen. Inside a batch it sees only the final value, so a value that changes and changes back makes no call.
- `source` and `onChange` run as the watch's owner, whichever code wrote the value. `onChange` runs untracked.
- The effects, scopes and cleanups `onChange` creates end when the watch next reruns, which is when something `source` read changes.
- It is owned, timed and disposed exactly like `effect`.

## Ownership

Every effect, watch, scope and cleanup has an owner, found when it is created:

1. Inside an effect run, a watch's `source` or `onChange`, a scope's setup or a cleanup, the owner is that run or scope.
2. Otherwise, inside code a Crucible object runs, the owner is that object:
   - while the object's `onEnable` runs, including callbacks it triggers synchronously as that object, the owner is the current enable interval. It ends right after `onDisable` returns, and at destroy or fault;
   - anywhere else, such as the constructor, a field initializer, `awake`, `start`, `onDisable`, a frame hook or a later callback, the owner is the object itself. It ends after `onDestroy`.
3. Otherwise there is no owner. `effect`, `watch`, `onCleanup` and `own` throw, and `scope` makes a root.

So a behaviour builds its reactions in `onEnable` and needs no `onDisable` code to undo them. A pooled behaviour rebuilds them on every enable.

```ts
import { atom } from "@rbxts/charm";
import { CrucibleBehaviour, effect, onCleanup } from "@trioforge/crucible";

export class Lamp extends CrucibleBehaviour<"Lamp", BasePart> {
	readonly lit = atom(false);

	protected override onEnable() {
		const light = new Instance("PointLight");
		light.Parent = this.instance;
		onCleanup(() => light.Destroy());
		effect(() => {
			light.Enabled = this.lit();
		});
	} // At disable the effect ends, then the light is destroyed.
}
```

A Subsystem has no `onEnable`, so its reactions live for the session. A disabled behaviour's object-owned effects keep running.

`All<T>.observe` keeps its own lifetime. A subscription made in `onEnable` lasts until destroy. Its deliveries for members that exist already run during `onEnable` and join the interval. To end the subscription at disable, register its disconnect with `onCleanup(members.observe(added, removed))`.

### Threads

The owner follows the code Crucible runs, not one thread:

- A `start` hook keeps its object as owner across yields, for its whole life.
- A thread that a hook or callback starts with `task.spawn` or `coroutine.wrap` has the hook's owner until it first yields. After it resumes, it runs as the code that resumed it. The engine resuming it, such as after `task.wait` or an event, gives it no owner. A Crucible callback that resumes it synchronously, such as another object's hook calling `coroutine.resume`, gives it that callback's owner, and what it creates then belongs to that callback. A thread made with `task.defer` never has an owner of its own.
- Server `onShutdown` hooks run on threads with no owner, so `effect` throws there while writes still work.

### Charm's own callbacks

Only Crucible's entry points carry an owner into the callbacks they hand to Charm. A callback that game code registers with Charm's `effect`, `subscribe` or `listen` gets no Crucible ownership. When a later write runs it, `effect`, `watch`, `scope`, `onCleanup` and `own` inside it attach to whatever code wrote. Use Crucible's `effect` and `watch` for anything that creates lifetimes.

### Scopes

`scope(setup)` runs `setup` now as a new lifetime and returns an idempotent dispose. Everything created inside `setup` belongs to it.

- Inside another owner it is a child, and it ends when its owner ends or when it is disposed, whichever comes first.
- Outside every owner it is a root that only its dispose ends. Roots work at module scope, before boot.
- `setup` must not yield. If it throws or yields, what it created ends and the error is rethrown at the `scope` call.

### Cleanups

`onCleanup(release)` registers `release` with the current owner. Register it right after you acquire the resource, so a later throw in the same hook or run leaks nothing.

Ending an owner ends its child effects and scopes first, then runs its cleanups, newest first. Each cleanup runs at once, protected, as the owner that registered it. A throw or yield is reported, and the next cleanup still runs. A lifecycle request a cleanup makes, such as `setEnabled`, applies after the current lifecycle step.

`own(value)` registers `value` itself as a cleanup and returns it. The release depends on the value's type, and `own` picks it when you call it:

| Value | Released with |
| --- | --- |
| A function | A call with no arguments |
| An `RBXScriptConnection` | `Disconnect()` |
| An `Instance` | `Destroy()` |
| A `thread` | `task.cancel` |
| An object with `destroy()` or `Destroy()` | That method, or `destroy()` when it has both |

```ts
import { CrucibleBehaviour, own } from "@trioforge/crucible";

const RunService = game.GetService("RunService");

export class Thruster extends CrucibleBehaviour<"Thruster", BasePart> {
	protected override onEnable() {
		const center = own(new Instance("Attachment"));
		center.Parent = this.instance;
		own(RunService.PreSimulation.Connect((dt) => this.push(center, dt)));
		own(task.delay(5, () => this.stall()));
		own(() => RunService.UnbindFromRenderStep("ThrusterCamera"));
	} // At disable the camera unbinds, the delay is cancelled, the connection disconnects and the attachment is destroyed.

	private push(center: Attachment, dt: number) {
		center.WorldCFrame = this.instance.CFrame.mul(new CFrame(0, 0, -dt));
	}

	private stall() {
		this.instance.Anchored = true;
	}
}
```

An owned value is a cleanup. It shares the owner, the newest-first order and the failure reporting of `onCleanup`. A failed release is reported with the source line of its function or method, or with the Instance's full name. Each call registers one release, so a value owned twice is released twice. The type `Ownable` lists what `own` takes. Anything else is a type error, and at runtime `own` throws `own() takes a function, an RBXScriptConnection, an Instance, a thread, or an object with destroy() or Destroy()`.

Under an owner that has already ended, `effect` and `watch` create nothing and return a dispose that does nothing, `scope` returns one without running `setup`, `onCleanup` runs `release` at once, and `own` releases `value` at once.

## When effects run

An effect runs inside the write that reaches it, or when the outermost batch ends.

Crucible opens a batch around every entry it runs that cannot yield: the constructor and field initializers, `awake`, `onEnable`, `onDisable`, `onDestroy`, frame hooks, `All<T>.observe` deliveries, `ClientReady` callbacks and their cleanups, and profile store callbacks. The effects an entry's writes reach run once, after the entry returns. An effect that fails then is reported as `Crucible: an effect failed after <object> <hook>: <error>`, or `after a data callback`, with its traceback. The entry's own code has already finished.

Crucible's own writes are batched the same way, so their readers see every value at once: a sync frame, a profile change, a config refresh. An effect that fails after one is reported as `Crucible: an effect failed after <what>: <error>`.

A `start` hook, a `receive` handler and a thread can yield, so Crucible never batches them: a batch left open across a yield would hold every effect in the place. A write there runs its effects at once. When one of them fails, the write throws in that code. Wrap writes in `batch` there yourself when the code between them cannot yield.

Charm 0.11.1 runs a write's effects in order and stops at the first that throws. The effects after it miss that write, and see the next one. Replication is not affected: a per-player projection that such a write reaches is still sent, once a quiet Heartbeat's scan finds it. Each quiet Heartbeat reads whole peers' projections and counts one visit per player and one per projection. It checks 1,024 visits as a threshold before each player, not as a ceiling, so it stops after the player that reaches it: a scan reads at most 1,024 + 255 projections. With 64 players and 256 per-player declarations it reads 4 players a Heartbeat and covers every player in 16 Heartbeats, about a quarter of a second. The send that follows obeys the same per-Heartbeat limits as any other change.

Nothing limits how often an effect reruns. Crucible's old engine stopped an effect after 64 runs in one flush and reported a feedback loop. Charm has no such check. An effect that writes a value it also reads reruns after its own write, and when every run writes a new value, it never stops: the place hangs until the script times out. Two effects that write each other's values loop the same way. Derive such a value with `computed`, or write it with an updater, which reads the current value without tracking it.

```ts
import { atom } from "@rbxts/charm";
import { effect } from "@trioforge/crucible";

const score = atom(0);
const history = atom<readonly number[]>([]);

export function recordScores() {
	// Never ends: each run reads history and writes it a new table.
	// effect(() => history([...history(), score()]));

	// Tracks only score: the updater reads history untracked.
	return effect(() => {
		const value = score();
		history((previous) => [...previous, value]);
	});
}
```

Each effect run is its own lifecycle step: the lifecycle requests it makes, such as `setEnabled`, apply after it returns.

## Boot

While boot is open, `effect` and `watch` create nothing yet. They queue their bodies in order and return a dispose. When boot commits, after it logs that it booted, Crucible creates the queued effects in order, inside one batch. If boot rolls back, the queue is discarded and no queued body runs.

A queued effect belongs to its owner from the start:

- The dispose it returned cancels it, and later ends the effect.
- Its owner's end cancels it: destruction, a disposed scope, or the end of an enable interval.
- At commit a cancelled entry is skipped, and so is one whose owner an earlier entry's first run destroyed.
- A body that disposes its own effect during its first run at commit runs once, and the effect ends there.
- A body that throws at commit is reported as `Crucible: an effect of <object> failed at boot's commit: <error>`, with its traceback. That effect ends, and the remaining entries are still created. A commit is never rolled back.

## Strict mode

In Studio, Charm's strict mode runs each effect body and each `untracked` and `batch` callback on a new coroutine, and fails a yield there. Crucible's ownership reaches through it: an effect created inside `untracked` in an effect run still belongs to that run, and a cleanup still runs as the owner that registered it.

## Moving from Crucible's values

Crucible no longer exports `state`, `computed`, `untracked`, `Readable` or `State`. Charm replaces them:

| Before | Now |
| --- | --- |
| `state(0)` from `@trioforge/crucible` | `atom(0)` from `@rbxts/charm` |
| `coins.set(5)` | `coins(5)` |
| `coins.update((current) => current + 1)` | `coins((current) => current + 1)` |
| `computed(read)` from `@trioforge/crucible` | `computed(read)` from `@rbxts/charm` |
| `untracked(read)` from `@trioforge/crucible` | `untracked(read)` from `@rbxts/charm` |
| `State<T>` | `Atom<T>` from `@rbxts/charm` |
| `Readable<T>` | `() => T` |
| `publish(declaration).set(value)` | `publish(declaration)(value)`, as `crucible docs replicated` describes |

A stored function needs the wrapper `handler(() => callback)`, because an atom calls a function it is given. `effect` and `watch` now return a dispose, and an effect runs inside the write instead of on a deferred flush.

## What the types reject

A read is a call, so an uncalled value in a condition is a type error. An effect returns nothing or a cleanup, and `own` takes only an `Ownable`.

```ts
import { atom, computed } from "@rbxts/charm";
import { effect, onCleanup, own } from "@trioforge/crucible";

const open = atom(false);
// @ts-expect-error an uncalled read in a condition is always true
if (open) print("always");
// @ts-expect-error the value type is fixed
open("yes");
// @ts-expect-error an updater returns the atom's type
open(() => "no");

const doubled = computed(() => 2);
// @ts-expect-error a computed takes no argument
doubled(3);
// @ts-expect-error a boolean is not a cleanup
effect(() => true);
// @ts-expect-error onCleanup takes a function
onCleanup(5);

// own returns its argument with its own type.
export const attachment: Attachment = own(new Instance("Attachment"));
// @ts-expect-error a number is not ownable
own(5);
// @ts-expect-error an object needs destroy() or Destroy()
own({ close() {} });
// @ts-expect-error own calls a function with no arguments
own((dt: number) => print(dt));
```
