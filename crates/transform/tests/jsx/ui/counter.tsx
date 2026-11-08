import type { Readable } from "@trioforge/crucible";
import { mount } from "@trioforge/crucible/ui";

export function Counter(props: { coins: Readable<number>; onBuy: () => void }) {
	return <textbutton Name="Counter" Text={() => `Coins ${props.coins()}`} TextSize={18} Activated={props.onBuy} />;
}

export function mountCounter(parent: Instance, coins: Readable<number>, onBuy: () => void) {
	return mount(() => <Counter coins={coins} onBuy={onBuy} />, parent);
}
