import type { Readable } from "@trioforge/crucible";
import { mount, Show } from "@trioforge/crucible/ui";

export function mountPanel(parent: Instance, open: Readable<boolean>, title: Readable<string>) {
	return mount(
		() => (
			<frame Name="Panel">
				<Show when={open} fallback={() => <textlabel Name="Closed" Text="closed" />}>
					{() => <textlabel Name="Open" Text={title} />}
				</Show>
			</frame>
		),
		parent,
	);
}
