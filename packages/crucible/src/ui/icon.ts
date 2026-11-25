import { lucideNames, lucideSheets } from "./lucide";

export type IconName = (typeof lucideNames)[number];

export interface IconImage {
	Image: string;
	ImageRectOffset: Vector2;
	ImageRectSize: Vector2;
}

const indices = new Map<IconName, number>();
for (let index = 0; index < lucideNames.size(); index++) {
	indices.set(lucideNames[index], index);
}

const columns = 16;
const iconsPerSheet = columns * columns;
const cell = 64;
const gutter = 2;
const rectSize = new Vector2(cell - 2 * gutter, cell - 2 * gutter);

export function icon(name: IconName): IconImage {
	const index = indices.get(name);
	if (index === undefined) error(`Unknown Lucide icon: ${name}`);
	const slot = index % iconsPerSheet;
	return {
		Image: lucideSheets[math.floor(index / iconsPerSheet)],
		ImageRectOffset: new Vector2((slot % columns) * cell + gutter, math.floor(slot / columns) * cell + gutter),
		ImageRectSize: rectSize,
	};
}
