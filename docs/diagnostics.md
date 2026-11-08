# Diagnostics

Find what each Crucible compile error from CRU001 to CRU036 means and how to fix it.

Crucible's compiler checks game code while `crucible build` compiles it, so a game made from the template reports these errors in `npm run build` and `npm run check`. The editor's TypeScript service does not run these rules, so it never reports them. Code that type-checks in the editor can still fail the build.

Each error is a TypeScript error whose code is 90000 plus the rule's number. CRU009 prints as `error TS90009: CRU009: Method resembles a lifecycle hook. Use its exact Crucible declaration name.` Every error fails the build.

An error that starts with `Crucible:` instead of a code is TS90000. It means the compiler could not run. It cannot read a `package.json`, cannot compute a type id for a declaring package, or failed on the input file. Fix the package metadata it names. A type id needs the declaring package's `name` and the same folder layout for its sources and its declarations. An error that names two ABI versions means the installed `@trioforge/crucible` was built for another compiler. Install the `@trioforge/crucible` release with the same version as your `crucible` CLI.

Several rules depend on the file's side. The side is the nearest folder named `server`, `client` or `shared` that holds the file, counted from the package root. A file under `src/server/systems` is a server file. A file under no such folder has no side.

| Code | Rule |
| --- | --- |
| CRU001 | A constructor parameter must be injectable. |
| CRU002 | A concrete behaviour has one string literal tag. |
| CRU003 | A registered class is named, non-generic, module-level and publicly constructible. |
| CRU004 | Imports use only the public Crucible entries. |
| CRU005 | `inject` names a class, an interface or `All` outside registered classes. |
| CRU006 | `boot` gets literal `load` globs inside the Rojo mapping. |
| CRU007 | An `attribute()` field has a type a Roblox attribute can hold. |
| CRU008 | A behaviour's instance type is an Instance class or a union of them. |
| CRU009 | A method name must not look like a lifecycle hook. |
| CRU010 | A type Crucible reads comes from a module, not a global declaration. |
| CRU011 | `preRender` and `renderPriority` belong to client classes. |
| CRU012 | Hooks and network callbacks are synchronous and return what their target allows. |
| CRU013 | A class extends an abstract base, not a concrete registered class. |
| CRU014 | Provided types and `All` or `inject` targets do not instantiate generic parameters. |
| CRU015 | `attribute()` initializes a behaviour instance field. |
| CRU016 | `onShutdown` belongs to server classes. |
| CRU017 | A declaration macro is an exported top-level `const` in the right module, with the right arguments. |
| CRU018 | A declaration name is valid and unique. |
| CRU019 | A declared type can be encoded. |
| CRU020 | An Instance sent to a client admits `undefined`. |
| CRU021 | `receive()` initializes a readonly Subsystem field and names one message token. |
| CRU022 | A `receive()` field is on the side that receives its target. |
| CRU023 | A declaration macro is only called, imported or exported. |
| CRU024 | A file references the server or client entry only from that side. |
| CRU025 | Profile store grants own `ProcessReceipt`. |
| CRU026 | A profile store field does not use a handle method name. |
| CRU027 | A profile store call runs on the side it reads or writes. |

## CRU001

```text
CRU001: Constructor parameter must be a plain class, interface or All<T>.
CRU001: Inject All<X> or call X.get(instance) for a concrete behaviour.
CRU001: Optional, defaulted and rest parameters are not injectable.
```

Crucible builds each registered class, a non-abstract class that extends `Subsystem` or `CrucibleBehaviour`, and fills its constructor parameters from the classes it has built. The rule checks the parameters of the class's own constructor, or of the nearest base constructor when the class has none.

A parameter type must be a class or an interface named by its declaration, or `All<T>`. The rule rejects primitives, object literal types, unions, enums, `typeof X`, Instance types such as `Part`, and generic instantiations such as `Cache<number>`. It rejects a concrete behaviour class, because many instances of it can exist. It also rejects a parameter with `?`, a default value, or `...`.

To fix it, depend on a class or an interface. To reach behaviours, take `All<Door>` and pick from it, or call `Door.get(instance)` when you have the instance. Pass any other value through a field that a hook sets. `crucible docs lifecycle` describes injection.

