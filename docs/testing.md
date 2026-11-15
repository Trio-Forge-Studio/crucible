# Testing

Build one Subsystem or behaviour in a Lune spec with fake constructor arguments and call its hooks, without `boot`.

The entry is `@trioforge/crucible/testing`. It is for specs that run the compiled game, such as Lune specs. Game code cannot import it. The compiler rejects the import with CRU004, the same rule that rejects `@trioforge/crucible/emit`, so the entry never reaches a game build by mistake.

## Load the entry from the class's module graph

Crucible keeps its construction claim, class registry and message catalog in module-level state. The entry only sees a class that was loaded with the same Crucible modules. In a spec, require the entry through the loader that loaded the class, not from a second copy of it.

```luau
local testing = import(crucible.testing)
local Controller = import(serverScripts.systems.controller).Controller
```

A class loaded in another module graph is not registered there, and `subsystem` throws `<class> is not a registered Crucible class; compile it with the Crucible compiler through crucible build`.

## Build a Subsystem

`subsystem(Class, ...args)` builds the class with the arguments given. A constructor that takes a dependency gets whatever table the spec passes.

```luau
local controller = testing.subsystem(Controller, { ready = true })
controller:awake()
controller:start()
controller:onShutdown()
```

Each call builds a new object. `receive()` fields register while the class builds, in the realm `RunService:IsClient()` reports. The entry asks `RunService` only when a `receive()` field registers, so a spec that replaces `RunService` with a double needs `IsClient` only for a class that receives. `subsystem` throws for a behaviour class, naming it.

## Build a behaviour

`behaviour(Class, instance, ...args)` builds a behaviour bound to `instance`. The instance can be a plain table that has a `GetAttribute` method.

```luau
local part = {
	GetAttribute = function(_, name)
		return attributes[name]
	end,
}
local member = testing.behaviour(Member, part)
member:onEnable()
member:onDestroy()
```

Each `attribute()` field reads the instance through `GetAttribute` and applies the same rule as binding. A missing optional attribute takes its fallback. A missing required attribute, or one of the wrong type, throws `<id>: attribute <name> is invalid (<type>)`. A behaviour with no `attribute()` fields never reads the instance. `behaviour` throws for a Subsystem class.

## Call hooks as methods

Hooks are plain method calls: `awake`, `start`, the frame hooks, `onEnable`, `onDisable`, `onDestroy` and `onShutdown`. The spec decides which run and in what order. Luau does not enforce `protected`.

## Constructor failures

A constructor that throws fails the build with its message unchanged. A constructor that yields fails with `<id>'s constructor yielded; boot faults a constructor that yields`. After either failure, `Class.new()` called directly still throws `constructed outside Crucible`, as it does after a successful build.

## Receivers

`receiver(message)` returns the callback a built Subsystem's `receive()` field registered for a message, or `nil` if none did. It throws if `message` is not a declared message.

```luau
testing.subsystem(Wallet)
local collect = testing.receiver(Collect)
collect(player, { size = 3 })
```

Receivers accumulate in the module graph. Building a receiving Subsystem a second time in the same graph throws `<name> has two receivers`. Load a fresh graph for each spec that builds one. A receiver in the wrong realm throws Crucible's realm error when it builds. A server message needs a server graph and a client message needs a client graph.

## Messages

`message(token)` returns `{ name, kind, input, output }` for a declared message, where `input` and `output` are schema nodes and `output` is set for an ask. It returns `nil` for anything else.

`transfer(token, value, which)` sends `value` through the message's compiled codec and returns what decoding gives back. `which` is `"input"` by default, or `"output"` for an ask's response. A value the codec refuses throws `<name> refused to encode <path>: <reason>`.

```luau
local got = testing.transfer(Collect, { size = 3 })
assert(got.size == 3)
```

`sealMessages()` seals the message catalog and returns the message names in wire order. It seals once per graph, and a second call throws. `n` is the schema builder set that `emit` exports.

