const tag = (strings: TemplateStringsArray, ...values: number[]) => strings.size() + values.size();
export const tagged = tag`a${1}b`;
const tagger = {
	mark(strings: TemplateStringsArray) {
		return strings.size();
	},
};
export const marked = tagger.mark`x`;
