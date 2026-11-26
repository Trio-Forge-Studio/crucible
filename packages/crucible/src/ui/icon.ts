import type { IconName } from "./lucide";

export type { IconName } from "./lucide";

export interface IconImage {
	Image: string;
	ImageRectOffset: Vector2;
	ImageRectSize: Vector2;
}

export function icon(_name: IconName): IconImage {
	error("icon() must be compiled by Crucible");
}
