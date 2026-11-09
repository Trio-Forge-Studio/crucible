import { cleanup, derive, effect, source, untrack } from "@rbxts/vide";
import { adapter } from "./adapter";
import { Attribute } from "./attributes";
import { heartbeat } from "./frame-loop";
import { menuOpen, selectedObject } from "./runtime";
import {
	glidePose,
	placementOf,
	stackBounds,
	stackTargets,
	swipeDismisses,
	swipeExit,
	swipeOffset,
	swipeProgress,
	swipeSlop,
	swipeVelocity,
	type SwipeSample,
	type ToastLayout,
	type ToastPlacement,
	type ToastPose,
	type ToastTarget,
} from "./toast-layout";
import { findToast, needsTick, type ToastEntry, type ToastId } from "./toast-state";
import { toasterCore, type Toaster } from "./toaster";

const GuiService = game.GetService("GuiService");
const UserInputService = game.GetService("UserInputService");

export interface ToastBinding<D> {
	readonly id: ToastId;
	readonly data: () => D;
	readonly closing: () => boolean;
	readonly depth: () => number;
	readonly expanded: () => boolean;
	readonly highlighted: () => boolean;
	readonly swipe: () => number;
	readonly collapsedHeight: () => number | undefined;
	readonly hasAction: () => boolean;
	readonly act: () => boolean;
	readonly close: () => void;
}
export interface ToastViewportOptions<D> {
	toaster: Toaster<D>;
	placement?: ToastPlacement;
	layout?: ToastLayout;
	gap?: number;
	peek?: number;
}
export interface ToastSlot<D> {
	readonly binding: ToastBinding<D>;
	readonly inert: () => boolean;
	readonly root: (host: GuiObject, holdExit?: () => boolean) => void;
}
interface Gesture {
	readonly input: InputObject;
	readonly from: Vector2;
	claimed: boolean;
	samples: SwipeSample[];
}

const probeWidth = 100;

function put<V>(map: ReadonlyMap<ToastId, V>, id: ToastId, value: V | undefined): ReadonlyMap<ToastId, V> {
	const copy = new Map<ToastId, V>();
	for (const [key, existing] of map) if (key !== id) copy.set(key, existing);
	if (value !== undefined) copy.set(id, value);
	return copy;
}
function point(input: InputObject): Vector2 {
	return new Vector2(input.Position.X, input.Position.Y);
}
function touch(input: InputObject): boolean {
	return input.UserInputType === Enum.UserInputType.Touch;
}

