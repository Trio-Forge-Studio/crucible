export function validateMaxLength(maxLength?: number): void {
	assert(
		maxLength === undefined || (maxLength >= 0 && maxLength < math.huge && maxLength % 1 === 0),
		"Text field maxLength must be a finite nonnegative integer",
	);
}
export function limitText(text: string, maxLength?: number): string {
	return maxLength === undefined ? text : text.sub(1, maxLength);
}
export function completionText(text: string, completion?: string): string | undefined {
	return completion === undefined ? undefined : text + completion.sub(text.size() + 1);
}
