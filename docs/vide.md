# Vide

Read Crucible reactive values from Vide components with `useReadable`.

Crucible's reactive graph and Vide's graph are separate. A Vide effect or JSX binding does not track a Crucible read, so a component that calls `coins()` directly renders once and never updates. `useReadable` turns a Crucible value into a Vide source that follows it.

Import it from `@trioforge/crucible/vide`. The subpath needs `@rbxts/vide` 0.6 installed in the game. It is an optional peer dependency. The root, server and client entries never load Vide, so a game without Vide does not need it.

```ts
import { useReadable } from "@trioforge/crucible/vide";
```

## Bind a value

`useReadable(readable)` returns a Vide source, a function of no arguments that Vide tracks. `readable` is any Crucible `Readable`. That covers a `state`, a `computed`, a replicated declaration, and a getter function that reads them.

```tsx
import Vide from "@rbxts/vide";
import { computed, state } from "@trioforge/crucible";
import { useReadable } from "@trioforge/crucible/vide";

export const coins = state(0);
const label = computed(() => `${coins()} coins`);

export function CoinCounter() {
	const text = useReadable(label);
	const rich = useReadable(() => coins() >= 100);
	return <textlabel Text={text} TextColor3={() => (rich() ? Color3.fromRGB(255, 200, 0) : new Color3(1, 1, 1))} />;
}
```

A replicated declaration binds the same way on the client.

```tsx
import Vide from "@rbxts/vide";
import { replicated } from "@trioforge/crucible";
import { useReadable } from "@trioforge/crucible/vide";

export const Round = replicated<number>("Match.Round", 0);

export function RoundLabel() {
	const round = useReadable(Round);
	return <textlabel Text={() => `Round ${round()}`} />;
}
```

## When it updates

- `useReadable` reads `readable` once when you call it, without tracking the read in Crucible. The first render has the current value.
- Later changes reach the Vide source on Crucible's reactive flush, which runs once per VM through `task.defer` after the first change. Several writes before the flush make one update with the last value. A value that changes and changes back before the flush makes no update. A change is a value that is not `rawequal` to the last one, as for `watch`.
- On a client that has not finished `boot`, updates wait until boot commits. The value read at the call is still the current one.

`@rbxts/vide-charm`'s `useAtom` behaves differently. Charm updates the Vide source at every atom write, inside the call that writes. With `useReadable`, code that writes a state and then reads the Vide source in the same call sees the old value. Read the Crucible value itself in that code, such as `coins()` in a click handler.

## Lifetime

Call `useReadable` inside a Vide owner: a component that Vide is rendering, or the callback of Vide's `root`. Outside one it throws `useReadable() needs a Vide owner: call it inside a component or root()` and creates nothing.

Each call makes a `watch` in its own root Crucible scope. Vide's `cleanup` ends that scope when the owner is destroyed: when you call the cleanup that `mount` or `root` returned, or when a `show` or `switch` branch unmounts the component. The binding does not belong to the Crucible owner around the call. A component mounted from a behaviour's `onEnable`, or from inside an effect run, keeps its bindings until Vide unmounts it. Pass the cleanup `mount` returns to `own` to unmount at disable.

```tsx
import Vide, { mount } from "@rbxts/vide";
import { CrucibleBehaviour, own, state } from "@trioforge/crucible";
import { useReadable } from "@trioforge/crucible/vide";

const score = state(0);

function Score() {
	const value = useReadable(score);
	return <textlabel Text={() => tostring(value())} />;
}

export class Scoreboard extends CrucibleBehaviour<"Scoreboard", ScreenGui> {
	protected override onEnable() {
		own(mount(() => <Score />, this.instance));
	} // At disable Vide unmounts Score, which ends its watch.
}
```

## Errors

`useReadable` adds no error handling of its own. It follows `watch`.

- If `readable` throws at the call, `useReadable` rethrows and creates nothing. The Vide component or `root` fails as it would for any error in it.
- If `readable` throws on a later flush, the binding's watch stops for good. Crucible reports the failure once through its diagnostics. The Vide source keeps its last value and gets no more updates, even after `readable` stops throwing. Remount the component to bind again.
- The same happens when a Vide effect throws while it handles an update, because Vide runs those effects inside the write to the source.

## Cost

Each binding is one Crucible `watch` plus one Vide source, and nothing is shared between bindings of the same readable. In Lune at optimization level 2, with 1,000 bindings, a binding costs 80 to 90 µs to create, flush once and destroy, and 43 to 46 µs per update. A bare Crucible `watch` costs about 65 µs and 35 to 39 µs of that. A Vide source written directly costs about 6 µs and 3 µs. Measure again with `lune run bench-vide` after `pnpm build`.

## Migrate from vide-charm

| Charm and vide-charm | Crucible |
| --- | --- |
| `atom(0)` | `state(0)` |
| `coins(5)` | `coins.set(5)` |
| `coins((current) => current + 1)` | `coins.update((current) => current + 1)` |
| `computed(() => coins() * 2)` | `computed(() => coins() * 2)` |
| `peek(coins)` | `untracked(coins)` |
| `useAtom(coins)` | `useReadable(coins)` |
| `useAtom(() => coins() * 2)` | `useReadable(() => coins() * 2)` |

Only the imports and the writes change in a component. Reads stay calls.

```tsx
import Vide from "@rbxts/vide";
import { state } from "@trioforge/crucible";
import { useReadable } from "@trioforge/crucible/vide";

export const coins = state(0);

export function Wallet() {
	const amount = useReadable(coins);
	return (
		<textbutton
			Text={() => `${amount()} coins`}
			Activated={() => coins.update((current) => current + 1)}
		/>
	);
}
```
