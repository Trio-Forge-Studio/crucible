declare global {
	namespace JSX {
		interface Element {}
		interface IntrinsicElements {
			frame: { Size?: number };
		}
	}
}
declare const props: { Size?: number } | undefined;
declare const style: { Size?: number };
export const loose = <frame {...props} />;
export const strict = <frame {...style} />;