```ts
import { CrucibleBehaviour, Subsystem, type All } from "@trioforge/crucible";

export class Door extends CrucibleBehaviour<"Door", Model> {}

export class Doors extends Subsystem {
	constructor(private readonly doors: All<Door>) {
		super();
	}

	protected override start() {
		this.doors.forEach((door) => print(door.instance.Name));
	}
}
```

## CRU002

```text
CRU002: A concrete behaviour tag must be exactly one string literal.
```

A non-abstract class that extends `CrucibleBehaviour` has a `tag` type that is not one string literal, such as `string` or `"Door" | "Gate"`. To fix it, give each concrete behaviour one literal tag, as in `CrucibleBehaviour<"Door", Model>`. Keep a generic tag parameter only on an abstract base.

## CRU003

```text
CRU003: Registered classes must be named, non-generic, module-level classes with public constructors.
```

A non-abstract class that extends `Subsystem` or `CrucibleBehaviour` is a class expression, has no name, is declared inside a function or a block, has type parameters, or has a `private` or `protected` constructor of its own or from a base. To fix it, declare the class with a name at the top level of its module, with no type parameters and a public constructor. Move generic code into an abstract base class.

## CRU004

```text
CRU004: Import only the root, server, client, vide, util/random or maths/spline Crucible entries.
```

An `import`, `export ... from`, `import type`, `import()` call or `import x = require()` names a path under `@trioforge/crucible/` other than `server`, `client`, `vide`, `util/random` and `maths/spline`. This covers `@trioforge/crucible/emit`, internal paths, and `@trioforge/crucible/testing`, which only specs import, as `crucible docs testing` describes. To fix it, import from `@trioforge/crucible` or from one of the five entries.

The runtime no longer ships the `vide` entry, so an import of `@trioforge/crucible/vide` passes this rule and then fails to resolve. Read Charm values in Vide with `@rbxts/vide-charm`, as `crucible docs vide` describes.

## CRU005

```text
CRU005: inject requires a class, interface or All target outside registered classes.
```

An `inject<T>()` call has no type argument, has a type argument that is not a class, an interface or `All<T>`, or sits inside a non-abstract class that extends `Subsystem` or `CrucibleBehaviour`. The rule rejects targets such as `number`, `{ a: number }`, an enum, `typeof X` and a type parameter.

To fix it, take the dependency as a constructor parameter inside a registered class. Call `inject` only outside registered classes, with a class, an interface or `All<T>` as its type argument. `crucible docs lifecycle` describes both.

## CRU006

```text
CRU006: boot requires literal load globs.
CRU006: boot requires a literal load property.
CRU006: A spread or computed key after load can replace its literal globs.
CRU006: load must be an array of string literals.
CRU006: load entry must be a string literal.
CRU006: Cannot load the running compiler's Rojo resolver or project mapping.
CRU006: load prefix maps to more than one Rojo folder.
CRU006: load prefix is outside the Rojo mapping.
```

`boot` takes an object literal with a `load` property whose value is an array literal of string literals. The compiler resolves each glob's folder through the Rojo project at compile time. The last three messages mean it could not load the Rojo project, or the folder maps to no instance or to more than one. `crucible docs boot` describes the globs, the Rojo mapping and the Rojo project.

## CRU007

```text
CRU007: Attribute fields support string, number, boolean, Vector3, Color3 and literal unions.
```

A field initialized with `attribute()` has a type that a Roblox attribute cannot hold, such as an Instance, a `CFrame`, an array or an object. The field type can be `string`, `number`, `boolean`, `Vector3`, `Color3`, a string, number or boolean literal, or a union of them, and the union may include `undefined`. To fix it, store an id or a name in the attribute and resolve it in code.

## CRU008

```text
CRU008: Instance type must be an Instances class or union of them.
```

A concrete behaviour's instance type, the second type argument of `CrucibleBehaviour`, is not a class in Roblox's `Instances` list or a union of such classes. An intersection such as `Model & { Hinge: BasePart }` or an interface fails. To fix it, use the class, such as `Model`, and read children in code.

