# Reactive state

Keep game state in reactive values and run effects when it changes, with lifetimes tied to your objects.

Import the reactive API from the root of `@trioforge/crucible`. The root is safe to import on the server and the client. Reactive state is per Luau VM: server and client values are separate.

```ts
import { computed, effect, onCleanup, scope, state, untracked, watch } from "@trioforge/crucible";
import type { Readable, State } from "@trioforge/crucible";
```

## Values

`state(initial)` makes a writable value. Read it by calling it. Write it with `set` or `update`.

```ts
import { state } from "@trioforge/crucible";

const coins = state(0);
coins.set(10);
coins.update((current) => current + 5);
print(coins()); // 15
```

A state needs no owner. Use it as a field initializer, a local, or a module constant.

- `set(value)` stores its argument as is. A function is stored, never called.
- `update(change)` calls `change` once with the current value and stores what it returns. Reads inside `change` are not tracked, and a write inside it throws.
- A write whose value is `rawequal` to the stored one does nothing. Mutating a table and storing the same table changes nothing, so store a new table.

A write is visible at once. A read right after it, in the same call, sees the new value, through any chain of computed values.

Any function of no arguments is a `Readable<T>`. A getter that reads state, such as `() => this.open() && !this.locked()`, tracks exactly like a state or a computed.

At runtime a `State` is a callable table, not a function. Every other readable is a plain function. Code that tests `type(x) == "function"`, such as a UI library's `read`, needs `() => s()` instead of the state itself.

## Computed values

`computed(read)` makes a cached value derived from other readables.

```ts
import { computed, state } from "@trioforge/crucible";

const price = state(4);
const quantity = state(3);
const total = computed(() => price() * quantity());
print(total()); // 12
price.set(5);
print(total()); // 15
```

- It is lazy. It evaluates on its first read, and again only when it is read after one of its dependencies changed.
- A result `rawequal` to the cached one keeps the cache, so nothing that depends on it runs again.
- A `read` that throws or yields caches the failure. Every read rethrows it, with the original traceback, until a dependency changes. The dependencies it read before failing count, and so do those of its last successful evaluation.
- A computed that reads itself, directly or through others, fails with `computed at <origin> reads itself`.
- `read` must be pure. Writing state, or calling `effect`, `watch`, `scope` or `onCleanup` inside it throws. It cannot yield.
- It needs no owner. A computed that nothing observes holds no subscription on its sources, so it is collected with whatever holds it.

## Effects

`effect(body)` keeps something outside the graph in step with state. The body may return a cleanup.

```ts
import { effect, state } from "@trioforge/crucible";

export function track(label: TextLabel) {
	const score = state(0);
	effect(() => {
		label.Text = `Score: ${score()}`;
	});
	effect(() => {
		const connection = label.GetPropertyChangedSignal("Visible").Connect(() => score.set(0));
		return () => connection.Disconnect();
	});
	return score;
}
```

- The first run, like every later one, happens on the reactive flush, never inside the call that created the effect.
- It runs again after anything its last run read changed. Several writes before the flush reach it as one run with the final values.
- Dependencies are rebuilt each run, so a branch that stops reading a value stops depending on it.
- Before the next run, and when the effect ends, the previous run ends: effects and scopes it created end first, then its cleanups run, newest first. The cleanup the body returned runs first among them.
- A run that throws, yields, or returns anything other than nothing or a function stops the effect for good. What the run created is released, the failure is reported once through Crucible's diagnostics, and other effects keep running. Nothing is rethrown into the code that wrote the state.

`effect` needs an owner, described below. Without one it throws `effect() needs an owner: create it in a Subsystem or behaviour, or inside scope()`.

## Watching one value

`watch(source, onChange)` calls `onChange(value, previous)` when `source()` changes.

```ts
import { scope, state, watch } from "@trioforge/crucible";

const phase = state<"waiting" | "running">("waiting");
const stop = scope(() => {
	watch(phase, (value, previous) => print(`${previous} -> ${value}`));
});
phase.set("running"); // prints "waiting -> running" on the next flush
stop();
```

- `source` is read once, at the `watch` call, as the baseline. A change between the call and the first flush is seen.
- It observes the value at each flush, not each write. A value that changes and changes back before the flush makes no call. Three writes before a flush make one call with the first and last values.
- `onChange` runs untracked. Effects and cleanups it creates end at the next change.
- If `source` throws at the call, `watch` rethrows and creates nothing.
- It is owned, timed and stopped exactly like `effect`.

## Untracked reads

`untracked(read)` calls `read` and returns its result without recording what it read. An effect created inside it still belongs to the current owner.

```ts
import { effect, state, untracked } from "@trioforge/crucible";

const target = state(10);
const step = state(1);

export function follow() {
	effect(() => {
		// Reruns when target changes, not when step does.
		print(target() + untracked(() => step()));
	});
}
```

## Ownership

Every effect, watch, scope and cleanup has an owner, found when it is created:

