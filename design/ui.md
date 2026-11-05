# Crucible UI design

Crucible UI is the declarative UI library built into Crucible. Games write JSX, the JSX renders from Crucible's reactive state, and `className` strings in a Tailwind-like vocabulary become Roblox properties at build time. Anvil's headless primitives move into the same entry, so one package gives a game both the renderer and unstyled, accessible components. It replaces Vide in Trio Forge's stack. This document records the design and the measurements it rests on, and the order the work lands in.

## Goal and pass bar

A game has one reactive system and one update pass per frame. UI code reads `state`, `computed`, and any other `Readable` directly, with no bridge. A fixed class string costs nothing at runtime, and an unknown class fails the build with its location.

The renderer has to be at least as fast as Vide, because Fragile Delivery (FD) runs its whole UI on Vide today. The gate below decides whether the renderer gets built.

| Measure, per element, 1,000 elements | Vide 0.6.1 | Spike variant B, on #65 | Gate |
|---|---|---|---|
| Mount | 24 µs | 15 µs | no slower than Vide |
| Update, separate states | 4.8 µs | 7.4 µs | within 10% of Vide |
| Update, one shared state | 3.7 µs | 6.1 µs | within 10% of Vide |
| Memory | 1.5 KB | 2.2 KB | within 10% of Vide |

The spike measured these in Lune on a fake Instance, median of 15 runs. Lune numbers rank the variants. They do not predict Studio frame times.

## What Vela shows

