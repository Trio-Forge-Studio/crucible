import { Mode, origin, Shape, Unused } from "./declarations";

const shape: Shape = { size: origin };
const unused: Unused | undefined = undefined;
print(shape.size, Mode.Slow, unused);
