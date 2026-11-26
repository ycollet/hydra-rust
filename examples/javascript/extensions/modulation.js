var frequency = modulation.lfo("sine",0.5).range(5,30);
var movement = modulation.drift(0.25,13).range(-0.5,0.5);
var pulse = modulation.envelope(0.1,0.4,1);
var tint = modulation.sampleHold(0.5,7).range(0.3,1);
osc(frequency)
  .rotate(movement)
  .brightness(pulse.mul(0.2))
  .color(tint,1,0.8)
  .blend(noise(3),modulation.clock(0.25,0.2,"beats").mul(0.15))
  .out()
