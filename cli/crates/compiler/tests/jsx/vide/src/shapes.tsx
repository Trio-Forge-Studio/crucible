/** @jsx h */
/** @jsxFrag Group */

declare function h(tag: unknown, props: unknown, ...children: unknown[]): unknown;
declare namespace h {
	namespace JSX {
		type Element = unknown;
		interface IntrinsicElements {
			frame: { Name?: string; LayoutOrder?: number; Visible?: boolean; children?: unknown };
		}
	}
}
export declare const Group: unknown;
declare const maybe: { Name: string } | undefined;
declare const rows: unknown[];
declare function count(): number;

export const text = (
	<frame>
		Coins &amp; gems &#33; left over from the last delivery, which the formatter keeps on several lines because the
		text is longer than one line
	</frame>
);
export const spaced = <frame> a </frame>;
export const spreadAfter = <frame Name="Last" {...maybe} Visible />;
export const spreadChildren = <frame>{...rows}</frame>;
export const ordered = (
	<frame LayoutOrder={count()}>
		{count()}
		{count()}
	</frame>
);
export const empty = <></>;
export const grouped = <>{count()}</>;
