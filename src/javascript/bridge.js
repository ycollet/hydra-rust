(() => {
  "use strict";
  const parseFrame = JSON.parse;
  const metadata = new Map(__metadata.map(m => [m.name, m]));
  for (const spec of metadata.values()) spec.inputs = spec.defaults.map((value,index)=>
    ({name:`i${index}`,type:"float",default:value}));
  delete globalThis.__metadata;
  const functionLimits = __functionLimits;
  delete globalThis.__functionLimits;
  const features = __features;
  delete globalThis.__features;
  const consoleLog = __consoleLog;
  delete globalThis.__consoleLog;
  globalThis.console = Object.freeze(Object.fromEntries(
    ["log", "info", "debug", "warn", "error"].map(level => [
      level, (...values) => consoleLog(level === "log" ? "info" : level, values.map(String).join(" "))
    ])
  ));
  let callbacks = [], committedCallbacks = [], definitions = new Map(), buffers = [null,null,null,null];
  let definitionBytes = 0;
  let parameters = new Map();
  let declaredParameters = new Set();
  let saved, committedUpdate = null, audioRead = false, running = false;
  let activeSlots = new Set(), pendingActiveSlots = new Set();
  const callbackFunctions = new WeakMap();
  const modulationSignals = new WeakMap();
  const modulationSizes = new WeakMap();
  class ModulationSignal {
    constructor(descriptor) {
      const children = [descriptor.left,descriptor.right,descriptor.signal].filter(Boolean);
      const size = {
        nodes:1+children.reduce((sum,child)=>sum+modulationSizes.get(child).nodes,0),
        depth:children.length ? 1+Math.max(...children.map(child=>modulationSizes.get(child).depth)) : 0
      };
      if (size.nodes>64 || size.depth>16) throw new Error("Modulation expression exceeds 64 nodes or 16 nesting levels");
      modulationSizes.set(descriptor,size);
      modulationSignals.set(this, descriptor);
      Object.freeze(this);
    }
    add(other) { return modulationBinary("add",this,other); }
    sub(other) { return modulationBinary("sub",this,other); }
    mul(other) { return modulationBinary("mul",this,other); }
    scale(other) { return this.mul(other); }
    div(divisor) {
      finite(divisor);
      if (divisor === 0) throw new Error("Modulation divisor must be nonzero");
      return new ModulationSignal({kind:"div",signal:modulationSignals.get(this),divisor});
    }
    range(low,high) {
      return new ModulationSignal({kind:"range",signal:modulationSignals.get(this),low:finite(low),high:finite(high)});
    }
  }
  const modulationDescriptor = value => {
    if (modulationSignals.has(value)) return modulationSignals.get(value);
    const descriptor = {kind:"number",value:finite(value)};
    modulationSizes.set(descriptor,{nodes:1,depth:0});
    return descriptor;
  };
  const modulationBinary = (kind,left,right) => new ModulationSignal({
    kind,left:modulationDescriptor(left),right:modulationDescriptor(right)
  });
  globalThis.modulation = Object.freeze({
    lfo(wave,rate=1,phase=0,base="seconds") {
      return new ModulationSignal({kind:"lfo",wave,rate:finite(rate),phase:finite(phase),base});
    },
    envelope(attack,decay,period,base="seconds") {
      return new ModulationSignal({kind:"envelope",attack:finite(attack),decay:finite(decay),period:finite(period),base});
    },
    sampleHold(rate=1,seed=0,base="seconds") {
      return new ModulationSignal({kind:"sampleHold",rate:finite(rate),seed:finite(seed),base});
    },
    drift(rate=1,seed=0,base="seconds") {
      return new ModulationSignal({kind:"drift",rate:finite(rate),seed:finite(seed),base});
    },
    clock(rate=1,duty=0.5,base="seconds") {
      return new ModulationSignal({kind:"clock",rate:finite(rate),duty:finite(duty),base});
    }
  });
  const arrayCallbacks = new WeakMap(), callbackArrays = new WeakMap();
  let renderBuffer = null, all = false, resolution = null, filters = [null,null,null,null], capture = false;
  let textRequest = null, clear = [false,false,false,false];
  let commands = [], midiBindings = new Map(), envelopes = new Map(), oscBindings = new Map();
  let midiValues = Array(64).fill(0), midiEnvelopes = Array(16).fill(0), oscValues = Array(32).fill(0);
  let audioSettings = features.fft ? __audioSettings : null;
  let regions=[],regionOutputs=[],regionValues=Array(16).fill(0),historyLimit=8;
  let regionSampling={rate:0,asynchronous:false};
  const regionSlots=new WeakMap();
  delete globalThis.__audioSettings;
  const unsupported = name => () => { throw new Error(`${name} is unsupported in embedded JavaScript`); };
  const editing = () => {
    if (running) throw new Error("graph changes during frame are unsupported");
  };
  const requireFeature = feature => {
    if (!features[feature]) throw new Error(`This native operation requires the ${feature} feature`);
  };
  const integer = (value,min,max,name) => {
    if (!Number.isInteger(value) || value < min || value > max) throw new Error(`Invalid ${name}`);
    return value;
  };
  const finite = value => {
    if (typeof value !== "number" || !Number.isFinite(value) || Math.abs(value) > 3.402823466e38) throw new Error("Native controls require finite numbers");
    return value;
  };
  globalThis.parameter = (name,defaultValue,min,max) => {
    editing();
    if (typeof name !== "string" || !name.trim()) throw new Error("Parameter names must be nonempty strings");
    const spec = {name,default:finite(defaultValue),min:finite(min),max:finite(max)};
    if (min>=max || defaultValue<min || defaultValue>max) throw new Error("Parameter default must lie inside finite bounds with min < max");
    const previous = parameters.get(name);
    if (previous && declaredParameters.has(name) && JSON.stringify(previous.spec)!==JSON.stringify(spec)) throw new Error(`Conflicting declarations for parameter ${name}`);
    if (!previous) {
      if (parameters.size>=64) throw new Error("Named parameter limit exceeded (64)");
      parameters.set(name,{slot:parameters.size,spec});
    }
    else parameters.set(name,{slot:previous.slot,spec});
    declaredParameters.add(name);
    return new ModulationSignal({kind:"parameter",name,slot:parameters.get(name).slot});
  };
  const queue = command => {
    editing();
    if (commands.length >= 144) throw new Error("Native request limit exceeded");
    commands.push(command);
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
    if (modulationSignals.has(v)) return {type:"modulation",value:modulationSignals.get(v)};
    if (v instanceof Node) return {type:"parameter",value:v};
    if (typeof v === "function") return slot(v);
    if (Array.isArray(v)) {
      if (!v.length) throw new Error("empty pattern");
      if (!arrayCallbacks.has(v)) {
        const callback = () => {
          const speed = v._speed ?? 1, offset = v._offset ?? 0;
          const position = time * tempo / 60 * speed + offset;
          const index = ((Math.floor(position) % v.length) + v.length) % v.length;
          return v[index];
        };
        arrayCallbacks.set(v,callback);
        callbackArrays.set(callback,v);
      }
      return slot(arrayCallbacks.get(v));
    }
    if (typeof v !== "number" || !Number.isFinite(v)) throw new Error("shader arguments require finite numbers or callbacks");
    return {type:"number",value:v};
  };
  class OutputNode {
    out(buffer = o0) {
      editing();
      const index = outputIndex(buffer);
      buffers[index] = this;
      return this;
    }
  }
  const decode = arg => {
    if (arg.type === "modulation") return new ModulationSignal(arg.value);
    if (arg.type === "parameter") return arg.value;
    if (arg.type === "number") return arg.value;
    if (arg.type === "callback" && callbackFunctions.has(arg)) {
      const callback = callbackFunctions.get(arg);
      return callbackArrays.get(callback) ?? callback;
    }
    if (arg.type === "texture") return arg.value.kind === "output" ? outputs[arg.value.index] : sources[arg.value.index];
    throw new Error("Invalid encoded graph argument");
  };
  class Node extends OutputNode {
    constructor(ops) {
      super();
      this.transforms = ops.map(op => {
        let name = op.name;
        let userArgs = op.args.map(decode);
        if (name === "src" || name === "ext_src") {
          userArgs = [name === "src" ? outputs[userArgs[0]] : sources[userArgs[0]]];
          name = "src";
        } else if (op.other) userArgs.unshift(op.other);
        const spec = definitions.get(name) ?? metadata.get(name);
        return {name,transform:{name,type:spec?.kind ?? "src",inputs:spec?.inputs ?? []},userArgs};
      });
    }
    get ops() {
      if (!Array.isArray(this.transforms) || this.transforms.length > 4096) throw new Error("Invalid graph transforms");
      return this.transforms.map(transform => {
        if (!transform?.transform || !Array.isArray(transform.userArgs)) throw new Error("Invalid graph transform");
        const name = transform.transform.name;
        const args = transform.userArgs.slice();
        if (name === "src") {
          const reference = textureRef(args[0]);
          return {name:reference.kind === "output" ? "src" : "ext_src",args:[value(reference.index)],other:null};
        }
        if (name === "text_src") return {name,args:[],other:null};
        if (name === "native_history") return {name,args:args.map(value),other:null};
        if (["native_blur","native_sharpen","native_sobel","native_emboss","native_dilate","native_erode"].includes(name)) {
          if (!(args[0] instanceof Node)) throw new Error("Missing native effect graph");
          return {name,args:[value(args[1])],other:args[0]};
        }
        if (name === "tileable") {
          if (!(args[0] instanceof Node)) throw new Error("Invalid tileable graph");
          return {name,args:[value(args[1])],other:args[0]};
        }
        const spec = definitions.get(name) ?? metadata.get(name);
        if (!spec) throw new Error(`Unknown graph function ${name}`);
        return operation(spec,args);
      });
    }
    tileable(width = 0.15) {
      editing();
      return new Node([{name:"tileable",args:[value(width)],other:this}]);
    }
    mul(factor) {
      editing(); finite(factor);
      return this.mult(solid(factor,factor,factor,1));
    }
  }
  const nativeEffects=new Map();
  for (const name of ["blur","sharpen","sobel","emboss","dilate","erode"]) {
    Node.prototype[name] = function(radius = 1) {
      editing(); finite(radius);
      if (radius < 0 || radius > 32) throw new Error("Effect radius must be in 0..32 pixels");
      return new Node([{name:`native_${name}`,args:[value(radius)],other:this}]);
    };
    Node.prototype[`native${name[0].toUpperCase()}${name.slice(1)}`] = Node.prototype[name];
    nativeEffects.set(name,Node.prototype[name]);
  }
  const outputIndex = buffer => {
    const i = outputs.indexOf(buffer);
    if (i < 0) throw new Error("expected o0, o1, o2, or o3");
    return i;
  };
  const outputs = Array.from({length:4}, (_,i) => Object.freeze({
    index:i,
    clear() { editing(); clear[i] = true; return this; },
    setNearest() { editing(); filters[i] = "nearest"; return this; },
    setLinear() { editing(); filters[i] = "linear"; return this; },
    setMode(mode) {
      editing();
      if (mode !== "nearest" && mode !== "linear") throw new Error("unsupported buffer mode");
      filters[i] = mode;
    }
  }));
  const sources = Array.from({length:4}, (_,i) => new Proxy({
    index:i,
    initCam(index = 0) {
      editing(); requireFeature("webcam");
      queue({kind:"cam",slot:i,index:integer(index,0,4294967295,"camera index")});
      return this;
    },
    ...Object.fromEntries([["initImage","image","image_url"],["initGif","gif","image_url"],["initVideo","video","video"],["initStream","stream","stream"]].map(([name,kind,feature])=>[
      name,function(location) {
        editing(); requireFeature(feature);
        if (typeof location !== "string" || !location.length || location.length > 4096) throw new Error("Source location requires a nonempty string of at most 4096 characters");
        queue({kind,slot:i,[kind === "stream" ? "addr" : "url"]:location});
        return this;
      }
    ]))
  }, {
    get(target, key) { return key in target ? target[key] : unsupported(`source.${String(key)}`); }
  }));
  function texture(v) {
    if (v instanceof Node) return v;
    if (outputs.includes(v) || sources.includes(v)) return src(v);
    if (typeof v === "number") return solid(v,v,v,1);
    throw new Error("expected a texture node");
  }
  const textureRef = texture => {
    if (outputs.includes(texture)) return {kind:"output",index:texture.index};
    if (sources.includes(texture)) return {kind:"source",index:texture.index};
    throw new Error("Sampler inputs require a native output or source buffer");
  };
  function operation(m, supplied) {
    editing();
    let other = null;
    if (m.kind === "combine" || m.kind === "combineCoord") other = texture(supplied.shift());
    const inputs = m.inputs ?? m.defaults.map(defaultValue=>({type:"float",default:defaultValue}));
    if (supplied.length > inputs.length) throw new Error(`too many arguments for ${m.name}`);
    const args = inputs.map((input,index)=>{
      if (input.type === "sampler2D") return {type:"texture",value:
        supplied[index] === undefined ? input.default : textureRef(supplied[index])};
      return value(supplied[index] === undefined ? input.default : supplied[index]);
    });
    return {name:m.name,args,other};
  }
  function install(m) {
    const call = (node, supplied) => new Node(node ? [...node.ops,operation(m,supplied)] : [operation(m,supplied)]);
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
    const identifier = name => typeof name === "string" && name.length <= functionLimits.name &&
      /^[A-Za-z_][A-Za-z0-9_]*$/.test(name) && !name.startsWith("gl_");
    if (!identifier(def.name) || def.glsl.length > functionLimits.body) throw new Error("Invalid custom function name or size");
    if (!Array.isArray(def.inputs ?? []) || (def.inputs ?? []).length > functionLimits.inputs) throw new Error("Invalid custom function inputs");
    const names = new Set();
    const inputs = (def.inputs ?? []).map(i => {
      if (!identifier(i.name) || ["st","c0","c1","_st","_c0","_c1"].includes(i.name) || names.has(i.name)) throw new Error("Invalid custom function input name");
      names.add(i.name);
      if (i.type === "sampler2D") return {name:i.name,type:i.type,default:textureRef(i.default ?? o0)};
      if (i.type !== "float") throw new Error("Only float and sampler2D shader inputs are supported");
      const d = i.default ?? 0;
      finite(d);
      return {name:i.name,type:i.type,default:d};
    });
    if (def.name.startsWith("native_") || ["src","ext_src","text_src","tileable","text","strokeText","fillStrokeText","strokeFillText","out","constructor","ops","transforms","history","setHistoryLimit","setRegionSampling","region","sendRegion","stopRegionOutput","mul","nativeBlur","nativeSharpen","nativeSobel","nativeEmboss","nativeDilate","nativeErode"].includes(def.name)) throw new Error("cannot replace graph infrastructure");
    if (!definitions.has(def.name) && definitions.size >= functionLimits.count) throw new Error("custom function limit");
    const bytes = definitionBytes - (definitions.get(def.name)?.glsl.length ?? 0) + def.glsl.length;
    if (bytes > functionLimits.bytes) throw new Error("Custom shader registry byte limit exceeded");
    definitionBytes=bytes;
    definitions.set(def.name,{name:def.name,kind:def.type,inputs,glsl:def.glsl});
    install(definitions.get(def.name));
  }
  function src(buffer) {
    if (outputs.includes(buffer)) return new Node([{name:"src",args:[value(buffer.index)],other:null}]);
    if (sources.includes(buffer)) return new Node([{name:"ext_src",args:[value(buffer.index)],other:null}]);
    throw new Error("src requires an output or source buffer");
  }
  Object.assign(globalThis, {Node,src,setFunction,window:globalThis,
    setHistoryLimit(limit) {editing();integer(limit,1,8,"history limit");historyLimit=limit;},
    setRegionSampling(rate,asynchronous=false) {
      editing();finite(rate);
      if((rate!==0&&(rate<1||rate>240))||typeof asynchronous!=="boolean")throw new Error("Invalid region sampling rate or mode");
      regionSampling={rate,asynchronous};
    },
    history(output,delay=1) {
      editing(); integer(delay,1,8,"history delay");
      return new Node([{name:"native_history",args:[value(outputIndex(output)),value(delay)],other:null}]);
    },
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
      if (key === "bins") {
        requireFeature("fft");
        return Array.from({length:audioSettings.bins},(_,index)=>new Proxy({},{
          get(_,property) {
            if (!["cutoff","scale","smooth"].includes(property)) throw new Error("Unsupported audio bin property");
            if (index >= audioSettings.bins) throw new Error("Audio bin index is outside the configured bins");
            return audioSettings.settings[index][property];
          },
          set(_,property,v) {
            editing();
            if (!["cutoff","scale","smooth"].includes(property)) throw new Error("Unsupported audio bin property");
            if (index >= audioSettings.bins) throw new Error("Audio bin index is outside the configured bins");
            queue({kind:"audio",method:property,value:finite(v),index});
            audioSettings.settings[index][property] = property === "cutoff" ? Math.max(0,v) :
              property === "smooth" ? Math.max(0,Math.min(1,v)) : v;
            audioRead=true;
            return true;
          }
        }));
      }
      if (["setBins","setCutoff","setScale","setSmooth","show","hide"].includes(key)) return v => {
        editing(); requireFeature("fft");
        if (key === "setBins") {
          const bins = integer(v,1,8,"audio bin count");
          queue({kind:"audio",method:key,value:bins});
          audioSettings.bins=bins;
          audioSettings.settings=Array.from({length:8},()=>({...audioSettings.defaults}));
        } else if (key === "show" || key === "hide") {
          if (v !== undefined) throw new Error("Audio show and hide accept no arguments");
          queue({kind:"audio",method:key});
        }
        else {
          const property = key.slice(3).toLowerCase();
          queue({kind:"audio",method:property,value:finite(v)});
          const value = property === "cutoff" ? Math.max(0,v) :
            property === "smooth" ? Math.max(0,Math.min(1,v)) : v;
          audioSettings.defaults[property]=value;
          audioSettings.settings.forEach(bin=>bin[property]=value);
        }
        audioRead=true;
        return audio;
      };
      return unsupported(`a.${String(key)}`);
    }
  });
  globalThis.a = audio;
  const signal = read => {
    const fn = () => read();
    fn.range = (lo,hi) => { finite(lo); finite(hi); return signal(()=>lo+read()*(hi-lo)); };
    fn.scale = factor => { finite(factor); return signal(()=>read()*factor); };
    return fn;
  };
  const selector = (channel,input) => ({
    channel:channel === "*" || channel === undefined ? null : integer(channel,0,15,"MIDI channel"),
    input:input === "*" || input === undefined ? null :
      typeof input === "string" && input.length && input.length <= 1024 ? input :
      integer(input,0,65535,"MIDI input")
  });
  const binding = spec => {
    editing(); requireFeature("midi");
    const key = JSON.stringify(spec);
    if (!midiBindings.has(key)) {
      if (midiBindings.size >= 64) throw new Error("MIDI value slot limit exceeded; reset the interpreter");
      midiBindings.set(key,{kind:"midiBind",slot:midiBindings.size,...spec});
    }
    const request = midiBindings.get(key);
    const fn = signal(()=>midiValues[request.slot]);
    fn.smooth = (factor = 0.01) => {
      if (spec.value !== "cc") throw new Error("Only MIDI controller values support smoothing");
      finite(factor);
      if (factor < 0 || factor > 1) throw new Error("MIDI smoothing must be in 0..1");
      return binding({...spec,smooth:factor});
    };
    fn.adsr = (a,d,s,r) => {
      editing(); requireFeature("midi");
      if (spec.value !== "note" && spec.value !== "velocity") throw new Error("MIDI envelopes require note or velocity");
      const adsr = [a,d,s,r].map(finite);
      if (adsr.some(v=>v<0) || s>1) throw new Error("Invalid MIDI envelope");
      const descriptor = {note:spec.index ?? -1,channel:spec.channel,input:spec.input,adsr};
      const key = JSON.stringify(descriptor);
      if (!envelopes.has(key)) {
        if (envelopes.size >= 16) throw new Error("MIDI envelope slot limit exceeded; reset the interpreter");
        envelopes.set(key,{kind:"midiAdsr",slot:envelopes.size,...descriptor});
      }
      const slot = envelopes.get(key).slot;
      return signal(()=>midiEnvelopes[slot]);
    };
    return fn;
  };
  globalThis.midi = new Proxy({}, {get:(_,key)=>{
    if (["start","pause","show","hide"].includes(key)) return (...args) => {
      editing(); requireFeature("midi");
      if (args.length) throw new Error("MIDI controls accept no arguments; select channels and inputs on value methods");
      queue({kind:"midiControl",method:key}); return midi;
    };
    if (["note","velocity","cc","aft"].includes(key)) return (index,channel,input) => binding({
      value:key,index:index === "*" || index === undefined ? null : integer(index,0,127,"MIDI index"),
      ...selector(channel,input),smooth:null
    });
    return unsupported(`midi.${String(key)}`);
  }});
  globalThis._osc = new Proxy({}, {get:(_,key)=>{
    if (["start","pause","show","hide"].includes(key)) return port => {
      editing(); requireFeature("osc");
      if (key !== "start" && port !== undefined) throw new Error("Only OSC start accepts a port");
      queue({kind:"oscControl",method:key,port:key === "start" ? integer(port ?? 57101,1,65535,"OSC port") : null});
      return _osc;
    };
    if (key === "get") return (address,index = 0) => {
      editing(); requireFeature("osc");
      if (typeof address !== "string" || !address.startsWith("/") || address.length > 1024) throw new Error("OSC addresses require a leading slash");
      integer(index,0,255,"OSC argument index");
      const key = JSON.stringify([address,index]);
      if (!oscBindings.has(key)) {
        if (oscBindings.size >= 32) throw new Error("OSC slot limit exceeded; reset the interpreter");
        oscBindings.set(key,{kind:"oscBind",slot:oscBindings.size,address,index});
      }
      const slot = oscBindings.get(key).slot;
      return signal(()=>oscValues[slot]);
    };
    return unsupported(`_osc.${String(key)}`);
  }});
  globalThis.region=(output,x=0,y=0,width=1,height=1,channel=4)=>{
    editing();
    [x,y,width,height].forEach(finite);
    integer(channel,0,4,"region channel");
    if(x<0||y<0||width<=0||height<=0||x+width>1||y+height>1)throw new Error("Invalid normalized region rectangle");
    const descriptor={output:outputIndex(output),rect:[x,y,width,height],channel};
    let slot=regions.findIndex(region=>JSON.stringify(region)===JSON.stringify(descriptor));
    if(slot<0){
      if(regions.length>=16)throw new Error("Image region limit exceeded; reset the interpreter");
      slot=regions.length;regions.push(descriptor);
    }
    const callback=signal(()=>regionValues[slot]);
    regionSlots.set(callback,{slot,region:regions[slot]});
    return callback;
  };
  globalThis.sendRegion=(control,destination,address,rate=30)=>{
    editing();requireFeature("osc");
    const binding=regionSlots.get(control);
    if(!binding||regions[binding.slot]!==binding.region||typeof destination!=="string"||destination.length>128||
      typeof address!=="string"||!address.startsWith("/")||address.length>1024||
      !Number.isFinite(rate)||rate<1||rate>60)throw new Error("Invalid region OSC output");
    const descriptor={slot:binding.slot,destination,address,rate};
    if(!regionOutputs.some(output=>JSON.stringify(output)===JSON.stringify(descriptor))){
      if(regionOutputs.length>=16)throw new Error("Region OSC output limit exceeded");
      regionOutputs.push(descriptor);
    }
  };
  globalThis.stopRegionOutput=()=>{editing();regionOutputs=[];};
  const facade = {
    regl: Object.freeze({embedded:true}),
    synth:{setFunction},
    sandbox:{makeGlobal() { return globalThis; }}
  };
  globalThis._hydra = new Proxy(facade, {get:(t,k)=> k in t ? t[k] : unsupported(`_hydra.${String(k)}`)});
  globalThis.hydra = _hydra;
  globalThis.getHydra = () => _hydra;
  globalThis.hydraText = {fontSize:"auto",lineWidth:"2%"};
  for (const [name,mode] of [["text","fill"],["strokeText","stroke"],["fillStrokeText","fillStroke"],["strokeFillText","fillStroke"]]) {
    globalThis[name] = (content,config = {}) => {
      editing();
      if (typeof content !== "string" || content.length > 4096) throw new Error("Text requires a string of at most 4096 bytes");
      if (typeof config === "string") config = {};
      if (!config || typeof config !== "object" || Array.isArray(config)) throw new Error("Text configuration requires an object");
      const fontSize = config.fontSize ?? hydraText.fontSize;
      const lineWidth = config.lineWidth ?? hydraText.lineWidth;
      if (![fontSize,lineWidth].every(v=>typeof v === "number" || typeof v === "string")) throw new Error("Text sizes require static numbers or strings");
      textRequest = {content,mode,font_size:String(fontSize),line_width:String(lineWidth)};
      return new Node([{name:"text_src",args:[],other:null}]);
    };
  }
  const serialize = node => node === null ? null : {
    ops:node.ops.map(op=>({
      name:op.name,
      args:op.args.map(arg=>{
        if (arg.type === "parameter") return {type:"parameter",value:serialize(arg.value)};
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
      declaredParameters=new Set();
      saved={parameters:new Map(parameters),callbacks:callbacks.slice(),buffers:buffers.slice(),definitions:new Map(definitions),definitionBytes,
        renderBuffer,all,resolution,filters:filters.slice(),capture,audioRead,
        committedCallbacks:committedCallbacks.slice(),committedUpdate,activeSlots,savedUpdate:update,textRequest,clear:clear.slice(),
        commands:commands.slice(),midiBindings:new Map(midiBindings),envelopes:new Map(envelopes),oscBindings:new Map(oscBindings),
        audioSettings:audioSettings ? JSON.parse(JSON.stringify(audioSettings)) : null};
      saved.regions=regions.slice();saved.regionOutputs=regionOutputs.slice();saved.historyLimit=historyLimit;
      saved.regionSampling={...regionSampling};
      callbacks=callbacks.map((fn,i)=>activeSlots.has(i) ? fn : null);
      capture=false; filters=[null,null,null,null];
      clear=[false,false,false,false];
      commands=[];
    },
    rollback() {
      const oldDefinitions = definitions;
      ({parameters,callbacks,buffers,definitions,definitionBytes,renderBuffer,all,resolution,filters,capture,audioRead,committedCallbacks,committedUpdate,activeSlots,textRequest,clear,commands,midiBindings,envelopes,oscBindings,audioSettings,regions,regionOutputs,historyLimit,regionSampling}=saved);
      globalThis.update=saved.savedUpdate;
      for (const [name,def] of oldDefinitions) {
        if (!definitions.has(name)) {
          if (metadata.has(name)) install(metadata.get(name));
          else if (def.kind === "src") delete globalThis[name];
          else delete Node.prototype[name];
        }
      }
      for(const [name,effect] of nativeEffects) {
        if(!definitions.has(name)) Node.prototype[name]=effect;
      }
      for (const def of definitions.values()) install(def);
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
      return JSON.stringify({parameters:[...parameters.values()].map(entry=>entry.spec),buffers:encodedBuffers,definitions:[...definitions.values()],
        render:renderBuffer,all,resolution,filters,screencap:capture,slots:callbacks.length,audio:audioRead,text:textRequest,clear,
        commands:[...commands,...midiBindings.values(),...envelopes.values(),...oscBindings.values()],regions,region_outputs:regionOutputs,region_sampling:regionSampling,history_limit:historyLimit});
    },
    audio() { return audioRead; },
    setAudioSettings(settings) { editing(); requireFeature("fft"); audioSettings=settings; },
    finishFrame() { running=false; },
    frame(encoded) {
      const input=parseFrame(encoded);
      Object.assign(globalThis,{time:input.time,beat:input.beat,tempo:input.tempo,bpm:input.tempo,
        phase:input.phase,width:input.resolution[0],height:input.resolution[1],
        innerWidth:input.resolution[0],innerHeight:input.resolution[1]});
      mouse.x=input.mouse[0]; mouse.y=input.mouse[1]; fft=input.fft;
      rhythm={onset:input.audio_onset,beat:input.audio_beat,bpm:input.audio_bpm};
      midiValues=input.midi_values ?? midiValues;
      midiEnvelopes=input.midi_envelope ?? midiEnvelopes;
      oscValues=input.osc ?? oscValues;
      regionValues=input.regions;
      running=true;
      if (committedUpdate) committedUpdate(input.time);
      const values=Array(128).fill(0);
      committedCallbacks.forEach((fn,i)=>{
        if (!activeSlots.has(i)) return;
        const v=fn({time,beat,bpm,tempo,phase,mouse,width,height});
        if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`callback ${i} must return a finite number`);
        values[i]=v;
      });
      return values;
    }
  });
})();
