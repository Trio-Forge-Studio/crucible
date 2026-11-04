# Testing

Build one Subsystem or behaviour in a Lune spec with fake constructor arguments and call its hooks, without `boot`.

The entry is `@trioforge/crucible/testing`. It is for specs that run the compiled game, such as Lune specs. Game code cannot import it. The transformer rejects the import with CRU004, the same rule that rejects `@trioforge/crucible/emit`, so the entry never reaches a game build by mistake.

## Load the entry from the class's module graph

Crucible keeps its construction claim, class registry and message catalog in module-level state. The entry only sees a class that was loaded with the same Crucible modules. In a spec, require the entry through the loader that loaded the class, not from a second copy of it.

```luau
local testing = import(crucible.testing)
local Controller = import(serverScripts.systems.controller).Controller
```

A class loaded in another module graph is not registered there, and `subsystem` throws `<class> is not a registered Crucible class; compile it with the Crucible transformer`.

## Build a Subsystem

`subsystem(Class, ...args)` builds the class with the arguments given. A constructor that takes a dependency gets whatever table the spec passes.

```luau
local controller = testing.subsystem(Controller, { ready = true })
controller:awake()
controller:start()
controller:onShutdown()
```

Each call builds a new object. `receive()` fields register while the class builds, in the realm `RunService:IsClient()` reports. `subsystem` throws for a behaviour class, naming it.

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

`registration(Class)` returns what the transformer registered for a class. A Subsystem gives `{ id, kind = "subsystem" }`. A behaviour gives `{ id, kind = "behaviour", tag, instanceClasses }`. It throws for an unregistered class.

## What it does not do

The entry builds an object and stops. It does not run `boot`, the lifecycle phases, `accepts`, tag binding or the scheduler. Built objects are not in the lifecycle's state, so `Behaviour.get`, `isEnabled` and `setEnabled` do not see them. Nothing calls a hook for the spec.
