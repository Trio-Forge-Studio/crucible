const counter = {
	value: 0,
	increment() {
		this.value += 1;
	},
};
const log = (message: string) => print(message);
counter.increment();
log("done");
