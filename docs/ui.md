# UI

Write JSX that creates Roblox instances and binds them to Crucible's reactive values, with `@trioforge/crucible/ui`.

Crucible UI has no virtual tree and no re-render. Crucible's compiler turns each element into instance code: it creates the instance, sets each fixed prop once, binds each prop whose value is a `Readable`, and connects each event. A component is a function that runs once and returns instances.

Only Crucible's compiler builds this JSX. `crucible build` still runs `rbxtsc`, which cannot build it, so a game that uses it builds with the compiler. The root, server and client entries never load the `ui` entry.

## Opt a file in

Set the JSX options in the tsconfig. TypeScript then reads the element types from `@trioforge/crucible/ui/jsx-runtime`, and the compiler lowers each element to instance code.

```json
{
	"compilerOptions": {
		"jsx": "react-jsx",
		"jsxImportSource": "@trioforge/crucible/ui"
	}
}
```

A file can also opt in alone with the pragma `/** @jsxImportSource @trioforge/crucible/ui */` when its tsconfig sets `jsx: "react-jsx"`. A tsconfig that sets `jsx: "react"` with `jsxFactory: "Vide.jsx"` lowers to Vide calls instead.

## Write elements

An intrinsic element is a creatable class name in lowercase, such as `textlabel` or `frame`. Each prop takes one of three kinds of value, and the prop's type decides which.

- A plain value, such as `TextSize={18}`, sets the property once when the element creates its instance.
- A `Readable`, such as a `state`, a `computed` or `() => ...`, binds the property. The element sets the current value at creation, so the first frame shows it, and sets it again on each reactive flush where the value changed.
- An event, such as `Activated`, takes a handler. The element connects it, and destroying the instance disconnects it.

```tsx
import { state } from "@trioforge/crucible";

export const coins = state(0);

export function CoinButton() {
	return (
		<textbutton
			Text={() => `Coins ${coins()}`}
			TextSize={18}
			Activated={() => coins.update((n) => n + 1)}
		/>
	);
}
```

The compiler decides from types, never from the value at run time, because a `state` is a callable table. A value whose type may be either a `Readable` or a plain value, such as `string | Readable<string>`, fails the build at the prop. So does a spread attribute on an intrinsic element. Pass one kind or the other.

A prop whose value may be `undefined`, such as an optional callback a component forwards, is set, bound or connected only when the value is not `undefined`. Otherwise the instance keeps its default, as with a prop left out.

A child element is parented to the element. Any other child, such as `props.children`, a component, a fragment or an array, is appended through `append`. A component receives its children in `props.children`: the child itself when there is one, and an array when there are several.

```tsx
import { type Node } from "@trioforge/crucible/ui";

export function Card(props: { title: string; children?: Node }) {
	return (
		<frame Name={props.title}>
			<uicorner />
			{props.children}
		</frame>
	);
}
```

## Mount a component

`mount(component, parent)` runs `component` once in a new scope and parents what it returns to `parent`. It returns a cleanup. The cleanup destroys the instances that the component returned, which also disconnects their events, and ends their bindings and effects. Inside another owner, such as a subsystem's `onStart`, the mount also ends when that owner ends.

```tsx
import { mount } from "@trioforge/crucible/ui";

export function showHud(playerGui: PlayerGui) {
	return mount(
		() => (
			<screengui Name="Hud">
				<textlabel Text="Ready" />
			</screengui>
		),
		playerGui,
	);
}
```

A component's elements need an owner. Create them inside `mount`, or inside another scope.

## Show a branch

`Show` mounts `children()` while `when()` is truthy, as JavaScript defines it, and `fallback()` otherwise. The first branch mounts at the call. On a reactive flush where the condition flips, the shown branch's scope ends, which destroys its instances, and the other branch mounts. The branch that is shown also ends with the owner that called `Show`.

```tsx
import { state } from "@trioforge/crucible";
import { Show } from "@trioforge/crucible/ui";

export const open = state(false);

export function Shop() {
	return (
		<frame>
			<Show when={open} fallback={() => <textlabel Text="Closed" />}>
				{() => <textlabel Text="Open" />}
			</Show>
		</frame>
	);
}
```

## Show a list

`For` mounts `children(item, index)` once for each item of `each()`, keyed by the item itself. `index` is a `Readable` that counts from 0. On a reactive flush where `each()` returns another array, an item that is still there keeps its row and gets its new index, a new item mounts a row, and a removed item's row ends. A write that changes the array in place is not seen, so set a new array. An array that holds the same item twice throws.

Roblox does not order children by when they were parented. Bind `LayoutOrder` to the index and add a `UIListLayout` to show the rows in order.

```tsx
import { state } from "@trioforge/crucible";
import { For } from "@trioforge/crucible/ui";

export const names = state<readonly string[]>(["Ana", "Bo"]);

export function Roster() {
	return (
		<frame>
			<uilistlayout SortOrder={Enum.SortOrder.LayoutOrder} />
			<For each={names}>{(name, index) => <textlabel Text={name} LayoutOrder={index} />}</For>
		</frame>
	);
}
```

## What compiled JSX calls

The compiler emits calls to two more exports. You rarely call them yourself.

- `bind(instance, property, readable)` sets `instance[property]` to the value of `readable` now, then again on each reactive flush where it changed, until the current owner ends. With no owner it throws.
- `append(parent, node)` parents each instance in `node` to `parent`, mounts a `Show` or `For` there, and walks an array.

The prop types are exported as `IntrinsicProps<T>`, `PropertyProps<T>` and `EventProps<T>`, for a component that forwards props to an element.
