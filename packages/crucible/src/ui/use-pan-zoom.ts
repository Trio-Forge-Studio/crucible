import { batch, cleanup, derive, effect, source, untrack } from "@rbxts/vide";
import { Attribute } from "./attributes";
import { heartbeat } from "./frame-loop";
import {
	containFit,
	coverPan,
	glideStep,
	revealPan,
	stickVector,
	zoomAround,
	type PanZoomInset,
	type PanZoomView,
} from "./pan-zoom-state";
import type { Value } from "./props";
import { active, alive, menuOpen, ownerContext, selectedObject, textEditing } from "./runtime";
import { useControllable } from "./use-controllable";

const GuiService = game.GetService("GuiService");
const UserInputService = game.GetService("UserInputService");

export interface PanZoomOptions {
	contentSize: Vector2;
	view?: Value<PanZoomView>;
	defaultView?: PanZoomView;
	onViewChange?: (view: PanZoomView) => void;
	minZoom?: number;
	maxZoom?: number;
	fit?: (viewport: Vector2) => number;
	clampPan?: (pan: Vector2, scale: number, half: Vector2) => Vector2;
	followSelection?: boolean | { inset: PanZoomInset };
	gamepad?: { pan?: boolean; zoom?: boolean };
}
export interface PanZoomBinding {
	view: () => PanZoomView;
	fit: () => number;
	scale: () => number;
	half: () => Vector2;
	dragging: () => boolean;
	panTo: (point: Vector2, options?: { glide?: boolean }) => void;
	zoomTo: (zoom: number, anchor?: Vector2) => void;
	viewport: (instance: GuiObject) => void;
	content: (instance: GuiObject) => void;
}
type Gesture =
	| {
			readonly kind: "Drag";
			readonly input: InputObject;
			readonly mouse: boolean;
			readonly from: Vector3;
			readonly pan: Vector2;
	  }
	| { readonly kind: "Pinch"; readonly zoom: number };

const wheelStep = 1.15;
const probeWidth = 100;
const defaultInset: PanZoomInset = { topLeft: new Vector2(100, 100), bottomRight: new Vector2(100, 100) };
const stickDeadzone = 0.2;
const stickSpeed = 900;
const triggerZoomPerSecond = 2;
const zoomKeys = new Set<Enum.KeyCode>([Enum.KeyCode.ButtonL2, Enum.KeyCode.ButtonR2]);

function reaches(viewport: GuiObject, content: GuiObject | undefined, point: Vector2): boolean {
	// A hidden viewport is in no hit list, so the empty list below would let it through.
	if (!alive(viewport)) return false;
	const position = viewport.AbsolutePosition;
	const corner = position.add(viewport.AbsoluteSize);
	if (point.X < position.X || point.Y < position.Y || point.X > corner.X || point.Y > corner.Y) return false;
	const gui = viewport.FindFirstAncestorOfClass("PlayerGui");
	for (const object of gui?.GetGuiObjectsAtPosition(point.X, point.Y) ?? []) {
		if (object.Active || object.BackgroundTransparency < 1)
			return (
				object === viewport || (content !== undefined && (object === content || object.IsDescendantOf(content)))
			);
	}
	return true;
}

