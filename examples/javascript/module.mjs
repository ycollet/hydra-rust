import { pulse } from "./libraries/pulse.mjs";

console.info("Loaded the local pulse module.");
osc(20).brightness(() => pulse(time) * 0.2).out();