export function useToastViewport<D>(options: ToastViewportOptions<D>) {
	const core = toasterCore(options.toaster);
	const placement = placementOf(options.placement ?? "TopRight");
	const layout = options.layout ?? "List";
	const gap = options.gap ?? 10;
	const peek = options.peek ?? 12;
	const device = adapter().device;
	cleanup(core.attach());

	const sizes = source<ReadonlyMap<ToastId, Vector2>>(new Map());
	const hosts = source<ReadonlyMap<ToastId, GuiObject>>(new Map());
	const stackHovered = source(false);
	const touchExpanded = source(false);
	const swiping = source<ToastId | undefined>(undefined);
	const blocked = source(false);
	const focused = source(true);
	const measures = new Set<() => void>();
	let frame: GuiObject | undefined;
	let ratio = 1;
	let unblock: (() => void) | undefined;

	const mounted = derive(() => core.state().entries.filter((entry) => entry.phase.kind !== "Queued"));
	const ids = derive(() => mounted().map((entry) => entry.id));
	const newestFirst = derive(() => {
		const open: ToastId[] = [];
		const entries = mounted();
		for (let index = entries.size() - 1; index >= 0; index--)
			if (entries[index].phase.kind === "Open") open.push(entries[index].id);
		return open;
	});
	const hovering = derive(() => stackHovered() && !newestFirst().isEmpty());
	const selectionInside = derive(() => {
		const selected = selectedObject();
		const roots = hosts();
		return (
			selected !== undefined &&
			newestFirst().some((id) => {
				const host = roots.get(id);
				return host !== undefined && selected.IsDescendantOf(host);
			})
		);
	});
	const expanded = derive(
		() => layout === "List" || hovering() || selectionInside() || touchExpanded() || swiping() !== undefined,
	);
	// On touch the first tap on a collapsed deck only expands it, so it must not reach the front toast's buttons.
	const tapToExpand = derive(() => !expanded() && device() === "Touch" && newestFirst().size() > 1);
	const stack = derive(() => {
		const order = newestFirst();
		const measured = sizes();
		const natural = order.map((id) => measured.get(id) ?? Vector2.zero);
		const placed = stackTargets(natural, { layout, expanded: expanded(), gap, peek, placement });
		const byId = new Map<ToastId, ToastTarget>();
		order.forEach((id, index) => byId.set(id, placed[index]));
		return { byId, bounds: stackBounds(natural, placed, placement) };
	});
	const pausing = derive(
		() =>
			hovering() ||
			selectionInside() ||
			swiping() !== undefined ||
			(layout === "Deck" && expanded()) ||
			!focused() ||
			menuOpen(),
	);
	effect(() => {
		if (pausing()) cleanup(options.toaster.pause());
	});
	const ticking = derive(() => needsTick(core.state()));
	effect(() => {
		if (ticking()) cleanup(heartbeat((now) => core.tick(now)));
	});
	effect(() => {
		if (newestFirst().size() < 2) touchExpanded(false);
	});

	// Roblox fires Activated as a touch lifts, so a tap or swipe keeps every toast inert until the next frame.
	const block = (input: InputObject) => {
		unblock?.();
		blocked(true);
		let later: (() => void) | undefined;
		const ended = UserInputService.InputEnded.Connect((ending) => {
			if (ending === input) later ??= heartbeat(() => release());
		});
		const release = () => {
			ended.Disconnect();
			later?.();
			unblock = undefined;
			blocked(false);
		};
		unblock = release;
	};
	const inStack = (screen: Vector2) => {
		const area = untrack(stack).bounds;
		if (!frame || !area) return false;
		const at = screen.sub(frame.AbsolutePosition.add(frame.AbsoluteSize.mul(placement.anchor))).div(ratio);
		return at.X >= area.min.X && at.Y >= area.min.Y && at.X <= area.max.X && at.Y <= area.max.Y;
	};
	cleanup(
		UserInputService.InputBegan.Connect((input) =>
			untrack(() => {
				if (!touch(input)) return;
				if (!inStack(point(input))) touchExpanded(false);
				else if (tapToExpand()) {
					touchExpanded(true);
					block(input);
				}
			}),
		),
	);
	cleanup(UserInputService.WindowFocusReleased.Connect(() => focused(false)));
	cleanup(UserInputService.WindowFocused.Connect(() => focused(true)));
	cleanup(() => unblock?.());

	const slot = (id: ToastId): ToastSlot<D> => {
		let latest = findToast(untrack(core.state), id) as ToastEntry<D>;
		// The keyed mount removes this slot in the same update that drops the entry, so keep the last one seen.
		const entry = derive(() => {
			latest = findToast(core.state(), id) ?? latest;
			return latest;
		});
		let resting: ToastTarget = { position: Vector2.zero, scale: 1, depth: 0, collapsedHeight: undefined };
		// A closing toast leaves the stack and keeps its last place while it exits.
		const target = derive(() => {
			resting = stack().byId.get(id) ?? resting;
			return resting;
		});
		const closing = derive(() => entry().phase.kind === "Closing");
		const host = source<GuiObject | undefined>(undefined);
		const hovered = source(false);
		const travel = source(0);
		const claimed = derive(() => swiping() === id);
		const inert = derive(() => closing() || blocked() || (!expanded() && (target().depth > 0 || tapToExpand())));
		const highlighted = derive(() => {
			const root = host();
			const selected = selectedObject();
			const inside = root !== undefined && selected !== undefined && selected.IsDescendantOf(root);
			return device() !== "Touch" && (hovered() || inside);
		});
		const rank = derive(() => mounted().findIndex((candidate) => candidate.id === id) + 1);
		const binding: ToastBinding<D> = {
			id,
			data: () => entry().data,
			closing: () => closing(),
			depth: () => target().depth,
			expanded: () => expanded(),
			highlighted: () => highlighted(),
			swipe: () => swipeProgress(travel()),
			collapsedHeight: () => target().collapsedHeight,
			hasAction: () => entry().action !== undefined,
			act: () => !untrack(inert) && core.act(id),
			close: () => {
				if (!untrack(inert)) core.dismiss(id, "Dismiss");
			},
		};

		const root = (instance: GuiObject, holdExit?: () => boolean) => {
			assert(untrack(host) === undefined, "A toast renders one Toast.Root");
			host(instance);
			hosts(put(untrack(hosts), id, instance));
			if (holdExit) cleanup(core.holdExit(id, holdExit));
			instance.AnchorPoint = placement.anchor;
			const scaler = new Instance("UIScale");
			scaler.Name = "ToastScale";
			scaler.Parent = instance;
			const desired = derive((): ToastPose => {
				const goal = target();
				const current = entry();
				let offset = placement.swipe.mul(swipeOffset(travel()));
				if (current.phase.kind === "Closing" && current.phase.reason === "Swipe")
					offset = offset.add(swipeExit(placement, untrack(sizes).get(id) ?? Vector2.zero));
				return { position: goal.position.add(offset), scale: goal.scale };
			});
			let pose = untrack(desired);
			const apply = () => {
				const { X, Y } = pose.position;
				instance.Position = new UDim2(placement.anchor.X, math.round(X), placement.anchor.Y, math.round(Y));
				scaler.Scale = pose.scale;
			};
			apply();
			effect(() => {
				const goal = desired();
				if (claimed() || GuiService.ReducedMotionEnabled) {
					pose = goal;
					apply();
					return;
				}
				if (pose.position === goal.position && pose.scale === goal.scale) return;
				const stop = heartbeat((_, delta) => {
					pose = glidePose(pose, goal, delta);
					apply();
					if (pose.position === goal.position && pose.scale === goal.scale) stop();
				});
				cleanup(stop);
			});

			// The deck shrink would distort the natural size, so only an unscaled toast is measured.
			const measure = () => {
				if (scaler.Scale !== 1) return;
				const size = instance.AbsoluteSize.div(ratio);
				if (untrack(sizes).get(id) !== size) sizes(put(untrack(sizes), id, size));
			};
			measures.add(measure);
			cleanup(instance.GetPropertyChangedSignal("AbsoluteSize").Connect(measure));
			measure();

			effect(() => (instance.ZIndex = rank()));
			effect(() => (instance.Interactable = !inert()));
			effect(() => instance.SetAttribute(Attribute.Closing, closing()));
			effect(() => instance.SetAttribute(Attribute.Expanded, expanded()));
			effect(() => instance.SetAttribute(Attribute.Depth, target().depth));
			effect(() => instance.SetAttribute(Attribute.Highlighted, highlighted()));
			effect(() => instance.SetAttribute(Attribute.Swiping, claimed()));
			cleanup(instance.MouseEnter.Connect(() => hovered(true)));
			cleanup(instance.MouseLeave.Connect(() => hovered(false)));

			let gesture: Gesture | undefined;
			const cancel = () => {
				if (gesture?.claimed) {
					swiping(undefined);
					travel(0);
				}
				gesture = undefined;
			};
			cleanup(
				UserInputService.InputBegan.Connect((input) =>
					untrack(() => {
						if (!touch(input) || gesture || swiping() !== undefined || inert()) return;
						const at = point(input);
						const low = instance.AbsolutePosition;
						const high = low.add(instance.AbsoluteSize);
						if (at.X < low.X || at.Y < low.Y || at.X > high.X || at.Y > high.Y) return;
						gesture = { input, from: at, claimed: false, samples: [] };
					}),
				),
			);
			cleanup(
				UserInputService.InputChanged.Connect((input) =>
					untrack(() => {
						if (!gesture || input !== gesture.input) return;
						const moved = point(input).sub(gesture.from).div(ratio);
						const along = moved.Dot(placement.swipe);
						if (!gesture.claimed) {
							if (moved.Magnitude <= swipeSlop) return;
							const across = moved.Dot(new Vector2(placement.swipe.Y, placement.swipe.X));
							if (math.abs(along) < math.abs(across)) {
								gesture = undefined;
								return;
							}
							gesture.claimed = true;
							swiping(id);
							block(input);
						}
						const now = os.clock();
						gesture.samples = [
							...gesture.samples.filter((sample) => now - sample.at <= 0.1),
							{ at: now, travel: along },
						];
						travel(along);
					}),
				),
			);
			cleanup(
				UserInputService.InputEnded.Connect((input) =>
					untrack(() => {
						if (!gesture || input !== gesture.input) return;
						const ended = gesture;
						gesture = undefined;
						if (!ended.claimed) return;
						const dismissed = swipeDismisses(travel(), swipeVelocity(ended.samples, os.clock()));
						swiping(undefined);
						if (dismissed) core.dismiss(id, "Swipe");
						else travel(0);
					}),
				),
			);
			cleanup(UserInputService.WindowFocusReleased.Connect(() => untrack(cancel)));
			effect(() => {
				if (closing()) untrack(cancel);
			});
			cleanup(() => {
				cancel();
				measures.delete(measure);
				sizes(put(untrack(sizes), id, undefined));
				hosts(put(untrack(hosts), id, undefined));
				scaler.Destroy();
			});
		};
		return { binding, inert, root };
	};

	return {
		ids: () => ids(),
		slot,
		viewport: (instance: GuiObject) => {
			frame = instance;
			const probe = new Instance("Frame");
			probe.Name = "ScaleProbe";
			probe.BackgroundTransparency = 1;
			probe.Size = UDim2.fromOffset(probeWidth, 0);
			probe.Parent = instance;
			const catcher = new Instance("Frame");
			catcher.Name = "HoverCatcher";
			catcher.BackgroundTransparency = 1;
			catcher.ZIndex = 0;
			catcher.Parent = instance;
			effect(() => {
				const area = stack().bounds;
				catcher.Visible = area !== undefined;
				if (!area) return;
				catcher.Position = new UDim2(placement.anchor.X, area.min.X, placement.anchor.Y, area.min.Y);
				catcher.Size = UDim2.fromOffset(area.max.X - area.min.X, area.max.Y - area.min.Y);
			});
			const rescale = () => {
				const measured = probe.AbsoluteSize.X / probeWidth;
				if (!(measured > 0)) return;
				ratio = measured;
				for (const measure of measures) measure();
			};
			cleanup(probe.GetPropertyChangedSignal("AbsoluteSize").Connect(rescale));
			cleanup(catcher.MouseEnter.Connect(() => stackHovered(true)));
			cleanup(catcher.MouseLeave.Connect(() => stackHovered(false)));
			cleanup(() => {
				probe.Destroy();
				catcher.Destroy();
				if (frame === instance) frame = undefined;
			});
			rescale();
		},
	};
}
