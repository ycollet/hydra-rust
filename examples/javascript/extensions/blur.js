osc(100,0.04,0.8).rotate(0.4).color(0.5,0.8,1).out(o0);
src(o0).nativeBlur(3).out(o1);
render(o1);