## Registration

`registration(Class)` returns what the compiler registered for a class. A Subsystem gives `{ id, kind = "subsystem" }`. A behaviour gives `{ id, kind = "behaviour", tag, instanceClasses }`. It throws for an unregistered class.

## What it does not do

The entry builds an object and stops. It does not run `boot`, the lifecycle phases, `accepts`, tag binding or the scheduler. Built objects are not in the lifecycle's state, so `Behaviour.get`, `isEnabled` and `setEnabled` do not see them. Nothing calls a hook for the spec.

## Load a compiled place in Lune

The npm package ships the Lune entry at `node_modules/@trioforge/crucible/lune`. It runs outside Roblox and needs Lune on `PATH`. Build your place with `crucible build` first.

```luau
local harness = require("../node_modules/@trioforge/crucible/lune")
local graph = harness.loadGame({
	place = "build/game.rbxl",
	client = false,
	services = { Workspace = workspaceDouble },
})
local shared = graph.place:GetService("ReplicatedStorage").TS
local PlayerData = graph.import(shared.profile).PlayerData
local testing = graph.import(graph.crucible.testing)
```

`loadGame(options)` reads a compiled `.rbxl` or `.rbxlx` place. Each call owns its place, module cache and `_G`. Repeated imports return the same value in that graph. `services` maps service names to doubles, `globals` overrides module globals, and `task` replaces the task library. `engine`, `network` and `dataPort` replace the corresponding runtime ports. `client` defaults to false. `loadPlace(client?, path?, options?)` is the positional form and defaults to `build/sample-game.rbxl`.

The result exposes `place`, `crucible`, `import(instance)` and `load(instance)`. Use `import` for compiled modules so RuntimeLib registers them. `load` evaluates a script directly. `engine` is the installed engine fake, `messages` captures printed lines, `closers` holds shutdown callbacks, `attributes` records the default local player's attributes and `runService` is the default realm double. `loaded` records evaluated modules. Call `graph.place:Destroy()` after the spec.

The loader supplies RunService realm checks, an empty ReflectionService class list and a local-player attribute double. Other services come from the deserialized place unless overridden. It replaces the runtime engine port and installs logging and network fakes. It does not emulate Roblox physics, run boot automatically or prove gameplay behavior in Studio. A game's include folder must be mapped at `ReplicatedStorage.rbxts_include`, with Crucible under `node_modules.@trioforge.crucible`, either directly or with an `out` child.

## Build a profile fixture from a compiled store

```luau
local backend = harness.Backend.new()
local realm = backend:realm()
local profile = harness.profileFixture(realm, PlayerData, {
	onLoad = function(player, value, reason)
		return value
	end,
})
local player = realm.join(4120)
local pending = realm.run(function()
	return profile:load(player, "join")
end)
backend:advance(1)
assert(pending.done and pending.value == true)
assert(profile.published[player] ~= nil)
```

`profileFixture(realm, compiledStore, options?)` uses the compiled store's sealed defaults, persistence schema and migrations, including server-only fields. It returns a player-facing fixture with `published[player]` snapshots and a `reasons` list. Optional `onLoad(player, value, reason)` and `onRelease(player, value, reason)` callbacks transform values under the runtime's normal claim rules. `load(player, reason)`, `write(player, change)`, `save(player, timeout?)`, `release(player, reason?)`, `writable(player)`, `leave(player)`, `handoff(player)`, `resume(player)` and `await(player)` run through the production engine. `players()` lists writable players. `state(player)` returns `"absent"`, `"loading"`, `"active"`, `"sealed"` or `"ended"`. Release defaults to `"leave"`. Yielding methods run through `realm.run`, whose result has `done` and `value`.

