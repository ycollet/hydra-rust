shape(5,0.4,0.01)
  .rotate(() => time * 0.2)
  .repeat(3,2)
  .out(o0);
src(o0).nativeSobel(2).out(o1);
render(o1);