## CRU009

```text
CRU009: Method resembles a lifecycle hook. Use its exact Crucible declaration name.
```

The rule catches a misspelled hook, which would otherwise be a method that Crucible never calls. It checks each method declared in the body of a class that extends `Subsystem` or `CrucibleBehaviour`, abstract or not. It does not check fields that hold functions, getters or setters.

The hooks are the protected methods that Crucible's base classes declare: `awake`, `start`, `fixedUpdate`, `update`, `lateUpdate`, `preRender`, `onShutdown`, `onEnable`, `onDisable` and `onDestroy`. A method with one of these exact names is never CRU009, on either kind of class. A Subsystem never calls `onEnable`, `onDisable` or `onDestroy`.

Any other method name fails when one of these is true:

- The name is `onInit`, `onStart`, `onTick`, `onPhysics` or `onRender`, the hook names of other frameworks.
- The name equals a hook name when case is ignored, such as `Update` or `prerender`.
- The name becomes a hook name by inserting, deleting or replacing exactly one character. The comparison is case-sensitive. Swapping two letters counts as two edits.

A name two or more edits from every hook is allowed.

| Name | Result | Reason |
| --- | --- | --- |
| `updte` | Fails | One deletion from `update` |
| `updated`, `updates` | Fail | One insertion after `update` |
| `starts`, `stars`, `stat` | Fail | One edit from `start` |
| `awaken` | Fails | One insertion after `awake` |
| `onEnabled`, `onDisabled` | Fail | One insertion after `onEnable` and `onDisable` |
| `onShutDown` | Fails | One replaced letter in `onShutdown` |
| `Start`, `prerender` | Fail | A hook name in another case |
| `onInit`, `onStart` | Fail | Another framework's hook name |
| `started`, `restart`, `startup`, `state`, `stop` | Allowed | Two or more edits from every hook |
| `onDestroyed`, `onUpdate`, `onAwake` | Allowed | Two edits from the nearest hook |
| `init`, `tick`, `render`, `enable`, `destroy`, `shutdown` | Allowed | Two or more edits from every hook |

To fix it, give a hook its exact name with `protected override`. Give any other method a name from the allowed side of the table. `crucible docs lifecycle` lists the hooks.

## CRU010

```text
CRU010: Type comes from global declarations; import a module type for incremental correctness.
CRU010: Attribute and dependency aliases must come from module declarations.
CRU010: <name> <location>: <type> must be declared in one imported module; incremental builds cannot see global declarations or merges.
CRU010: This augmentation changes a type a Crucible declaration lowers; incremental builds cannot see the merge. Declare the type in one module.
```

Crucible records what a type looks like at compile time. An incremental build recompiles only the files that changed, so it cannot see a change to a global declaration that another file uses. The rule fails when a type that Crucible reads is declared in a script without imports or exports, or inside `declare global`. Crucible reads the types of constructor parameters, `inject` targets, provided bases and interfaces, `attribute()` fields, and the payload and value types of declaration macros. For a declaration macro, a type declared in more than one file also fails. So does a `declare global` or `declare module` block that adds to a type the declaration reaches. Types from `@rbxts` packages and the default library are exempt.

To fix it, declare the type once in a module, export it, and import it where you use it.

## CRU011

```text
CRU011: preRender and renderPriority require a client class.
```

A class that extends `Subsystem` or `CrucibleBehaviour` declares a member named `preRender` or `renderPriority`, or inherits one from a base class of your own, and its file is not on the client side. Rendering runs only on the client, as `crucible docs lifecycle` describes. To fix it, move the class under a `client` folder, or move the member to a client subclass.

## CRU012

```text
CRU012: Crucible hooks must be synchronous and must not return a Promise.
CRU012: Network callbacks cannot be async or return a Promise; spawn a thread inside the callback.
CRU012: <Target> callbacks return nothing; use a block body; this one can return <type>.
CRU012: ClientReady callbacks return nothing or a cleanup function; this one can return <type>.
```

