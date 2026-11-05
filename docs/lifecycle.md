# Lifecycle

Write Subsystems and behaviours, and learn the order Crucible runs their hooks, how it injects dependencies, and what a fault or a yield does.

Game code is classes. A class that extends `Subsystem` is a Subsystem. `boot` constructs one object of it on its side. A class that extends `CrucibleBehaviour` is a behaviour. Crucible constructs one object of it for each instance that has its tag. Both kinds come from the root entry, `@trioforge/crucible`, and a class registers when the module that declares it runs, through a call the compiler adds after the class. `boot` requires the modules its `load` globs match, as `crucible docs boot` describes, and constructs every class registered by then. That includes classes in modules those modules import. A class whose module is first required after boot throws `registered after boot`.

A registered class is named, declared at the top level of its module, has no type parameters and has a public constructor. Code never calls `new` on one. `new` outside Crucible throws `constructed outside Crucible`. `crucible docs diagnostics` lists the compile errors for each rule on this page.

## Write a Subsystem

```ts
import { Subsystem } from "@trioforge/crucible";

export class Round extends Subsystem {
	private elapsed = 0;

	public timeLeft() {
		return math.max(0, 120 - this.elapsed);
	}

	protected override awake() {
		print("round ready");
	}

	protected override start() {
		task.wait(3);
		print("round started");
	}

	protected override update(dt: number) {
		this.elapsed += dt;
	}
}
```

Hooks are `protected override` methods. Override only the hooks you need. A Subsystem exists from boot until the server or client stops, and nothing destroys it except a fault.

## Write a behaviour

A behaviour's first type argument is its CollectionService tag, one string literal. The second is the instance class it accepts, or a union of classes, and defaults to `Instance`. `this.instance` is the tagged instance.

```ts
import { attribute, CrucibleBehaviour } from "@trioforge/crucible";

export class Spinner extends CrucibleBehaviour<"Spinner", BasePart> {
	private readonly speed = attribute(90);
	private readonly label = attribute<string>();

	protected override onEnable() {
		print(`${this.label} spins at ${this.speed}`);
	}

	protected override update(dt: number) {
		this.instance.CFrame = this.instance.CFrame.mul(CFrame.Angles(0, math.rad(this.speed * dt), 0));
	}
}
```

Crucible binds a behaviour to an instance when the instance has the tag, is in the game, is of the declared class, has valid attributes, and `accepts` returns true. The default `accepts` refuses instances inside `ServerStorage`, `ReplicatedStorage`, `ServerScriptService`, `StarterGui`, `StarterPack` and `StarterPlayer`. Override the static `accepts(instance)` to add a condition. Call `Spinner.refresh(instance)` after a change that `accepts` reads, so binding checks the instance again.

An instance of the wrong class is never bound, and Crucible warns once.

### Attributes

`attribute()` reads an instance attribute into a field. The attribute's name is the field name with its first letter in upper case, so `speed` reads `Speed`. `attribute(90)` is optional and takes `90` when the attribute is missing as the behaviour is built. `attribute<string>()` is required. A field can be `string`, `number`, `boolean`, `Vector3`, `Color3`, a literal, or a union of them.

A required attribute that is missing, or any attribute of the wrong type, keeps the instance unbound and logs a warning. Fixing the attribute binds it. When a bound behaviour's attribute changes, the field takes the new value. A write of the wrong type keeps the last valid value and warns. Removing a required attribute counts as a write of the wrong type. Removing an optional attribute sets the field to `undefined`, not to its fallback, so read an optional field with `??` when the attribute can be removed at run time.

### Find, enable and remove a behaviour

- `Spinner.get(instance)` returns the bound, live behaviour, or `undefined`. A disabled behaviour is returned. A faulted one, or one not awake yet, is not.
- `setEnabled(false)` disables a behaviour and `setEnabled(true)` enables it again. `isEnabled()` reports its state.
- `remove()` destroys the behaviour. The instance stays unbound until its tag is removed or it leaves the game, as for a fault.

## The hooks

| Hook | Subsystem | Behaviour | Runs | May yield |
| --- | --- | --- | --- | --- |
| constructor | Yes | Yes | Once, with its dependencies | No |
| `awake` | Yes | Yes | Once, after the constructors of its wave | No |
| `onEnable` | No | Yes | Each time it is enabled | No |
| `start` | Yes | Yes | Once, the first time it is enabled | Yes |
| `fixedUpdate(dt)` | Yes | Yes | At a fixed rate while enabled, after `start` returns | No |
| `update(dt)` | Yes | Yes | Every frame while enabled, after `start` returns | No |
| `lateUpdate(dt)` | Yes | Yes | Every frame after every `update` | No |
| `preRender(dt)` | Client | Client | Every frame before rendering | No |
| `onDisable` | No | Yes | Each time it is disabled, and before `onDestroy` if enabled | No |
| `onDestroy` | No | Yes | Once, when the behaviour ends | No |
| `onShutdown` | Server | No | Once, when the server closes | Yes |

