// Send only to a local receiver that you explicitly choose.
stopRegionOutput();
solid(() => 0.5 + 0.5 * Math.sin(time),0.2,0.6,1).out(o0);
var level = region(o0,0,0,1,1,0);
sendRegion(level,"127.0.0.1:9000","/hydra/brightness",30);
render(o0);
