import type { Readable } from "@trioforge/crucible";
import { For, mount } from "@trioforge/crucible/ui";

export function mountList(parent: Instance, items: Readable<readonly string[]>) {
	return mount(
		() => (
			<frame Name="List">
				<uilistlayout Name="Layout" />
				<For each={items}>{(item, index) => <textlabel Name="Row" Text={item} LayoutOrder={index} />}</For>
			</frame>
		),
		parent,
	);
}