1. Inside an effect run, a `watch` callback, a scope's setup or a cleanup, the owner is that run or scope.
2. Otherwise, inside code a Crucible object runs, the owner is that object:
   - while the object's `onEnable` runs, including callbacks it triggers synchronously as that object, the owner is the current enable interval. It ends right after `onDisable` returns, and at destroy or fault;
   - anywhere else, such as the constructor, a field initializer, `awake`, `start`, `onDisable`, a frame hook or a later callback, the owner is the object itself. It ends after `onDestroy`.
3. Otherwise there is no owner. `effect`, `watch` and `onCleanup` throw, and `scope` makes a root.

So a behaviour builds its reactions in `onEnable` and needs no `onDisable` code to undo them. A pooled behaviour rebuilds them on every enable.

```ts
import { CrucibleBehaviour, effect, onCleanup, state } from "@trioforge/crucible";

export class Lamp extends CrucibleBehaviour<"Lamp", BasePart> {
	readonly lit = state(false);

	protected override onEnable() {
		const light = new Instance("PointLight");
		light.Parent = this.instance;
		onCleanup(() => light.Destroy());
		effect(() => {
			light.Enabled = this.lit();
		});
	} // at disable: the effect ends, then the light is destroyed
}
```

A Subsystem has no `onEnable`, so its reactions live for the session. A disabled behaviour's object-owned effects keep running.

`All<T>.observe` keeps its own lifetime: a subscription made in `onEnable` lasts until destroy. Its deliveries for members that exist already run during `onEnable` and join the interval. To end the subscription at disable, register its disconnect: `onCleanup(members.observe(added, removed))`.

### Scopes

`scope(setup)` runs `setup` now as a new lifetime and returns an idempotent dispose. Everything created inside `setup` belongs to it.

- Inside another owner it is a child, and it ends when its owner ends or when it is disposed, whichever comes first.
- Outside every owner it is a root that only its dispose ends. Roots work at module scope, before boot.
- `setup` must not yield. If it throws or yields, what it created ends and the error is rethrown at the `scope` call.

### Cleanups

`onCleanup(release)` registers `release` with the current owner. Register it right after you acquire the resource, so a later throw in the same hook or run leaks nothing.

Ending an owner ends its child effects and scopes first, newest first, then runs its cleanups, newest first. Each cleanup runs at once, protected: a throw or yield is reported and the next cleanup still runs. A lifecycle request a cleanup makes, such as `setEnabled`, applies after the current lifecycle step.

Under an owner that has already ended, `effect` and `watch` create nothing, `scope` returns a dispose without running `setup`, and `onCleanup` runs `release` at once.

A thread made with `task.spawn`, `task.defer` or `coroutine.wrap` has no owner and tracks nothing, even inside an effect run. Server `onShutdown` hooks run that way, so `effect` throws there while writes still work.

## When effects run

Effects run on one deferred flush per VM, requested through `task.defer` by the first change after the previous flush.

- No effect runs inside a setter, a constructor or field initializer, a lifecycle hook, or a network callback.
- Each run is complete, cleanup registered or failure released, before the lifecycle requests it made apply, and those apply before the next effect runs.
- A flush runs in rounds, in creation order. A write made by one effect reaches the effects that read it in the next round, so later effects see each completed write.
- An effect whose computed dependencies all recomputed to equal values is skipped.
- A frame hook's write is visible at once to later hooks, but its effects run on the flush. Work that must happen before the next member's hook belongs in the hook itself.

Effects created while modules load or while `boot` runs wait until boot has finished. No effect body runs before boot commits. If boot fails, the effects of the objects it rolled back never run.

The flush bounds its own work:

| Bound | Value | When reached |
| --- | --- | --- |
| Runs of one effect in one flush | 64 | The effect stops and is reported: it is a feedback loop |
| Effect runs in one flush | 65,536 | The rest waits for the next flush, unstopped |
| Rounds in one flush | 256 | The rest waits for the next flush, unstopped |
| Live effects and scopes one object owns directly | 4,096 | One warning names the object |

## What the types reject

A read is a call, so an uncalled readable in a condition is a type error. A `State` does not widen to a wider value type, and an effect returns nothing or a cleanup.

```ts
import { computed, effect, onCleanup, state, type Readable, type State } from "@trioforge/crucible";

const open = state(false);
// @ts-expect-error an uncalled read in a condition is always true
if (open) print("always");
// @ts-expect-error the same on the left of &&
export const shown = open && true;
// @ts-expect-error a read takes no argument; write with set
open(true);
// @ts-expect-error the value type is fixed
open.set("yes");
// @ts-expect-error an updater returns the state's type
open.update(() => "no");
// @ts-expect-error a state does not widen
export const wider: State<boolean | undefined> = open;
export const reader: Readable<boolean | undefined> = open;

const doubled = computed(() => 2);
// @ts-expect-error a computed has no setter
doubled.set(3);
// @ts-expect-error a boolean is not a cleanup
effect(() => true);
// @ts-expect-error onCleanup takes a function
onCleanup(5);
```
