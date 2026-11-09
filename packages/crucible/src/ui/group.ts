import { cleanup, effect, read, source, untrack } from "@rbxts/vide";
import type { Value } from "./props";
import { alive, selectedObject } from "./runtime";

export type Orientation = "Horizontal" | "Vertical";
export interface GroupOptions {
	orientation?: Value<Orientation>;
	loop?: Value<boolean>;
}
export interface Group {
	item: (object: GuiObject, disabled?: () => boolean) => void;
}
interface Item {
	readonly object: GuiObject;
	readonly disabled: () => boolean;
}
// Roblox's own navigation moves selection; the group only points the selected item's NextSelection links along its axis.
export function createGroup(options: GroupOptions = {}): Group {
	const items: Item[] = [];
	const version = source(0);
	const step = (from: number, direction: 1 | -1): GuiObject | undefined => {
		const count = items.size();
		const loop = read(options.loop ?? false);
		for (let offset = 1; offset < count; offset++) {
			let index = from + direction * offset;
			if (loop) index = (index + count) % count;
			else if (index < 0 || index >= count) return undefined;
			const item = items[index];
			if (item.object.Selectable && alive(item.object) && !item.disabled()) return item.object;
		}
		return undefined;
	};
	return {
		item: (object, disabled = () => false) => {
			const entry = { object, disabled };
			const changed = () => version(untrack(version) + 1);
			items.push(entry);
			changed();
			cleanup(() => {
				const index = items.indexOf(entry);
				if (index >= 0) items.remove(index);
				changed();
			});
			effect(() => {
				version();
				if (selectedObject() !== object) return;
				const index = items.indexOf(entry);
				const previous = step(index, -1) ?? object;
				const following = step(index, 1) ?? object;
				if (read(options.orientation ?? "Horizontal") === "Horizontal") {
					object.NextSelectionLeft = previous;
					object.NextSelectionRight = following;
				} else {
					object.NextSelectionUp = previous;
					object.NextSelectionDown = following;
				}
			});
		},
	};
}
