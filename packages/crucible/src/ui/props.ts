import type Vide from "@rbxts/vide";

export type Value<T> = Vide.Derivable<T>;
export type Getter<T> = () => T;
export type Children = Vide.Node;
export type ChildFactory = () => Vide.Node;
