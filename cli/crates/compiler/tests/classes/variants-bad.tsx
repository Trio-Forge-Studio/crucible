const Row = () => <frame className="group-hover:flex" />;
export const views = [
	<frame className="hovr:flex" />,
	<frame className="xl:flex" />,
	<frame className="hover:hover:flex" />,
	<frame className="portrait:landscape:flex" />,
	<frame className="touch:gamepad:flex" />,
	<frame className="group-hover:flex" />,
	<frame className="group">
		<frame className="group-hover/card:flex" />
	</frame>,
	<frame className="group-hover:flex">
		<frame className="group" />
	</frame>,
	<frame className="group">
		<Row />
	</frame>,
	<frame className="hover:bg-brand bg-white" />,
	<frame className="lg:bg-brand md:bg-white" />,
	<frame className="hover:group" />,
];
