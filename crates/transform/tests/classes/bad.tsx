export const views = [
	<frame className="flex bad nope" />,
	<frame className="flex&#32;bad" />,
	<frame className={"flex\u0020bad"} />,
	<frame className={`flex\x20bad`} />,
	<frame className="hovr:flex" />,
	<frame className="bg-white/101" />,
	<frame className="p-0.3" />,
	<frame className="text-white" />,
	<frame className="opacity-50" />,
	<frame className="rounded-md">
		<uicorner />
	</frame>,
];
let mutable = "flex" as const;
export const a = <frame className={mutable} />;
export const b = <frame className={((): "flex" => "flex")()} />;
export const c = (value: string) => <frame className={value} />;
const Card = (props: { className?: string }) => <frame Name={props.className} />;
export const d = <Card className="flex" />;
