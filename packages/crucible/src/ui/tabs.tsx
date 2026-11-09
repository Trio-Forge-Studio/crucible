import Vide, { read, show } from "@rbxts/vide";
import { createGroup, type Group, type Orientation } from "./group";
import { buttonHost, frameHost, partContext, type AsChildProps, type ButtonPartProps } from "./part";
import type { ChildFactory, Value } from "./props";
import { useTabs, type TabsBinding, type TabsOptions, type TabsTriggerBinding } from "./use-tabs";

interface TabsContext {
	tabs: TabsBinding<unknown>;
	group: Group;
	disabled?: Value<boolean>;
}
const context = partContext<TabsContext>("Tabs.Root");

export interface TabsRootProps<T> extends TabsOptions<T> {
	orientation?: Value<Orientation>;
	loop?: Value<boolean>;
	children: (tabs: TabsBinding<T>) => Vide.Node;
}
function TabsRoot<T>(props: TabsRootProps<T>): Vide.Node {
	const tabs = useTabs<T>(props);
	const group = createGroup({ orientation: props.orientation, loop: props.loop ?? true });
	return context.provide({ tabs: tabs as TabsBinding<unknown>, group, disabled: props.disabled }, () =>
		props.children(tabs),
	);
}

export type TabsListProps = AsChildProps<Frame>;
function TabsList(props: TabsListProps): Vide.Node {
	context.use("Tabs.List");
	return frameHost(props, "Tabs.List", [], () => props.children);
}

export type TabsTriggerProps = {
	value: unknown;
	disabled?: Value<boolean>;
	selectFirst?: Value<boolean>;
} & ButtonPartProps<TabsTriggerBinding>;
function TabsTrigger(props: TabsTriggerProps): Vide.Node {
	const { tabs, group, disabled } = context.use("Tabs.Trigger");
	const trigger = tabs.trigger(props.value, { disabled: props.disabled, selectFirst: props.selectFirst });
	const apply = (instance: GuiButton) => {
		trigger.action(instance);
		group.item(instance, () => read(disabled ?? false) || read(props.disabled ?? false));
	};
	return buttonHost(props, "Tabs.Trigger", ["value", "disabled", "selectFirst"], apply, trigger);
}

export type TabsContentProps = { value: unknown } & AsChildProps<Frame, ChildFactory> & { children: ChildFactory };
function TabsContent(props: TabsContentProps): Vide.Node {
	const { tabs } = context.use("Tabs.Content");
	return show(
		() => tabs.value() === props.value,
		() => frameHost(props, "Tabs.Content", ["value"], () => props.children()),
	);
}

export const Tabs = { Root: TabsRoot, List: TabsList, Trigger: TabsTrigger, Content: TabsContent };