A method with a hook's exact name is `async` or has a return type that includes a `Promise`. Or a `receive()` callback is `async` or returns a `Promise`. The callback can be a function written inline, or a function declaration, a method or a `const` arrow function that `receive` names. A `ToServer` or `ToClient` callback must return nothing, and a `ClientReady` callback returns nothing or a cleanup function. An arrow function with an expression body returns the expression's value, which is the common cause.

To fix it, make the hook or callback synchronous and start asynchronous work with `task.spawn` inside it, as `crucible docs lifecycle` describes. Give an event callback a block body. `crucible docs networking` lists what each target's callback takes and returns.

```ts
import { receive, Subsystem } from "@trioforge/crucible";
import { Collect } from "../shared/messages";

export class Coins extends Subsystem {
	private readonly collect = receive(Collect, (player, { size }) => {
		task.spawn(() => print(`${player.Name} collected ${size}`));
	});
}
```

## CRU013

```text
CRU013: Extend an abstract Crucible base, not a concrete registered class.
```

A class extends a non-abstract class that extends `Subsystem` or `CrucibleBehaviour`, directly or through a variable, an alias or a mixin. To fix it, move the shared members into an abstract class and make each concrete class extend it.

## CRU014

```text
CRU014: Provided types must not instantiate non-tag generic parameters.
CRU014: Provided interfaces must not instantiate generic parameters.
CRU014: All targets must not instantiate generic parameters except string tags.
CRU014: inject targets must not instantiate generic parameters except string tags.
```

A registered class also provides its bases and the interfaces it implements, so other classes can depend on them. Crucible identifies each provided type by its declaration, not by its type arguments. The rule fails when a non-abstract class extends a base with type arguments, such as `Store<Coins>`, or implements an interface with type arguments. The one exception is a behaviour base's tag parameter, so `extends Openable<"Door">` is allowed. In an `All<T>` or `inject<T>` target, the tag parameter is allowed only as `string`.

To fix it, provide a non-generic interface or abstract class. To reach every behaviour of a generic base, use `All<Openable<string>>`.

```ts
import { CrucibleBehaviour, Subsystem, type All } from "@trioforge/crucible";

export abstract class Openable<Tag extends string> extends CrucibleBehaviour<Tag, Model> {}

export class Gate extends Openable<"Gate"> {}

export class Openables extends Subsystem {
	constructor(private readonly openables: All<Openable<string>>) {
		super();
	}
}
```

## CRU015

```text
CRU015: attribute() must initialize a behaviour instance field.
```

`attribute()` is called anywhere other than as the whole initializer of a non-static field of a class that extends `CrucibleBehaviour`, as `crucible docs lifecycle` shows. A local variable, a static field, a Subsystem field and an expression such as `attribute() ?? 1` all fail. To fix it, write `readonly speed = attribute(16);` as a field of the behaviour, and pass a fallback instead of using `??`.

## CRU016

```text
CRU016: onShutdown requires a server class.
```

A class that extends `Subsystem` or `CrucibleBehaviour` declares a member named `onShutdown`, or inherits one from a base class of your own, and its file is not on the server side. The server calls `onShutdown` when it closes. To fix it, move the class under a `server` folder.

## CRU017

```text
CRU017: Declare <kind of declaration> as an exported top-level const in a <side> module.
CRU017: <macro> needs explicit payload types; use undefined for no payload.
CRU017: <macro> needs an explicit type argument.
CRU017: <macro> needs at most one type argument.
CRU017: askServer options must be { timeout: <number literal in (0, 120]> }.
CRU017: Unexpected declaration arguments.
CRU017: Cannot read <macro>'s value type; give it an explicit type argument.
```

Each declaration macro is the whole initializer of an `export const` at the top level of a module on the side the table names. It takes the type arguments and arguments in the table, and nothing more. `askServer` options are an object literal with one `timeout` property, a number literal greater than 0 and at most 120.

| Macro | Side | Type arguments | Arguments |
| --- | --- | --- | --- |
| `toServer`, `toClient` | shared | 1 | name |
| `askServer` | shared | 2 | name, options |
| `replicated`, `replicatedPerPlayer` | shared | 1 | name, initial value |
| `profileStore` | shared | at most 1 | name, options |
| `dataStore` | server | 1 | name, options |
| `profileMessage` | server | 1 | name |
| `config`, `playerConfig` | server | 1 | name, fallback |

