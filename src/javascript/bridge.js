(() => {
  "use strict";
  const metadata = new Map(__metadata.map(m => [m.name, m]));
  delete globalThis.__metadata;
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
  let saved, committedUpdate = null, audioRead = false, running = false;
  let activeSlots = new Set(), pendingActiveSlots = new Set();
  const callbackFunctions = new WeakMap();
  let renderBuffer = null, all = false, resolution = null, filters = [null,null,null,null], capture = false;
  let textRequest = null, clear = [false,false,false,false];
  let commands = [], midiBindings = new Map(), envelopes = new Map(), oscBindings = new Map();
  let midiValues = Array(64).fill(0), midiEnvelopes = Array(16).fill(0), oscValues = Array(32).fill(0);
  let audioSettings = features.fft ? __audioSettings : null;
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
    tileable(width = 0.15) {
      editing();
      return new Node([{name:"tileable",args:[value(width)],other:this}]);
    }
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
    if (["src","ext_src","text_src","tileable","text","strokeText","fillStrokeText","strokeFillText","out","constructor"].includes(def.name)) throw new Error("cannot replace graph infrastructure");
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
        committedCallbacks:committedCallbacks.slice(),committedUpdate,activeSlots,savedUpdate:update,textRequest,clear:clear.slice(),
        commands:commands.slice(),midiBindings:new Map(midiBindings),envelopes:new Map(envelopes),oscBindings:new Map(oscBindings),
        audioSettings:audioSettings ? JSON.parse(JSON.stringify(audioSettings)) : null};
      callbacks=callbacks.map((fn,i)=>activeSlots.has(i) ? fn : null);
      capture=false; filters=[null,null,null,null];
      clear=[false,false,false,false];
      commands=[];
    },
    rollback() {
      const oldDefinitions = definitions;
      ({callbacks,buffers,definitions,renderBuffer,all,resolution,filters,capture,audioRead,committedCallbacks,committedUpdate,activeSlots,textRequest,clear,commands,midiBindings,envelopes,oscBindings,audioSettings}=saved);
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
        render:renderBuffer,all,resolution,filters,screencap:capture,slots:callbacks.length,audio:audioRead,text:textRequest,clear,
        commands:[...commands,...midiBindings.values(),...envelopes.values(),...oscBindings.values()]});
    },
    audio() { return audioRead; },
    setAudioSettings(settings) { editing(); requireFeature("fft"); audioSettings=settings; },
    finishFrame() { running=false; },
    frame(input) {
      Object.assign(globalThis,{time:input.time,beat:input.beat,tempo:input.tempo,bpm:input.tempo,
        phase:input.phase,width:input.resolution[0],height:input.resolution[1],
        innerWidth:input.resolution[0],innerHeight:input.resolution[1]});
      mouse.x=input.mouse[0]; mouse.y=input.mouse[1]; fft=input.fft;
      rhythm={onset:input.audio_onset,beat:input.audio_beat,bpm:input.audio_bpm};
      midiValues=input.midi_values ?? midiValues;
      midiEnvelopes=input.midi_envelope ?? midiEnvelopes;
      oscValues=input.osc ?? oscValues;
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
