var frequency = parameter("Frequency",12,1,40);
var angle = parameter("Angle",0,-1,1);
var blendAmount = parameter("Blend",0.2,0,1);
osc(frequency).rotate(angle).blend(noise(3),blendAmount).out(o0)
noise(3).out(o1)
shape(4).out(o2)
gradient().out(o3)
