import { mount, Show } from "@trioforge/crucible/ui";

export function mountVariants(parent: Instance) {
	return mount(
		() => (
			<frame Name="Root">
				<textbutton Name="Hover" className="bg-slate-800 hover:bg-brand" />
				<textbutton Name="Active" className="bg-slate-800 active:bg-brand" />
				<textbox Name="Focus" className="bg-slate-800 focus:bg-brand" />
				<frame Name="Wide" className="w-full md:w-1/2" />
				<frame Name="Oriented" className="bg-slate-800 portrait:bg-brand landscape:bg-white" />
				<frame Name="Input" className="bg-slate-800 touch:bg-brand gamepad:bg-white mouse:bg-black" />
				<frame Name="Dark" className="bg-white dark:bg-slate-900" />
				<frame Name="Group" className="group">
					<frame Name="Child" className="bg-slate-800 group-hover:bg-brand" />
				</frame>
				<frame Name="Card" className="group/card">
					<frame Name="Inner" className="group">
						<Show when={() => true}>
							{() => <frame Name="Named" className="bg-slate-800 group-hover/card:bg-brand" />}
						</Show>
					</frame>
				</frame>
				<frame Name="Outlined" className="hover:border-white" />
			</frame>
		),
		parent,
	);
}
