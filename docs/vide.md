# Vide

Render UI with Vide, and read Charm values in components with `@rbxts/vide-charm`.

Crucible has no UI entry of its own. A game renders UI with `@rbxts/vide` and reads Charm values in components with `useAtom` from `@rbxts/vide-charm`. The game lists Charm, Vide and `vide-charm` in its own `dependencies`, as a game made with `crucible create` does. Crucible imports neither Vide nor `vide-charm`.

```ts
import Vide from "@rbxts/vide";
import { useAtom } from "@rbxts/vide-charm";
```

`vide-charm` loads the Charm and Vide folders next to it in the place, which are the game's own. Keep one copy of each, as "Install Charm" in `crucible docs reactive` describes.

## Compile JSX

`crucible create` writes these options into a new game's tsconfig, and `crucible upgrade` adds them to a tsconfig you have not edited. Otherwise, set them yourself:

```json
{
	"compilerOptions": {
		"jsx": "react",
		"jsxFactory": "Vide.jsx",
		"jsxFragmentFactory": "Vide.Fragment"
	}
}
```

Automatic JSX modes (`react-jsx` and `react-jsxdev`) and `jsxImportSource` fail compilation at the tsconfig with this fix. Crucible no longer exports `@trioforge/crucible/ui` or compiles `className` and `theme.toml` into instance styles.

## Bind a value

`useAtom(getter)` returns a Vide source that follows a Charm value. `getter` is a function of no arguments that reads Charm values: an atom, a computed, a replicated declaration, or a getter that reads them. A profile handle is a callable table, not a function, and `useAtom` throws when it is given one. Pass `useAtom(() => handle())` instead. The types do not catch it.

```tsx
import { atom, computed } from "@rbxts/charm";
import Vide from "@rbxts/vide";
import { useAtom } from "@rbxts/vide-charm";

export const coins = atom(0);
const label = computed(() => `${coins()} coins`);

export function CoinCounter() {
	const text = useAtom(label);
	const rich = useAtom(() => coins() >= 100);
	return <textlabel Text={text} TextColor3={() => (rich() ? Color3.fromRGB(255, 200, 0) : new Color3(1, 1, 1))} />;
}
```

A replicated declaration binds the same way on the client.

```tsx
import Vide from "@rbxts/vide";
import { useAtom } from "@rbxts/vide-charm";
import { replicated } from "@trioforge/crucible";

export const Round = replicated<number>("Match.Round", 0);

export function RoundLabel() {
	const round = useAtom(Round);
	return <textlabel Text={() => `Round ${round()}`} />;
}
```

## When it updates

`useAtom` reads `getter` when you call it, so the first render has the current value. It then updates the Vide source inside the Charm write that changes the value, or when the outermost batch ends.

- A write from a `receive` handler or a `start` hook updates the source at once.
- A write from a hook Crucible batches, such as `onEnable`, updates it after the hook returns.
- A replicated declaration updates when the client applies the server's frame.

## Lifetime

Call `useAtom` inside a Vide owner: a component that Vide is rendering, or the callback of Vide's `root`. Outside one, Vide throws `cannot cleanup outside a stable or reactive scope` after the Charm listener exists, and nothing ever ends that listener.

Vide's `cleanup` ends the binding when its owner is destroyed: when you call the cleanup that `mount` or `root` returned, or when a `show` or `switch` branch unmounts the component. The binding does not belong to the Crucible owner around the call. A component mounted from a behaviour's `onEnable` keeps its bindings until Vide unmounts it. Pass the cleanup `mount` returns to `own` to unmount at disable.

```tsx
import { atom } from "@rbxts/charm";
import Vide, { mount } from "@rbxts/vide";
import { useAtom } from "@rbxts/vide-charm";
import { CrucibleBehaviour, own } from "@trioforge/crucible";

const score = atom(0);

function Score() {
	const value = useAtom(score);
	return <textlabel Text={() => tostring(value())} />;
}

export class Scoreboard extends CrucibleBehaviour<"Scoreboard", ScreenGui> {
	protected override onEnable() {
		own(mount(() => <Score />, this.instance));
	} // At disable Vide unmounts Score, which ends its binding.
}
```

A binding is a raw Charm callback, so it gets no Crucible ownership. A Vide effect that an update runs runs inside the write, where Crucible's owner is the writer's. Do not call `effect`, `watch`, `scope`, `onCleanup` or `own` from a Vide effect.

## Errors

A Vide effect that throws while it handles an update fails the Charm write that caused it, as a failing effect does. A hook Crucible batches reports it after the hook returns. A `receive` handler or a `start` hook sees the write throw.

Vide effects must not yield. In Studio, Charm's strict mode fails a yield in the update. Start yielding work with `task.spawn`.

## Migrate from `@trioforge/crucible/vide`

The `@trioforge/crucible/vide` entry and its `useReadable` are gone. Bind Charm values with `useAtom` instead.

| Before | Now |
| --- | --- |
| `import { useReadable } from "@trioforge/crucible/vide"` | `import { useAtom } from "@rbxts/vide-charm"` |
| `useReadable(coins)` | `useAtom(coins)` |
| `useReadable(() => coins() * 2)` | `useAtom(() => coins() * 2)` |

`crucible docs reactive` lists how Crucible's own values became Charm's.