[Vela](https://github.com/astra-void/vela-rbxts) is a roblox-ts UI library with Tailwind-style classes. The planning research read it as a reference and found it worth learning from but not worth depending on. It is young, has one maintainer, and is pre-1.0.

- **The vocabulary carries over.** `rounded-*` becomes a UICorner, `p-*` a UIPadding, `border` a UIStroke, and `flex` with `gap` a UIListLayout. Sizes use a 4 px scale.
- **The variants carry over.** `hover:`, `active:`, `focus:` (which also covers gamepad selection), breakpoints, orientation, `touch:`, `mouse:`, `gamepad:`, `dark:`, and `group-*`.
- **The runtime parser does not.** Vela parses classes in the game whenever a class string is built at runtime or uses a variant, and it drops unknown classes without an error. Crucible's compiler sees types, so it resolves every class at build time and reports the ones it does not know.
- **Re-rendering does not.** Vela renders with React, so a hover change re-renders the element. Here a hover change updates only the properties that `hover:` sets.

## The design

### One entry, `@trioforge/crucible/ui`

The renderer, the class runtime, the JSX types, and the primitives live in one side entry with its own physical `ui` folder, like `vide`. The root entry never imports it. Only Crucible's compiler builds code that uses it, for the reason given under [Two JSX lowerings](#two-jsx-lowerings).

### Components run once

A component is a function that runs once inside a reactive `scope` and returns instances. It owns its instances, connections, and effects through `onCleanup` and `own`, and they end when its scope ends. `Show` mounts and unmounts a branch on a condition, and `For` keeps a keyed list. Nothing re-renders. Vide uses the same model, so FD's components keep their shape when they move.

### The compiler emits instance code

The compiler lowers an intrinsic element, such as `<textlabel>`, straight to instance creation. It reads each prop's declared type and value type and emits one of three things:

- **A fixed value** becomes an assignment when the instance is created.
- **A `Readable<T>`** becomes a binding. A `Readable` is any function of no arguments (`reactive.ts`), and a `State` is a callable table, so the compiler decides from the type and never from a runtime `typeof`.
- **An event prop**, such as `Activated`, becomes a connection. Event props and reactive props are both functions, and only the element's declared prop types tell them apart.

```tsx
<textlabel Text={() => `Coins ${coins()}`} TextSize={18} Activated={buy} />
```

```lua
local e = Instance.new("TextLabel")
e.TextSize = 18
bind(e, "Text", function() return "Coins " .. coins() end)
e.Activated:Connect(buy)
```

A component element, such as `<Button>`, lowers to a call with a props table. Children, fragments, and spread attributes lower the same way in both cases.

### Bindings are their own node kind

A binding reads one `Readable`, tracks what it read, and on change calls one sink on the reactive flush. It has no worker, no cleanup list, and no ownership frame of its own. It ends with its owner. The caller reads the initial value untracked and writes it when the instance is created, so the first frame never shows a default value.

The binding is a node kind in the reactive graph, not an effect with flags. The spike's variant B is this node, and it is the only renderer variant that came close to Vide. Variant A, one ordinary effect per prop, lost every row by 3 to 6 times.

### Classes compile to style tables

The compiler resolves each class token to a rule. An element's style groups those rules by target:

```
Value   = typed write, such as Size(X, Offset) or TextColor3(Rgb)
Rule    = { variant: Variant, value: Value }
          -- Target is derived from Value.
Target  = Prop(name) | Modifier(kind, prop)
          -- kind: UICorner, UIPadding, UIStroke, UIListLayout, UIGradient, size and aspect constraints
Variant = Base | Hover | Press | Focus | Disabled | Breakpoint(name) | Orientation(name)
        | Input(Touch | Mouse | Gamepad) | Dark | Group(name, Variant)
Style   = Map<Target, ordered list of (Variant, Value)>
```

Emission follows from the shape.

- A target with only a `Base` entry is a fixed value. It becomes an assignment, or a modifier child created with the element.
- A target with variant entries becomes one binding. The binding picks the last entry whose variant is active, the same precedence a later class has over an earlier one.
- A `className` whose type is a union of literals, such as `open ? "bg-slate-600" : "bg-slate-800"`, compiles every member. The element binds to the condition and switches between precompiled styles.
- A `className` passed to a component reaches the intrinsic element as a precompiled `Style`. When a component merges its own classes with the caller's, the runtime merges two style tables and parses nothing.
- A `className` of type `string`, or a token the theme does not define, is a compile error at the token's location.

The theme defines colors, spacing, radii, text sizes, and breakpoints in one data file that the compiler reads from beside the tsconfig, the same way it finds a game's Rojo project.

### Primitives come from Anvil

Anvil is about 3,900 lines of headless primitives on Vide: presses, selection and focus scopes, toggles, checkboxes, switches, radio groups, sliders, tabs, text fields, dialogs, toasts, the topbar, and pan and zoom. They move into `@trioforge/crucible/ui` unstyled, and the Anvil repository is archived after the move.

Anvil's Vide imports are `effect` (11 files), `source` (10), `untrack` (9), `read` (9), `cleanup` (8), `derive` (6), and `root`, `context`, and `batch` once each. Most map to Crucible's existing exports: `state`, `computed`, `effect`, `untracked`, `onCleanup`, and `scope`. The port adds three things to the reactive core: a context that flows down the ownership tree, `batch`, and `read` for a value that may be a `Readable`.

### One environment state drives variants

`hover:`, `active:`, and `focus:` read the same interaction state that the press and focus primitives keep, so a styled button and a headless press agree on what is pressed. Breakpoints, orientation, input mode, and `dark:` read one client-wide environment state. Anvil's `installAnvil` adapter supplies the input device today. In Crucible, a client subsystem keeps the environment, and a game has no install call.

## Two JSX lowerings

The compiler lowers JSX two ways while FD moves off Vide.

- **A file whose JSX targets Vide** lowers each element to a plain `Vide.jsx` call. FD has 2,106 elements, and only its 17 spread attributes need a fact. This lowering lets FD build with the Crucible compiler before its UI moves.
- **A file whose JSX targets `@trioforge/crucible/ui`** gets the instance code above.

The Vide lowering is deleted when the last FD screen moves.

Crucible UI has no `rbxtsc` transformer. Phase 6 of the [compiler design](compiler.md) moves `crucible build` onto the Crucible compiler and deletes `transformer-src`, so a second build of the class compiler as a transformer would have been thrown away within one phase. A game that uses Crucible UI builds with `crucible build`.

## Phases

Each phase ends in a check that a reviewer can repeat.

1. **Fix the reactive core's write and flush cost, and add the binding node.** A state write allocates nothing, a flush skips its sort when the batch is already in creation order, and the binding node joins the graph. The Vide bridge moves onto the binding. The check is `pnpm check`, new Lune specs for the binding, and the spike's benchmark rerun against trunk and Vide. If the gate in [Goal and pass bar](#goal-and-pass-bar) fails, stop and decide again before phase 2.
2. **Add JSX to the compiler.** Add both lowerings, the `ui` entry with `bind`, `Show`, `For`, and mounting, and the JSX types for intrinsic elements. The check is Lune specs that mount, update, and unmount trees on the sample game built by the compiler, and FD's Vide JSX compiling unchanged.
3. **Add classes.** Add the theme file, fixed classes, and modifiers first, then variants and the environment subsystem. The check is compiler tests that pin each class to the properties and children it emits, and a compile error for each bad input listed above.
4. **Move Anvil in.** Port the primitives and Anvil's Lune specs into `@trioforge/crucible/ui`, and add context, `batch`, and `read` to the reactive core. The check is every ported spec passing, then archiving the Anvil repository and deprecating `@trioforge/anvil`.
5. **Move FD's screens.** FD switches its UI one screen at a time once it builds with the Crucible compiler. A Studio smoke run checks hover, gamepad selection, and breakpoints on the first screen. When no screen imports Vide, delete the Vide lowering, `@trioforge/crucible/vide`, and the optional `@rbxts/vide` peer dependency.

## Decisions

These are settled.

- **Home.** Crucible UI is the `@trioforge/crucible/ui` entry of the runtime package, and the compiler owns its JSX lowering and class resolution. One PR can change both, so a game never runs a renderer that disagrees with its compiler.
- **Anvil.** Anvil's primitives move into the `ui` entry, and Anvil stops being a separate package.
- **No runtime class parser.** Every class resolves at build time. The runtime only applies and merges precompiled styles.
- **Compiler only.** Crucible UI is never built by `rbxtsc`.
- **No re-renders.** Components run once, as in Vide.
- **Opt-in.** A file gets the compiled lowering with `jsx: "react-jsx"` and `jsxImportSource: "@trioforge/crucible/ui"`, or the `@jsxImportSource` pragma. TypeScript 5 and 7 both read the JSX types from `@trioforge/crucible/ui/jsx-runtime`, so the editor checks each prop against `T | Readable<T>` and each event against its handler. A file with `jsx: "react"` and a `jsxFactory` gets the Vide lowering. Any other setting is a compile error.
- **Tests.** Phase 2's Lune specs run in the compiler's ignored tests, on a fixture game in `cli/crates/compiler/tests/jsx/ui` and the compiler-built runtime. The sample game that `pnpm check` builds uses no Crucible UI JSX.

- **Theme ownership.** `theme.toml` beside the invoked tsconfig is the game's complete theme. No defaults, overlays, or upward search. `spacing` is pixels per step, `[color]` holds flat colors or one level of shades, `[text]` holds TextSize values from 1 to 100, and `[radius]` holds whole-pixel radii. Unknown sections fail. Breakpoints wait for variants.
- **Fixed className.** GuiObject intrinsics accept `className?: string`. The compiler requires a literal, a template without substitutions, or a const name or readonly property of a const with one string-literal type. Calls, mutable names, reactive strings, unions, and component forwarding wait for later work. Typed writes derive their targets. One private style owns the ordered rules for each target. Later tokens win, and explicit properties own their properties regardless of attribute order.
- **Units.** Spacing steps produce whole-pixel offsets. Size fractions and full keywords use Roblox scale. A single Size axis preserves the other axis. A `UIScale` can scale the subtree without runtime work per class.

## Open questions

- **Editor completion.** Build errors give a location, but completing class names in the editor needs a TypeScript plugin or language server.

## Risks

- **Deferred effects.** Crucible's effects run on the flush, and Vide's run inside the write. A primitive that writes state and reads a derived result in the same tick may act differently after the port (inferred). Phase 4's ported specs show it.
- **The gate may fail.** If phase 1 cannot bring updates and memory within 10% of Vide, the renderer is slower for FD. Phase 3's classes could still target the Vide lowering in that case, because the class output is plain properties and modifier children.
- **Lune is not Studio.** The fake Instance proves resolution and lifecycle, not layout or input. Phase 5 depends on the Studio smoke run.

## Evidence

- The renderer spike is `origin/spike/ui-renderer` at `e3e7109`, folder `spike/ui`. It holds hand-written Luau for what the lowering would emit, in variants A and B, the same UI in vendored Vide 0.6.1, the benchmark `bench.luau`, and `check.luau`, which asserts that all three behave the same.
- FD's UI is 183 TSX files, about 17,000 lines, all on Vide. Its most-used Vide features are `source`, `cleanup`, `Show`, `read`, `For`, and `effect`.
- Anvil's import counts come from its `src` folder at `047e7c8`.
