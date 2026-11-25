Object.defineProperty(Array.prototype, "scaled", {
  configurable: true,
  value: function (factor) {
    return this.map(value => value * factor);
  }
});

Object.getPrototypeOf(osc()).tintCycle = function (speed = 1) {
  return this.color(
    () => 0.5 + 0.5 * Math.sin(time * speed),
    () => 0.5 + 0.5 * Math.cos(time * speed),
    1
  );
};

setFunction({
  name: "diagonalBands",
  type: "src",
  inputs: [{ name: "frequency", type: "float", default: 12 }],
  glsl: "return vec4(vec3(0.5 + 0.5 * sin((_st.x + _st.y) * frequency)), 1.0);"
});
