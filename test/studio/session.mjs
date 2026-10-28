import { randomUUID } from "node:crypto";

export function createPlaceName(label) {
	return `s2-${label}-${process.pid}-${randomUUID()}.rbxl`;
}

export function selectStudio(studios, placeName) {
	const matches = studios.filter((studio) => studio.name === placeName);
	if (matches.length > 1) throw Error(`Multiple Studios match owned place ${placeName}`);
	return matches[0]?.id;
}
