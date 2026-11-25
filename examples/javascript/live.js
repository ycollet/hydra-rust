loadScript("libraries/helpers.js");

var rotationSteps = [0, 1, 2, 3].scaled(Math.PI / 2);
var frameCounter = 0;
update = () => { frameCounter += 1; };

osc(20, 0.1, 1)
  .tintCycle(0.5)
  .rotate(rotationSteps)
  .blend(diagonalBands(18), () => 0.2 + 0.1 * Math.sin(time))
  .out();