export function usePanZoom(options: PanZoomOptions): PanZoomBinding {
	const owner = ownerContext();
	const contentSize = options.contentSize;
	const minZoom = options.minZoom ?? 0.5;
	const maxZoom = options.maxZoom ?? 1.6;
	assert(contentSize.X > 0 && contentSize.Y > 0, "PanZoom contentSize must be positive");
	assert(minZoom > 0 && minZoom <= maxZoom, "PanZoom zoom bounds must be positive and ordered");
	const clampPan =
		options.clampPan ?? ((pan: Vector2, scale: number, half: Vector2) => coverPan(pan, contentSize, scale, half));
	const follow = options.followSelection ?? true;
	const safeArea = follow === false ? undefined : follow === true ? defaultInset : follow.inset;
	const initial = options.defaultView ?? { pan: Vector2.zero, zoom: 1 };
	const initialZoom = math.clamp(initial.zoom, minZoom, maxZoom);
	const [view, write] = useControllable<PanZoomView>({
		value: options.view,
		defaultValue: { pan: clampPan(initial.pan, initialZoom, Vector2.zero), zoom: initialZoom },
		onChange: options.onViewChange,
	});
	const fit = source(1);
	const half = source(Vector2.zero);
	const measured = source(false);
	const dragging = source(false);
	const glide = source<Vector2 | undefined>(undefined);
	const steering = source(false);
	const content = source<GuiObject | undefined>(undefined);
	const scale = derive(() => fit() * view().zoom);
	const live = derive(() => active(owner) && !menuOpen());
	let viewport: GuiObject | undefined;
	let probe: GuiObject | undefined;
	// Screen pixels per logical pixel. The ScaleProbe measures it, so any UIScale above the viewport counts.
	let ratio = 1;
	let gesture: Gesture | undefined;
	let stick = Vector2.zero;
	const triggers = new Set<Enum.KeyCode>();

	const commit = (pan: Vector2, zoom: number) =>
		untrack(() => {
			const clamped = math.clamp(zoom, minZoom, maxZoom);
			const wanted = { pan: clampPan(pan, fit() * clamped, half()), zoom: clamped };
			const current = view();
			if (wanted.pan !== current.pan || wanted.zoom !== current.zoom) write(wanted);
		});
	const glideTo = (target: Vector2) => {
		if (!GuiService.ReducedMotionEnabled) return glide(target);
		glide(undefined);
		commit(target, view().zoom);
	};
	const zoomAt = (zoom: number, anchor: Vector2) =>
		untrack(() => {
			const zoomed = zoomAround(view(), math.clamp(zoom, minZoom, maxZoom), anchor, fit());
			commit(zoomed.pan, zoomed.zoom);
		});
	const fromCentre = (point: Vector2) =>
		viewport ? point.sub(viewport.AbsolutePosition.add(viewport.AbsoluteSize.div(2))).div(ratio) : Vector2.zero;
	const offsetOf = (point: Vector2) => untrack(() => point.sub(contentSize.div(2)).add(view().pan).mul(scale()));
	const measure = () =>
		untrack(() => {
			if (!viewport || !probe) return;
			const measuredRatio = probe.AbsoluteSize.X / probeWidth;
			if (!(measuredRatio > 0)) return;
			const size = viewport.AbsoluteSize.div(measuredRatio);
			if (size.X <= 0 || size.Y <= 0) return;
			ratio = measuredRatio;
			batch(() => {
				fit(options.fit ? options.fit(size) : containFit(size, contentSize));
				half(size.div(2));
				measured(true);
				const current = view();
				commit(current.pan, current.zoom);
			});
		});
	const endGesture = () => {
		gesture = undefined;
		dragging(false);
	};
	const release = () => {
		stick = Vector2.zero;
		triggers.clear();
		steering(false);
	};
	const steer = () => steering(stick.Magnitude > stickDeadzone || !triggers.isEmpty());

	const begin = (input: InputObject) =>
		untrack(() => {
			const mouse = input.UserInputType === Enum.UserInputType.MouseButton1;
			if ((!mouse && input.UserInputType !== Enum.UserInputType.Touch) || gesture || !live()) return;
			gesture = { kind: "Drag", input, mouse, from: input.Position, pan: view().pan };
			glide(undefined);
			dragging(true);
		});
	const moved = (input: InputObject) => {
		if (input.UserInputType === Enum.UserInputType.MouseWheel) {
			const point = new Vector2(input.Position.X, input.Position.Y);
			if (!viewport || !reaches(viewport, untrack(content), point)) return;
			glide(undefined);
			zoomAt(view().zoom * wheelStep ** input.Position.Z, fromCentre(point));
			return;
		}
		if (gesture?.kind !== "Drag") return;
		if (gesture.mouse ? input.UserInputType !== Enum.UserInputType.MouseMovement : input !== gesture.input) return;
		if (!viewport || !alive(viewport)) return endGesture();
		const delta = input.Position.sub(gesture.from);
		glide(undefined);
		commit(gesture.pan.add(new Vector2(delta.X, delta.Y).div(ratio * scale())), view().zoom);
	};
	const pinched = (positions: Vector2[], factor: number, state: Enum.UserInputState) => {
		const middle = positions.size() >= 2 ? positions[0].add(positions[1]).div(2) : undefined;
		if (state === Enum.UserInputState.Begin) {
			if (!middle || !viewport || !reaches(viewport, untrack(content), middle)) return;
			gesture = { kind: "Pinch", zoom: view().zoom };
			dragging(false);
			glide(undefined);
		} else if (gesture?.kind !== "Pinch") return;
		else if (state !== Enum.UserInputState.Change || !viewport || !alive(viewport)) endGesture();
		else if (middle) zoomAt(gesture.zoom * factor, fromCentre(middle));
	};

	effect(() => {
		if (!live()) return;
		cleanup(UserInputService.InputChanged.Connect((input) => untrack(() => moved(input))));
		cleanup(
			UserInputService.InputEnded.Connect((input) => {
				if (gesture?.kind === "Drag" && input === gesture.input) untrack(endGesture);
			}),
		);
		cleanup(
			UserInputService.TouchPinch.Connect((positions, factor, _, state) =>
				untrack(() => pinched(positions, factor, state)),
			),
		);
		cleanup(UserInputService.WindowFocusReleased.Connect(() => untrack(endGesture)));
		cleanup(() => {
			endGesture();
			glide(undefined);
		});
	});

	const pads = options.gamepad;
	if (pads?.pan || pads?.zoom)
		effect(() => {
			if (!live() || textEditing()) return;
			if (pads.pan)
				cleanup(
					UserInputService.InputChanged.Connect((input) => {
						if (input.KeyCode !== Enum.KeyCode.Thumbstick2) return;
						stick = new Vector2(input.Position.X, input.Position.Y);
						untrack(steer);
					}),
				);
			if (pads.zoom)
				cleanup(
					UserInputService.InputBegan.Connect((input) => {
						if (!zoomKeys.has(input.KeyCode)) return;
						triggers.add(input.KeyCode);
						untrack(steer);
					}),
				);
			cleanup(
				UserInputService.InputEnded.Connect((input) => {
					if (input.KeyCode === Enum.KeyCode.Thumbstick2) stick = Vector2.zero;
					else if (!triggers.delete(input.KeyCode)) return;
					untrack(steer);
				}),
			);
			cleanup(UserInputService.GamepadDisconnected.Connect(() => untrack(release)));
			cleanup(UserInputService.WindowFocusReleased.Connect(() => untrack(release)));
			cleanup(release);
		});

	const step = (delta: number) => {
		if (steering()) {
			if (!viewport || !alive(viewport)) return;
			glide(undefined);
			const current = view();
			const push = stickVector(stick, stickDeadzone).mul((stickSpeed * delta) / scale());
			const direction =
				(triggers.has(Enum.KeyCode.ButtonR2) ? 1 : 0) - (triggers.has(Enum.KeyCode.ButtonL2) ? 1 : 0);
			commit(
				current.pan.add(new Vector2(-push.X, push.Y)),
				current.zoom * triggerZoomPerSecond ** (direction * delta),
			);
			return;
		}
		const target = glide();
		if (target === undefined) return;
		const pan = glideStep(view().pan, target, delta);
		commit(pan, view().zoom);
		if (pan === target) glide(undefined);
	};
	effect(() => {
		if (!live() || (glide() === undefined && !steering())) return;
		cleanup(heartbeat((_, delta) => untrack(() => step(delta))));
	});

	if (safeArea)
		effect(() => {
			const selected = selectedObject();
			const box = content();
			if (!selected || !box || !live() || !measured()) return;
			const area = half();
			if (!selected.IsDescendantOf(box)) return;
			untrack(() => {
				const size = box.AbsoluteSize;
				if (size.X <= 0 || size.Y <= 0) return;
				const centre = selected.AbsolutePosition.add(selected.AbsoluteSize.div(2));
				const point = centre.sub(box.AbsolutePosition).div(size).mul(contentSize);
				const target = revealPan(view().pan, offsetOf(point), scale(), area, safeArea);
				if (target !== undefined) glideTo(clampPan(target, scale(), area));
			});
		});

	return {
		view,
		fit: () => fit(),
		scale,
		half: () => half(),
		dragging: () => dragging(),
		panTo: (point, panOptions) =>
			untrack(() => {
				const target = clampPan(contentSize.div(2).sub(point), scale(), half());
				if (panOptions?.glide) glideTo(target);
				else {
					glide(undefined);
					commit(target, view().zoom);
				}
			}),
		zoomTo: (zoom, anchor) =>
			untrack(() => {
				glide(undefined);
				zoomAt(zoom, anchor ? offsetOf(anchor) : Vector2.zero);
			}),
		viewport: (instance) => {
			viewport = instance;
			instance.Active = true;
			const meter = new Instance("Frame");
			meter.Name = "ScaleProbe";
			meter.BackgroundTransparency = 1;
			meter.Size = UDim2.fromOffset(probeWidth, 0);
			meter.Parent = instance;
			probe = meter;
			effect(() => instance.SetAttribute(Attribute.Dragging, dragging()));
			cleanup(instance.InputBegan.Connect(begin));
			cleanup(instance.GetPropertyChangedSignal("AbsoluteSize").Connect(measure));
			cleanup(meter.GetPropertyChangedSignal("AbsoluteSize").Connect(measure));
			cleanup(() => {
				meter.Destroy();
				if (viewport !== instance) return;
				viewport = undefined;
				probe = undefined;
			});
			measure();
		},
		content: (instance) => {
			content(instance);
			instance.AnchorPoint = new Vector2(0.5, 0.5);
			instance.Size = UDim2.fromOffset(contentSize.X, contentSize.Y);
			const scaler = new Instance("UIScale");
			effect(() => (scaler.Scale = scale()));
			scaler.Parent = instance;
			effect(() => {
				const offset = view().pan.mul(scale());
				instance.Position = new UDim2(0.5, math.round(offset.X), 0.5, math.round(offset.Y));
			});
			effect(() => (instance.Visible = measured()));
			cleanup(() => {
				scaler.Destroy();
				if (untrack(content) === instance) content(undefined);
			});
		},
	};
}
