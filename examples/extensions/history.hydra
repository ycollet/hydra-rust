// A moving source makes the four-frame echo visible.
setHistoryLimit(8);
shape(3,0.3,0.01)
  .rotate(() => time * 1.5)
  .color(0.2,0.7,1)
  .out(o0);
src(o0).blend(history(o0,4),0.5).out(o1);
render(o1);