The last message means a declaration has no type argument and the compiler cannot infer the value type from the call. To fix it, pass the type argument. `crucible docs networking`, `crucible docs replicated`, `crucible docs profile-stores`, `crucible docs data-stores` and `crucible docs configs` show each declaration.

## CRU018

```text
CRU018: <kind> names are 1 to <bytes> bytes of dot-separated segments [A-Za-z][A-Za-z0-9_]*; Crucible is reserved.
CRU018: <name> is already declared at <file>:<line>.
CRU018: <store> value["<field>"]: a field the owner's client receives is named by one declaration segment, [A-Za-z][A-Za-z0-9_]*; rename it or mark it serverOnly
CRU018: <store> value.<field>: its view name Crucible.Data.<store>.<field> is <n> bytes, over 64; ...
```

A declaration's first argument is a string literal in quotes. A template literal or a variable fails. The name is dot-separated segments, each a letter followed by letters, digits or underscores, and its first segment is not `Crucible`. Message, replicated, profile message and config names are at most 64 bytes, and store names at most 50.

A name is unique among declarations of one kind. `toServer`, `toClient` and `askServer` share one set of names, `profileStore` and `dataStore` share one, and `config` and `playerConfig` share one. Replicated values and profile messages each have their own.

The last two messages are about profile stores. Each top-level field the owner's client receives becomes a declaration named `Crucible.Data.<store>.<field>`, so the field name must be one segment and the whole name at most 64 bytes. `crucible docs profile-stores` covers views. To fix it, rename the field, shorten the store name, or mark the field `serverOnly`.

## CRU019

```text
CRU019: <name> <location>: <reason>
```

A declaration's payload or value type contains a part that Crucible cannot encode. `<name>` is the declaration's name. `<location>` is the path to the part, starting at `payload`, `request`, `response` or `value`, such as `value.items[]` or `value.scores.key`. `<reason>` says what is wrong and usually how to fix it.

These reasons apply to every declaration:

- `any cannot be checked; use unknown`
- `use undefined for no payload`, for `void`
- `cannot encode <type>`, for `never`, symbols, bigints, template literal types, type parameters and other types with no encoding
- `functions cannot cross the network`, for a function type or a method
- `symbol property names cannot cross the network`, `numeric property names are not supported; use a Map`, and `numeric or symbol property names are not supported; use a Map`
- `an empty object type accepts anything; use undefined`
- `a field of type undefined carries no information; remove it`
- `recursive payload type`
- `nonfinite literal`
- `conflicting widths`, `only numeric width intersections are supported`, and `only a Crucible width brand may intersect number; this brand is foreign`, for number intersections other than one of Crucible's width types
- `Roblox datatype not supported in v1`, for a Roblox datatype other than `Vector3`, `Color3`, `CFrame` and Instance classes
- `collection elements and map values cannot admit undefined`, for an array, set or map element that admits `undefined`. A map value in a replicated value, a store, a profile message or a config may admit it.
- `tuples must be fixed length with no optional or rest elements`
- `map keys must be string, u8, u16, u32 or a literal union; float keys can round to a duplicate`
- `union members <types> have no discriminant; give each a distinct literal property`
- `union has ambiguous runtime kinds`, for a union with two members of the same runtime kind, such as two array types
- `too many union branches`, for more than 256
- `only a string index signature can encode a record`
- `schema exceeds the depth limit of <n>`, where `<n>` is 32 for messages and 28 for replicated and saved values
- `schema exceeds MAX_SCHEMA_NODES`, for more than 4096 nodes

Replicated values, stores, profile messages and configs keep state, so they also fail on these:

- `replicated state cannot hold Instances; replicate a stable id`
- `Instances cannot be saved or replicated; store a stable id`
- `unknown cannot be stored or replicated state; give the value a declared type`
- `state maps are keyed by string or a union of string literals; frames carry string-keyed records`
- `an empty array has no element type; annotate it: [] as string[]`
- `<datatype> cannot be saved: a save holds JSON; store its components`, for `Vector3`, `Color3` and `CFrame` in a store, a profile message or a config
- `a store's value is a record; wrap a <type> in an object`
- `a profile store's value is an object with declared fields; wrap the record in a field`

