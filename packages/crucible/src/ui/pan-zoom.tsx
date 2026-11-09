import Vide from "@rbxts/vide";
import { frameHost, partContext, type AsChildProps } from "./part";
import { usePanZoom, type PanZoomBinding, type PanZoomOptions } from "./use-pan-zoom";

export interface PanZoomRootProps extends PanZoomOptions {
	children: (panZoom: PanZoomBinding) => Vide.Node;
}
export type PanZoomViewportProps = AsChildProps<Frame>;
export type PanZoomContentProps = AsChildProps<Frame>;
const context = partContext<PanZoomBinding>("PanZoom.Root");

function PanZoomRoot(props: PanZoomRootProps): Vide.Node {
	const panZoom = usePanZoom(props);
	return context.provide(panZoom, () => props.children(panZoom));
}
function PanZoomViewport(props: PanZoomViewportProps): Vide.Node {
	const panZoom = context.use("PanZoom.Viewport");
	return frameHost(props, "PanZoom.Viewport", [], () => props.children, panZoom.viewport, {
		Size: UDim2.fromScale(1, 1),
	});
}
function PanZoomContent(props: PanZoomContentProps): Vide.Node {
	const panZoom = context.use("PanZoom.Content");
	return frameHost(props, "PanZoom.Content", [], () => props.children, panZoom.content);
}

export const PanZoom = { Root: PanZoomRoot, Viewport: PanZoomViewport, Content: PanZoomContent };