`Backend.new()` creates committed storage and virtual time. `backend:realm({ id?, mono?, wall?, latency?, seed? })` creates an independent server realm. `realm.join(userId)`, `leave(player)`, `crash()` and `suspended` control presence and execution. `realm.clock()`, `wall()`, `spawn`, `wait`, `delay` and `cancel` form its fake clock and scheduler. `backend:advance(seconds)` runs due events and reports coroutine failures. `seed(store, key, value, userIds?, metadata?)`, `envelope(store, key)` and `stored(store, key)` seed or inspect copied committed values. `callsTo(filter)` lists calls matching fields such as `server`, `kind`, `store` and `key`.

`backend:fail(kind, filter?, times?, options?)` injects `throttle`, `transient`, `committed`, `conflict`, `late`, `stall` or `reject` failures. Options include `seconds`, `code` and `after`. `gate(point, filter?)` parks future calls at `transform`, `commit`, `committed` or `response`; call the returned `release()` to continue. A crashed realm never runs again, but an already-sent write can still commit.

## Use the engine and network doubles

`engine.new()` returns an engine with `port` and `loggingPort`. It supports `instance`, `addTag`, `removeTag`, `setAttribute`, `setInWorld`, `move`, `destroy`, `clone`, `flush`, `frame`, `cycle` and `logged`. Signals queue until `flush()`. `frame(engine.FrameEvent.Heartbeat, dt)` and `cycle(dt, renders)` drive frame callbacks. `logs` and `gameLogs` capture output.

`networkPort()` returns a port that records `fired`, `kicked`, `listeners` and `timers`. `failNext` names the next operation to throw, one of `"create"`, `"publish"`, `"discover"`, `"fireClient"` or `"fireServer"`. The failure is consumed once, before recording a send. It supplies an empty-game discovery catalog; games that exercise declared messages provide a network double implementing the catalog and transport they need.

## Replace copied BAFM helpers

BAFM's `load.luau`, `engine.luau`, `data-backend.luau` and `network-port.luau` map to this single entry's `loadGame`, `engine`, `Backend` and `networkPort`. Load `PlayerProfile` and `ProfileRules` through `graph.import` on their compiled place instances. Pass the exported profile store to `profileFixture`, and pass `ProfileRules.prepareProfile` through `onLoad`. The fixture no longer queries `internal/data/declarations`, constructs `DataEngine`, or loads `profiles`, `lease`, `envelope` and `admission` by private path. Tests of saved values use `backend:seed` and `backend:envelope`.

## Replace modules and inspect lifecycle

`graph.provideGame(instance, value)` replaces a compiled module before it or a consumer imports it. Imports and requires receive the replacement, including `false`. Replacing an already evaluated module throws. Replacements belong to one graph.

`graph.lifecycle(Class, instance?)` returns `"unconstructed"`, `"constructed"`, `"awake"`, `"enabled"`, `"disabled"`, `"faulted"` or `"destroyed"` for a registered class in this graph. Pass a bound instance for a behaviour. This replaces numeric phase assertions such as FD's private `Phase.Faulted` check. It observes lifecycle driven by boot and engine events; objects constructed manually through `/testing` are outside lifecycle.

## Load a Rojo sourcemap

A game can load its compiled output without building a place. Generate a Rojo sourcemap including scripts, then use:

```luau
local graph = harness.loadGame({
    sourcemap = "sourcemap.json",
    root = ".",
    output = "out",
    sourceRoot = "src",
})
```

`root` is the folder against which sourcemap file paths resolve. Lua and Luau paths load directly. TypeScript paths below `sourceRoot` load their matching `.luau` file below `output`, preserving folders and `init` modules. Paths already pointing to compiled output need no `output` option. All paths are relative to the current working directory unless absolute. The graph API and isolation match place loading.

Only the game APIs described above are supported. Disk loaders (`module`, `fresh`, `runtime`, `runtimeAt`), their module paths, and backend realm fields exposing runtime internals are unsupported white-box hooks for Crucible's own specs. Games must not depend on them.
