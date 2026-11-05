const classes = "bg-brand p-4";
const styles = { card: "rounded-md" } as const;
const typed: { readonly card: "rounded-md" } = { card: "rounded-md" };
export const views = [
	<frame className="flex" />,
	<frame className={"flex-col"} />,
	<frame className={`border`} />,
	<frame className={classes} />,
	<frame className={styles.card} />,
	<frame className={typed.card} />,
	<frame className="" />,
	<frame className="  " />,
];