Profile store templates also fail on these:

- `an empty object type accepts anything; annotate an empty template table: {} as Record<string, number>`
- `a position that is always undefined; annotate it: undefined as string | undefined`
- `serverOnly marks a profileStore template field; it means nothing here`, for `ServerOnly` in a replicated value, a data store, a profile message or a config
- `serverOnly marks a template field, never the store's whole value; ...`
- `this field is already inside a serverOnly field`
- `serverOnly marks a field on an object path from the template root, never an array element, a map value or a union member`

`crucible docs networking` lists the types a payload can use. `crucible docs profile-stores` describes templates and `serverOnly`.

## CRU020

```text
CRU020: <name> <location>: a client may not have streamed this Instance in; admit undefined or wrap collection elements in objects with an optional field.
```

A `toClient` payload or an `askServer` response holds an Instance type that does not admit `undefined`. A client may not have streamed the Instance in. `Player` alone is exempt. To fix it, declare the field as `BasePart | undefined`. An array element cannot admit `undefined`, so wrap each element in an object with an optional field.

```ts
import { toClient } from "@trioforge/crucible";

export const Highlight = toClient<{ readonly parts: { readonly part?: BasePart }[] }>("Effects.Highlight");
```

## CRU021

```text
CRU021: receive() must initialize a readonly instance field of a Subsystem.
CRU021: A receiver field cannot shadow an inherited receiver.
CRU021: receive target must be exactly one message token or ClientReady.
```

`receive()` is the whole initializer of a `readonly`, non-static field of a class that extends `Subsystem`. Its field name is not the name of a `receive()` field in a base class. Its first argument has the type of one `ToServer`, `ToClient` or `AskServer` token, or is `ClientReady`. A union of tokens fails. To fix it, declare one `private readonly` field per message on the Subsystem, as `crucible docs networking` shows.

## CRU022

```text
CRU022: <Target> must be received by a <side> Subsystem.
```

A `receive()` field is in a file on the wrong side. `ToClient` targets are received on the client. `ToServer`, `AskServer` and `ClientReady` targets are received on the server. To fix it, move the Subsystem under the matching `server` or `client` folder.

## CRU023

```text
CRU023: <macro> is a declaration macro and cannot be used as a value.
```

A declaration macro is used other than by calling it, importing it or exporting it. The compiler replaces each call at compile time, so a macro passed as an argument, stored in a variable or read through a namespace import such as `C.toServer` or `C["toServer"]` cannot work. The macros are `toServer`, `toClient`, `askServer`, `receive`, `replicated`, `replicatedPerPlayer`, `profileStore`, `dataStore`, `profileMessage`, `config` and `playerConfig`. To fix it, import the macro by name and call it directly.

## CRU024

```text
CRU024: This file cannot reference @trioforge/crucible/<side>; move it under a <side> folder.
CRU024: This file cannot reference @trioforge/crucible/<side> through <name>; move it under a <side> folder.
```

A file not on the server side imports or names something from `@trioforge/crucible/server`, or a file not on the client side does so for `@trioforge/crucible/client`. The second message is for a value whose type comes from a side entry, such as a `dataStore` handle used in a shared file. Each side entry checks at load time that it runs on its side. To fix it, move the code under the matching folder. Keep shared code on the root entry, `@trioforge/crucible`.

## CRU025

```text
CRU025: Crucible's data module owns ProcessReceipt; register grants with a profile store's grant(productId, ...).
```

Code assigns `MarketplaceService.ProcessReceipt` in a game that calls `grant` on a profile store anywhere. Roblox keeps one `ProcessReceipt` callback, and Crucible sets it to route purchases to the grants. To fix it, register each product with the store's `grant(productId, ...)` and remove the assignment. `crucible docs profile-stores` describes grants.

## CRU026

```text
CRU026: <store> <location>: `<field>` is a handle method name (set, update, insert, remove); rename the field
```

