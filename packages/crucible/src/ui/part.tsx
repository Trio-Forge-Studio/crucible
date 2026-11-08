import Vide, { context } from "@rbxts/vide";

const RunService = game.GetService("RunService");

export type HostProps<T extends Instance> = Vide.InstanceAttributes<T>;
// With asChild the part renders nothing of its own, so host properties are a type error rather than silently dropped.
export type AsChildProps<T extends Instance, Children = Vide.Node> =
	| { asChild: true; children: Children }
	| ({ asChild?: false } & Omit<HostProps<T>, "children"> & { children?: Children });

export interface PartContext<T> {
	provide: <R extends Vide.Node | void>(value: T, run: () => R) => R;
	use: (part: string) => T;
}
export function partContext<T extends defined>(root: string): PartContext<T> {
	const value = context<T | undefined>(undefined);
	return {
		provide: (given, run) => value(given, run),
		use: (part) => {
			const found = value();
			if (found === undefined) error(`${part} must be used inside ${root}`, 3);
			return found;
		},
	};
}

export function chain<T>(first: (instance: T) => void, second?: (instance: T) => void): (instance: T) => void {
	if (!second) return first;
	return (instance) => {
		first(instance);
		second(instance);
	};
}

// roblox-ts has no object rest, so parts copy their props without the keys they own before spreading them on the host.
export function hostProps<P extends object, K extends keyof P>(
	props: P,
	own: readonly K[],
): Pick<P, Exclude<keyof P, K | "children" | "action" | "asChild">> {
	const skip = new Set<unknown>(["children", "action", "asChild", ...own]);
	const result: Record<string, unknown> = {};
	for (const [key, value] of pairs(props as Record<string, unknown>)) if (!skip.has(key)) result[key] = value;
	return result as never;
}

const reserved = [
	"NextSelectionLeft",
	"NextSelectionRight",
	"NextSelectionUp",
	"NextSelectionDown",
	"SelectionImageObject",
] as const;

function collect(node: unknown, into: Instance[], part: string): void {
	if (node === undefined) return;
	if (typeIs(node, "userdata")) into.push(node as Instance);
	else if (typeIs(node, "table"))
		for (const [, child] of pairs(node as Record<string, unknown>)) collect(child, into, part);
	else error(`${part} asChild needs instances, not a ${typeOf(node)}`, 4);
}

export function slot<T extends GuiObject>(
	children: Vide.Node,
	part: string,
	className: "GuiObject" | "GuiButton" | "TextBox",
	apply: (instance: T) => void,
	control = false,
): T {
	const nodes: Instance[] = [];
	collect(children, nodes, part);
	const hosts = nodes.filter((node) => node.IsA("GuiObject"));
	if (hosts.size() !== 1) error(`${part} asChild needs exactly one GuiObject child, got ${hosts.size()}`, 3);
	const host = hosts[0] as GuiObject;
	if (!host.IsA(className)) error(`${part} asChild needs a ${className}, got a ${(host as Instance).ClassName}`, 3);
	for (const node of nodes) {
		if (node === host) continue;
		if (!node.IsA("UIComponent"))
			error(`${part} asChild only moves UI modifiers beside its child, got a ${node.ClassName}`, 3);
		node.Parent = host;
	}
	if (control && RunService.IsStudio())
		for (const property of reserved)
			if (host[property] !== undefined)
				warn(`${part} owns ${property}; the value on its asChild child is replaced`);
	apply(host as T);
	return host as T;
}

// A function child receives the part's binding, so styled children can read its state, with or without asChild.
export type BindingChildren<B> = Vide.Node | ((binding: B) => Vide.Node);
export type ButtonPartProps<B> = AsChildProps<TextButton, BindingChildren<B>>;

export function buttonHost<B>(
	props: ButtonPartProps<B>,
	part: string,
	own: readonly string[],
	apply: (instance: GuiButton) => void,
	binding: B,
): Vide.Node {
	const given = props.children;
	const children = typeIs(given, "function") ? (given as (binding: B) => Vide.Node)(binding) : given;
	if (props.asChild) return slot(children, part, "GuiButton", apply, true);
	return (
		<textbutton
			Text=""
			AutoButtonColor={false}
			BackgroundTransparency={1}
			BorderSizePixel={0}
			{...hostProps(props as ButtonPartProps<B> & Record<string, unknown>, own)}
			action={chain(apply, props.action)}
		>
			{children}
		</textbutton>
	);
}
export function frameHost(
	props: AsChildProps<Frame, unknown>,
	part: string,
	own: readonly string[],
	children: () => Vide.Node,
	apply: (instance: GuiObject) => void = () => {},
	defaults: HostProps<Frame> = {},
): Vide.Node {
	if (props.asChild) return slot(children(), part, "GuiObject", apply);
	return (
		<frame
			BackgroundTransparency={1}
			BorderSizePixel={0}
			{...defaults}
			{...hostProps(props as AsChildProps<Frame, unknown> & Record<string, unknown>, own)}
			action={chain(apply, props.action)}
		>
			{children()}
		</frame>
	);
}
