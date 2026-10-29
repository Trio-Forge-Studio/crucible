import { boot, BOOTED_ATTRIBUTE } from "@trioforge/crucible/client";

boot({ load: ["src/client/systems/**"] });
script.SetAttribute("CrucibleClient", game.GetService("Players").LocalPlayer.GetAttribute(BOOTED_ATTRIBUTE));
print("S2 client booted");