A profile store template has a field named `set`, `update`, `insert` or `remove` at any depth. Handles expose these names as methods, so a field cannot use them. To fix it, rename the field.

## CRU027

```text
CRU027: <store>() reads the local player's profile; on the server call <store>(player).
CRU027: <store>.loaded() reads the local player's profile; on the server call <store>.loaded(player).
CRU027: <store>(player) is server-only; the client reads <store>().
CRU027: <store>.<method> is server-only; the client reads <store>().
```

A profile store call runs on the wrong side. `store()` and `store.loaded()` with no arguments read the local player's profile, so they belong in client files. `store(player)`, `store.loaded(player)` and every other method, such as `save`, `onLoad` or `grant`, belong in server files. A file with no side fails both ways. To fix it, move the call to a file on the matching side, or call the form for that side. `crucible docs profile-stores` lists the methods.

CRU028 to CRU031 are retired and will never be reused; CRU037 was removed with Crucible UI and will never be reused.

## Type-aware rules

Only the Crucible compiler reports CRU032 to CRU036. It checks these rules in its fact pass. Each diagnostic is an error and stops compilation. These checks also apply outside registered Crucible classes, except for the lifecycle rules that require Crucible ancestry.

| Code | Rule | Message |
| --- | --- | --- |
| CRU032 | Lua truthiness | 0, NaN, and "" are falsy in TS. Compare explicitly, because Luau treats them as truthy. |
| CRU033 | Misleading LuaTuple checks | Unexpected LuaTuple in conditional expression. Add [0]. |
| CRU033 | LuaTuple declarations | Unexpected LuaTuple in declaration, use array destructuring. |
| CRU034 | Array pairs | Do not use Array<T> with pairs(). Key values will not be shifted from 1-indexed to 0-indexed. |
| CRU034 | Array ipairs | Do not use Array<T> with ipairs(). Key values will not be shifted from 1-indexed to 0-indexed. |
| CRU035 | Frame connections | This `<signal>` connection lasts the object's whole life. Implement Crucible's update, fixedUpdate, lateUpdate or preRender instead. |
| CRU036 | Subsystem destruction | Crucible never destroys a subsystem. Do last writes in onShutdown, and give other lifetimes their own owner. |
| CRU036 | Subsystem Janitor | A subsystem lives for the whole session. Put the Janitor on the lifetime that ends. |

CRU032 checks conditions, negation, and the left operand of `&&` and `||`. In an `if` condition using `??`, it checks the right operand. Numeric and string literals that cannot be zero or empty are allowed. Explicit boolean coercion with `!!` is allowed because the compiler preserves JavaScript truthiness. Generic constraints, unions, intersections, `any`, `unknown`, and empty object types participate in the check.

CRU033 checks LuaTuple values in conditions, logical operands, equality operands, and iteration. A single variable cannot capture a LuaTuple return or iterator step. Array destructuring is allowed. An unused `_` iterator binding discards the step and is allowed. Direct iteration over a LuaTuple is allowed because the compiler packs the returned values into an array.

CRU034 checks arrays passed to `pairs` and `ipairs`. Maps and unconstrained values are allowed.

CRU035 checks direct `RunService` frame signal connections from a constructor, `awake`, or `start` in a Subsystem or CrucibleBehaviour, including inherited abstract bases. It covers `Connect`, `Once`, and `ConnectParallel` on `Heartbeat`, `PostSimulation`, `Stepped`, `PreSimulation`, `RenderStepped`, `PreRender`, and `PreAnimation`. Nested callbacks and other methods are allowed. A behaviour may assign a connection to `this.field` when its own or an inherited `onDestroy` calls `this.field.Disconnect()` or `this.field?.Disconnect()`.

CRU036 rejects a `destroy` method and a field initialized by `Janitor()` or `new Janitor()` on a Subsystem. Other lifetime owners and Janitors returned by another function are allowed.

During migration, these five rules retain the matching `oxlint-disable-next-line` directives from `roblox-ts/lua-truthiness`, `roblox-ts/misleading-lua-tuple-checks`, `roblox-ts/no-array-pairs`, `fragile-delivery/frame-hooks`, and `fragile-delivery/session-lifetime`.
