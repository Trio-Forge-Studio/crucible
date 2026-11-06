class Particle {
	x: number;
	y: number;
	vx: number;
	vy: number;

	constructor(x: number, y: number, vx: number, vy: number) {
		this.x = x;
		this.y = y;
		this.vx = vx;
		this.vy = vy;
	}

	step(dt: number) {
		this.x += this.vx * dt;
		this.y += this.vy * dt;
	}
}

export function construct(): number {
	let total = 0;
	for (let i = 0; i < 2000; i++) {
		const particle = new Particle(i, -i, 1, 2);
		particle.step(0.5);
		total += particle.x + particle.y;
	}
	return total;
}
