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

The compiler decides from types, never from the value at run time, because a `state` is a callable table. A value whose type may be either a `Readable` or a plain value, such as `string | Readable<string>`, fails the build at the prop. So does a `Readable` that may be `undefined`, and a spread attribute on an intrinsic element. Pass one kind or the other.

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

## Style with classes

`className` on a GuiObject intrinsic resolves at build time. Fixed classes emit assignments and modifier children. They add no runtime class parser, style table, binding, or UI import.

Place `theme.toml` beside the tsconfig you invoke. The file owns the whole theme. There are no built-in theme values, overlays, or upward searches. This starter defines a spacing step, colors, text sizes, and corner radii.

```toml
spacing = 4

[color]
white = "#ffffff"
black = "#000000"
brand = "#2563eb"

[color.slate]
600 = "#475569"
800 = "#1e293b"
900 = "#0f172a"

[text]
sm = 14
base = 16
lg = 18

[radius]
DEFAULT = 4
md = 6
lg = 8
```

Names start with a lowercase letter and contain lowercase letters and digits. Color shade names also allow a leading digit. `DEFAULT` is allowed only in `[radius]`. Colors use `#rrggbb`. Spacing is a positive finite number at most 1,000,000. Text sizes are numbers from 1 to 100. Radii are nonnegative whole pixels. Duplicate names, class collisions, unknown sections, and invalid values fail with CRU030 at the theme key or value. A broken theme skips class resolution to avoid further unknown-class errors.

```tsx
export function BuyButton() {
	return (
		<textbutton
			className="bg-brand/90 text-white text-lg size-16 w-64 rounded-md p-4 px-6 border-white flex-col gap-2"
			Text="Buy"
		/>
	);
}
```

Later tokens win for each target. `size-16 w-64` sets width to 256 px and height to 64 px with the starter theme. `p-4 px-6` sets top and bottom padding to 16 px and left and right padding to 24 px. Explicit JSX properties beat classes in either attribute order, including reactive props. An explicit `Size` owns both axes. A width or height class alone preserves the instance's other axis.

The value accepts a string literal, a template without substitutions, or a const name or readonly property of a const whose type is one string literal. Calls, getters, mutable names, widened strings, unions, and reactive class strings fail with CRU029. Components cannot accept a `className` attribute yet. Their intrinsic elements can use fixed classes.

In the table, `c` names a theme color, `n` is a nonnegative number, and `a` is a whole number from 0 to 100. Spacing classes multiply `n` by `spacing`. The result must be whole pixels. Size fractions use Roblox scale. Fixed offsets stay in pixels. Use a `UIScale` for screen scaling.

| Classes | Properties or modifier children |
| --- | --- |
| `bg-<c>[/a]`, `bg-transparent` | BackgroundColor3 and BackgroundTransparency. Transparent sets only transparency to 1. |
| `text-<c>[/a]` | TextColor3 and TextTransparency. |
| `text-<[text] key>` | TextSize in pixels. |
| `text-left`, `text-center`, `text-right` | TextXAlignment. |
| `w-<n>`, `h-<n>`, `size-<n>` | Width, height, or both in spacing steps. |
| `w-px`, `h-px`, `size-px` | One pixel. |
| `w-full`, `h-full`, `size-full` | Scale 1. |
| `w-<a>/<b>`, `h-<a>/<b>`, `size-<a>/<b>` | Scale `a / b`, with a positive denominator. |
| `opacity-<0..100>` | GroupTransparency on CanvasGroup only. |
| `rounded`, `rounded-<[radius] key>`, `rounded-full` | One UICorner. Bare rounded uses DEFAULT. Full uses `UDim.new(1, 0)`. |
| `p-<n>`, `px-<n>`, `py-<n>`, `pt-<n>`, `pr-<n>`, `pb-<n>`, `pl-<n>` | One UIPadding, with all, horizontal, vertical, top, right, bottom, or left padding. |
| `border`, `border-<n>`, `border-<c>[/a]` | One UIStroke with ApplyStrokeMode Border. Width is pixels, independent of spacing. A color alone makes a 1 px stroke. |
| `flex`, `flex-col`, `gap-<n>` | One UIListLayout with SortOrder LayoutOrder. Flex is horizontal, flex-col is vertical, and gap alone creates a horizontal list. |

Each color class also sets its transparency to 0, or `(100 - a) / 100` for a suffix. `bg-transparent bg-brand` therefore shows the opaque brand color. Every modifier is configured before it is parented, before authored children. A direct authored child of the same modifier kind fails with CRU031. Opaque runtime children are not inspected.

Without a theme, keywords such as `flex`, `border`, and `w-full` still work. Theme-backed tokens fail with CRU028 and a note that no theme file is beside the tsconfig. Variants, breakpoints, component forwarding, font and alignment helpers, arbitrary values, AutomaticSize, constraints, gradients, position, and anchor classes are deferred.

| Diagnostic | Meaning and location |
| --- | --- |
| CRU028 | Unknown token, unsupported variant, fractional pixel offset, or invalid alpha. Reports at the token, with a nearby-name suggestion within edit distance 2. Indirect values report at their use and name the token and its byte offset. |
| CRU029 | Unsupported className value or component attribute. Reports at the value, or the attribute name when bare. TypeScript can reject a wrong-kind value first. |
| CRU030 | Unreadable, malformed, invalid, or conflicting theme. Reports in theme.toml at the key or value, with line and column. |
| CRU031 | A class does not fit the element, or duplicates a direct modifier child. Reports at the token or the child's tag. |

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
