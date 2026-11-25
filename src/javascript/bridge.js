(() => {
  "use strict";
  const metadata = new Map(__metadata.map(m => [m.name, m]));
  delete globalThis.__metadata;
  const consoleLog = __consoleLog;
  delete globalThis.__consoleLog;
  globalThis.console = Object.freeze(Object.fromEntries(
    ["log", "info", "debug", "warn", "error"].map(level => [
      level, (...values) => consoleLog(level === "log" ? "info" : level, values.map(String).join(" "))
    ])
  ));
  let callbacks = [], committedCallbacks = [], definitions = new Map(), buffers = [null,null,null,null];
  let saved, committedUpdate = null, audioRead = false, running = false;
  let activeSlots = new Set(), pendingActiveSlots = new Set();
  const callbackFunctions = new WeakMap();
  let renderBuffer = null, all = false, resolution = null, filters = [null,null,null,null], capture = false;
  const unsupported = name => () => { throw new Error(`${name} is unsupported in embedded JavaScript`); };
  const editing = () => {
    if (running) throw new Error("graph changes during frame are unsupported");
  };
  const allocateSlot = fn => {
    let i = callbacks.indexOf(fn);
    if (i < 0) {
      i = callbacks.indexOf(null);
      if (i < 0) {
        if (callbacks.length >= 128) throw new Error("JavaScript callback slot limit");
        i = callbacks.length;
      }
      callbacks[i] = fn;
    }
    return i;
  };
  const slot = fn => {
    const arg = {type:"callback", value:allocateSlot(fn)};
    callbackFunctions.set(arg,fn);
    return arg;
  };
  const value = v => {
    if (typeof v === "function") return slot(v);
    if (Array.isArray(v)) {
      if (!v.length) throw new Error("empty pattern");
      return slot(() => {
        const speed = v._speed ?? 1, offset = v._offset ?? 0;
        const position = time * tempo / 60 * speed + offset;
        const index = ((Math.floor(position) % v.length) + v.length) % v.length;
        return v[index];
      });
    }
    if (typeof v !== "number" || !Number.isFinite(v)) throw new Error("shader arguments require finite numbers or callbacks");
    return {type:"number",value:v};
  };
  class Node {
    constructor(ops) { this.ops = ops; }
    out(buffer = o0) {
      editing();
      const index = outputIndex(buffer);
      buffers[index] = this;
      return this;
    }
  }
  const outputIndex = buffer => {
    const i = outputs.indexOf(buffer);
    if (i < 0) throw new Error("expected o0, o1, o2, or o3");
    return i;
  };
  const outputs = Array.from({length:4}, (_,i) => Object.freeze({
    index:i,
    setNearest() { editing(); filters[i] = "nearest"; return this; },
    setLinear() { editing(); filters[i] = "linear"; return this; },
    setMode(mode) {
      editing();
      if (mode !== "nearest" && mode !== "linear") throw new Error("unsupported buffer mode");
      filters[i] = mode;
    }
  }));
  const sources = Array.from({length:4}, (_,i) => new Proxy({index:i}, {
    get(target, key) { return key in target ? target[key] : unsupported(`source.${String(key)}`); }
  }));
  function texture(v) {
    if (v instanceof Node) return v;
    if (outputs.includes(v) || sources.includes(v)) return src(v);
    if (typeof v === "number") return solid(v,v,v,1);
    throw new Error("expected a texture node");
  }
  function install(m) {
    const call = (node, supplied) => {
      editing();
      let other = null;
      if (m.kind === "combine" || m.kind === "combineCoord") other = texture(supplied.shift());
      if (supplied.length > m.defaults.length) throw new Error(`too many arguments for ${m.name}`);
      const args = m.defaults.map((d,i) => value(supplied[i] === undefined ? d : supplied[i]));
      const op = {name:m.name,args,other};
      return new Node(node ? [...node.ops,op] : [op]);
    };
    if (m.kind === "src") {
      delete Node.prototype[m.name];
      globalThis[m.name] = (...args) => call(null,args);
    } else {
      delete globalThis[m.name];
      Node.prototype[m.name] = function(...args) { return call(this,args); };
    }
  }
  for (const m of metadata.values()) install(m);
  function setFunction(def) {
    editing();
    if (!def || typeof def.name !== "string" || typeof def.glsl !== "string") throw new Error("invalid setFunction definition");
    if (!["src","coord","color","combine","combineCoord"].includes(def.type)) throw new Error("unsupported setFunction type");
    const inputs = (def.inputs ?? []).map(i => {
      if (i.type !== "float") throw new Error("only float shader inputs are supported");
      const d = i.default ?? 0;
      if (typeof d !== "number" || !Number.isFinite(d)) throw new Error("invalid input default");
      return {name:i.name,default:d};
    });
    if (["src","ext_src","out","constructor"].includes(def.name)) throw new Error("cannot replace graph infrastructure");
    if (!definitions.has(def.name) && definitions.size >= 256) throw new Error("custom function limit");
    definitions.set(def.name,{name:def.name,kind:def.type,inputs,glsl:def.glsl});
    install({name:def.name,kind:def.type,defaults:inputs.map(i=>i.default)});
  }
  function src(buffer) {
    if (outputs.includes(buffer)) return new Node([{name:"src",args:[value(buffer.index)],other:null}]);
    if (sources.includes(buffer)) return new Node([{name:"ext_src",args:[value(buffer.index)],other:null}]);
    throw new Error("src requires an output or source buffer");
  }
  Object.assign(globalThis, {Node,src,setFunction,window:globalThis,
    time:0,beat:0,tempo:120,bpm:120,phase:0,width:1,height:1,innerWidth:1,innerHeight:1,
    mouse:{x:0,y:0},update:()=>{},
    render(buffer) { editing(); all = buffer === undefined; renderBuffer = all ? null : outputIndex(buffer); },
    setResolution(w,h) {
      editing();
      if (!Number.isInteger(w) || !Number.isInteger(h) || w < 1 || h < 1 || w > 16384 || h > 16384) throw new Error("invalid resolution");
      resolution=[w,h];
    },
    screencap() { editing(); capture=true; },
    hush() { editing(); buffers = outputs.map(()=>solid(0,0,0,0)); },
    fetch:unsupported("fetch"),requestAnimationFrame:unsupported("requestAnimationFrame"),
    setTimeout:unsupported("setTimeout"),setInterval:unsupported("setInterval")
  });
  outputs.forEach((o,i)=>globalThis[`o${i}`]=o);
  sources.forEach((s,i)=>globalThis[`s${i}`]=s);
  let fft = [0,0,0,0,0,0,0,0], rhythm = {onset:0,beat:0,bpm:0};
  const audio = new Proxy({}, {
    get(_,key) {
      if (key === "fft") { audioRead=true; return fft; }
      if (key in rhythm) { audioRead=true; return rhythm[key]; }
      return unsupported(`a.${String(key)}`);
    }
  });
  globalThis.a = audio;
  globalThis.midi = new Proxy({}, {get:(_,key)=>unsupported(`midi.${String(key)}`)});
  const facade = {
    regl: Object.freeze({embedded:true}),
    synth:{setFunction},
    sandbox:{makeGlobal() { return globalThis; }}
  };
  globalThis._hydra = new Proxy(facade, {get:(t,k)=> k in t ? t[k] : unsupported(`_hydra.${String(k)}`)});
  globalThis.hydra = _hydra;
  globalThis.getHydra = () => _hydra;
  const serialize = node => node === null ? null : {
    ops:node.ops.map(op=>({
      name:op.name,
      args:op.args.map(arg=>{
        const fn = callbackFunctions.get(arg);
        const encoded = fn ? {type:"callback",value:allocateSlot(fn)} : arg;
        if (encoded.type === "callback") {
          if (typeof callbacks[encoded.value] !== "function") throw new Error("invalid callback slot");
          pendingActiveSlots.add(encoded.value);
        }
        return encoded;
      }),
      other:serialize(op.other)
    }))
  };
  globalThis.__bridge = Object.freeze({
    begin() {
      saved={callbacks:callbacks.slice(),buffers:buffers.slice(),definitions:new Map(definitions),
        renderBuffer,all,resolution,filters:filters.slice(),capture,audioRead,
        committedCallbacks:committedCallbacks.slice(),committedUpdate,activeSlots,savedUpdate:update};
      callbacks=callbacks.map((fn,i)=>activeSlots.has(i) ? fn : null);
      capture=false; filters=[null,null,null,null];
    },
    rollback() {
      const oldDefinitions = definitions;
      ({callbacks,buffers,definitions,renderBuffer,all,resolution,filters,capture,audioRead,committedCallbacks,committedUpdate,activeSlots}=saved);
      globalThis.update=saved.savedUpdate;
      for (const [name,def] of oldDefinitions) {
        if (!definitions.has(name)) {
          if (metadata.has(name)) install(metadata.get(name));
          else if (def.kind === "src") delete globalThis[name];
          else delete Node.prototype[name];
        }
      }
      for (const def of definitions.values()) install({name:def.name,kind:def.kind,defaults:def.inputs.map(i=>i.default)});
    },
    commit() {
      if (typeof update !== "function") throw new Error("update must be a function");
      committedCallbacks=callbacks.slice();
      committedUpdate=update;
      activeSlots=pendingActiveSlots;
    },
    snapshot() {
      pendingActiveSlots=new Set();
      const encodedBuffers=buffers.map(serialize);
      return JSON.stringify({buffers:encodedBuffers,definitions:[...definitions.values()],
        render:renderBuffer,all,resolution,filters,screencap:capture,slots:callbacks.length,audio:audioRead});
    },
    audio() { return audioRead; },
    finishFrame() { running=false; },
    frame(input) {
      Object.assign(globalThis,{time:input.time,beat:input.beat,tempo:input.tempo,bpm:input.tempo,
        phase:input.phase,width:input.resolution[0],height:input.resolution[1],
        innerWidth:input.resolution[0],innerHeight:input.resolution[1]});
      mouse.x=input.mouse[0]; mouse.y=input.mouse[1]; fft=input.fft;
      rhythm={onset:input.audio_onset,beat:input.audio_beat,bpm:input.audio_bpm};
      running=true;
      if (committedUpdate) committedUpdate(input.time);
      const values=Array(128).fill(0);
      committedCallbacks.forEach((fn,i)=>{
        if (!activeSlots.has(i)) return;
        const v=fn({time,beat,bpm,tempo,phase,mouse,width,height});
        if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`callback ${i} must return a finite number`);
        values[i]=v;
      });
      return JSON.stringify(values);
    }
  });
})();
