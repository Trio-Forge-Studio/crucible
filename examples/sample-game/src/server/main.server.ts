import { boot } from "@trioforge/crucible/server";

boot({ load: ["src/server/systems/**"] });
script.SetAttribute("CrucibleServer", true);
print("S2 server booted");