A Subsystem has no `onEnable`, `onDisable` or `onDestroy`, so Crucible never calls them on one. `preRender` and `renderPriority` need a client class, and `onShutdown` needs a server class. A method whose name is one typo from a hook name fails the build with CRU009, as `crucible docs diagnostics` describes. Hooks must be synchronous. An `async` hook fails with CRU012.

## Order

### Construction and the first batch

Crucible constructs new objects in batches. A batch runs these steps in order:

1. Every constructor of the wave.
2. Every `awake` of the wave.
3. Every `onEnable`, for behaviours.
4. Every `start`.
5. Frame hooks, from the next frame.

The first wave is the batch's objects. Objects that code creates during an `awake` form the next wave, which runs steps 1 and 2 again. Step 3 begins once a wave creates nothing more. So every object of a batch is awake before any is enabled, and enabled before any starts. The first batch on each side holds every Subsystem and every behaviour for the instances that are tagged at boot. Its Subsystems come first in each phase.

Subsystems are ordered dependencies first. A Subsystem's constructor dependencies are constructed before its own constructor runs, and awake before its own `awake` runs. Crucible walks Subsystems in registration order and visits each one's dependencies before it. Registration order is the order `boot` requires modules, sorted by full instance path, then the order of classes in each module. A module's imports run before it, so they register first. The walk can move a dependency ahead of Subsystems that do not depend on it, so do not rely on the order of Subsystems that do not depend on each other. The order between behaviours of different tags is not defined.

A behaviour tagged after boot gets its own batch, with the same phases.

### Start

Each `start` runs on its own thread. When it yields, the next object's `start` begins, and boot carries on. Boot can finish while a `start` is still suspended. An object gets frame hooks only after its own `start` returns, and only while it is enabled.

### Frames

| Hook | Runs on |
| --- | --- |
| `preRender` | `RunService.PreRender`, or `BindToRenderStep` at the class's `renderPriority` |
| `fixedUpdate` | `RunService.PreSimulation`, zero to four times, with `dt` equal to one fixed step |
| `update` | `RunService.PostSimulation` |
| `lateUpdate` | `RunService.PostSimulation`, after every object's `update` |

The fixed rate is 60 steps per second by default. Set it with `boot({ load, fixedRate })`. A frame longer than 0.25 seconds counts as 0.25 seconds, and a frame runs at most four fixed steps.

Within one hook, objects run in the order they began to tick. An object that is disabled and enabled again moves to the end.

### Shutdown

When the server closes, Crucible calls `onShutdown` on every Subsystem that has one. A Subsystem's `onShutdown` waits until the `onShutdown` of every Subsystem that depends on it has returned. Independent ones run at the same time and may yield. After 25 seconds, Crucible starts every `onShutdown` that is still waiting. A throw in `onShutdown` is logged and the others continue.

## Inject dependencies

### Constructor parameters

Take a dependency as a constructor parameter. Crucible passes the object when it constructs the class.

```ts
import { Subsystem } from "@trioforge/crucible";

export class Scores extends Subsystem {
	public readonly ready = true;
}

export class Leaderboard extends Subsystem {
	public constructor(private readonly scores: Scores) {
		super();
	}

	protected override awake() {
		assert(this.scores.ready);
	}
}
```

A parameter can name:

- A Subsystem class. Crucible passes that Subsystem.
- An interface or an abstract class that exactly one Subsystem implements or extends. Crucible passes that Subsystem. A class provides each base class and each interface it implements, including the interfaces those extend.
- `All<T>`, where `T` is a Subsystem or behaviour class, or an interface or abstract class that one provides. Crucible passes a live set of the objects that provide `T`. An Instance type such as `All<Part>` fails the build with CRU001.

A behaviour's constructor can take Subsystems and `All<T>` in the same way. A parameter cannot be optional, have a default value or be a rest parameter. A concrete behaviour class cannot be a parameter, because many instances of it can exist. Take `All<Spinner>`, or call `Spinner.get(instance)`.

### All

`All<T>` holds the enabled objects that provide `T`. An object joins after its `onEnable` returns and leaves before its `onDisable` runs, or when it faults.

```ts
import { CrucibleBehaviour, Subsystem, type All } from "@trioforge/crucible";

export class Checkpoint extends CrucibleBehaviour<"Checkpoint", BasePart> {}

export class Course extends Subsystem {
	public constructor(private readonly checkpoints: All<Checkpoint>) {
		super();
	}

	protected override start() {
		print(`${this.checkpoints.size()} checkpoints`);
		this.checkpoints.observe(
			(checkpoint) => print(`added ${checkpoint.instance.Name}`),
			(checkpoint) => print(`removed ${checkpoint.instance.Name}`),
		);
	}
}
```

