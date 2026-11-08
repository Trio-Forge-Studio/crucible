declare const count: number;
declare const text: string | undefined;
declare const part: Instance;
declare const flag: boolean | undefined;
if (count) print("count");
if (text) print("text");
if (part) print("part");
export const fallback = flag ?? true;