`size()`, `has(item)`, `first()` and `forEach(visit)` read the set now. `observe(added, removed)` calls `added` for every current member and every later one, calls `removed` when one leaves, and returns a function that stops it. A subscription made from a hook ends with its object. Callbacks passed to `forEach` and `observe` must not yield or throw. A failure is logged and faults no object.

### inject

`inject<T>()` returns the one Subsystem that provides `T`, and `inject<All<T>>()` returns the set. It is for code outside registered classes, such as a helper module that a hook calls. Inside a Subsystem or a concrete behaviour, `inject` fails the build with CRU005, so take a constructor parameter there.

```ts
import { inject } from "@trioforge/crucible";
import { Store } from "../server/systems/store";

export function storeReady() {
	return inject<Store>().ready;
}
```

`inject<T>()` throws when it runs before boot, when no Subsystem or more than one class provides `T`, when the provider is faulted, and when the provider is not awake yet. Call it from `start` or later.

### What boot checks

`boot` checks the dependency graph before it constructs anything. It fails, naming every problem at once, when:

- Nothing provides a parameter's type.
- More than one class provides a parameter's type.
- Only behaviours provide it. Take `All<T>` or call `get` instead.
- Subsystems depend on each other in a cycle. The error reads `dependency cycle A -> B -> A; break it with All<T>`.
- A class is registered twice.
- A class with `preRender` or `renderPriority` is loaded on the server.

The compiler does not look for cycles, so a cycle compiles and fails at boot. To break one, make one side take `All<T>` of the other. `All<T>` adds no ordering edge. The set is filled once the other side is enabled, so read it from `start` or later, as with `this.others.first()`.

## Faults

A hook or a constructor that throws faults its object. So does a yield in any hook except `start` and `onShutdown`, as the next section describes. A throw in `onShutdown` is only logged. Crucible logs one error, `<Class> faulted in <hook>: <error>`, with the instance's path for a behaviour. Errors in the cleanup that follows are added to the same message.

A faulted object is finished for good. Crucible does this, in order:

1. It removes the object from its frame hooks and from every `All<T>` set, so `removed` observers run.
2. For a behaviour that was enabled, it runs `onDisable`.
3. It stops a suspended `start`. For a behaviour that was awake, it runs `onDestroy`.
4. It ends the object's `observe` subscriptions and `receive()` handlers.

A constructor that throws leaves no object, so no hook runs for it.

Nothing restarts a faulted object, and no API does.

- **A faulted Subsystem** stays faulted until the server or client stops. Its object still exists, and its public methods still run when other objects that hold it call them. It no longer ticks, and its `receive()` handlers stop: events for it are dropped with a warning and asks get `unavailable`. `inject` of it throws. `onShutdown` still runs for it.
- **A faulted behaviour** leaves its instance unbound. `get` returns `undefined`. The instance gets a new behaviour object only when its tag is removed and added again, or when it leaves the game and comes back. `refresh` and moving the instance inside the game do not bind it again.

A Subsystem that faults while `boot` runs fails the boot. Crucible destroys every object it constructed and `boot` throws `Crucible boot failed: <fault>`. A fault in a behaviour, or in a Subsystem after boot finished, is logged and does not stop the game. A Subsystem whose `start` yields lets boot finish, so a throw after that yield is a fault after boot.

Guard the work that can fail, so one bad call does not end the object. Wrap it in `pcall` inside the hook, or validate input from players before you use it.

## Yields

Only `start` and `onShutdown` may yield. A constructor, `awake`, `onEnable`, `onDisable`, `onDestroy` and a frame hook must return without yielding. `task.wait`, `WaitForChild`, `Network.ask`, a DataStore call and anything else that suspends the thread is a yield. A yield there faults the object with `yielded; only start may yield`, and the rest of the hook never runs.

Callbacks passed to `forEach` and `observe` must not yield either. A yield there stops that callback and is logged, but it faults no object.

`accepts` must not yield or throw. For an instance that is not bound yet, a failure logs an error and keeps the instance unbound until its tag is removed or it leaves the game. For a bound behaviour, a failure faults the behaviour.

This includes a yield in a function the hook calls, at any depth. `pcall` does not catch it: Luau lets a yield pass through `pcall`, so wrapping the call changes nothing.

To wait from another hook, start a thread. A thread made with `task.spawn` or `task.defer` inside a hook belongs to no object and may yield.

```ts
import { CrucibleBehaviour } from "@trioforge/crucible";

export class Door extends CrucibleBehaviour<"Door", Model> {
	protected override onEnable() {
		task.spawn(() => {
			const hinge = this.instance.WaitForChild("Hinge");
			print(`${hinge.GetFullName()} ready`);
		});
	}
}
```

A thread made that way is not stopped when the object is disabled or faults. Check `isEnabled()` after it resumes. `receive()` callbacks run on their own threads and may yield, as `crucible docs networking` describes.

The compiler checks frame connections and session ownership, including inherited bases. See CRU035 and CRU036 in `crucible docs diagnostics`.
