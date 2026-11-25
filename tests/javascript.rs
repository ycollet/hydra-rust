#![cfg(feature = "javascript")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use hydra_rust::RenderMode;
use hydra_rust::javascript::{JavascriptFrame, JavascriptRuntime, JavascriptSettings};

fn runtime() -> JavascriptRuntime {
    JavascriptRuntime::new(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn persistent_context_and_native_emitter() {
    let mut js = runtime();
    let first = js
        .eval("var frequency=7; osc(frequency).rotate(0.25).out(o1); render(o1)")
        .unwrap();
    assert!(matches!(first.render_mode, RenderMode::Single(1)));
    let shader = first.shaders[1].as_ref().unwrap();
    assert!(shader.contains("osc("));
    assert!(shader.contains("7.0"));
    assert!(shader.find("rotate(").unwrap() < shader.find("osc(").unwrap());
    let second = js
        .eval("frequency+=2; solid(frequency,0,0,1).out(o0)")
        .unwrap();
    assert!(second.shaders[0].as_ref().unwrap().contains("9.0"));
    assert_eq!(first.shaders[1], second.shaders[1]);
}

#[test]
fn opt_in_pixel_parameters_and_effect_passes_share_native_codegen() {
    for (javascript, rhai) in [
        ("osc(noise(2).mul(40)).out()", "osc(noise(2) * 40).out()"),
        (
            "osc(10).blur(1).sharpen(1).out()",
            "osc(10).blur(1.0).sharpen(1.0).out()",
        ),
        ("osc(10).sobel().out()", "osc(10).sobel().out()"),
        ("osc(10).emboss().out()", "osc(10).emboss().out()"),
        (
            "osc(10).dilate().erode().out()",
            "osc(10).dilate().erode().out()",
        ),
        (
            "history(o0,8).blend(osc(10),0.1).out()",
            "history(o0,8).blend(osc(10),0.1).out()",
        ),
    ] {
        let result = runtime().eval(javascript).unwrap();
        let native = hydra_rust::eval(rhai).unwrap();
        assert_eq!(result.shaders, native.shaders, "{javascript}");
        assert_eq!(
            result.render_plan.passes, native.render_plan.passes,
            "{javascript}"
        );
        assert_eq!(result.render_plan.history, native.render_plan.history);
    }
    let mut js = runtime();
    let graph = js.eval("osc(10).out()").unwrap();
    for code in [
        "osc(10).blur(33).out()",
        "history(o0,9).out()",
        "history(s0,2).out()",
        "osc(10).blur(()=>1).out()",
        "var effect=osc(10);for(var n=0;n<17;n++)effect=effect.blur();effect.out()",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
        assert_eq!(js.eval("").unwrap().shaders, graph.shaders);
    }
    assert!(
        js.eval("osc(noise(2).blur()).out()")
            .unwrap()
            .render_plan
            .passes
            .len()
            >= 2
    );
    assert!(js.eval("setFunction({name:'blur',type:'color',inputs:[],glsl:'return c0;'});throw new Error('rollback')").is_err());
    assert_eq!(
        js.eval("osc(10).blur().out()")
            .unwrap()
            .render_plan
            .passes
            .len(),
        2
    );
    js.eval(
        "setFunction({name:'blur',type:'color',inputs:[],glsl:'return c0;'});osc(10).blur().out()",
    )
    .unwrap();
    assert!(js.eval("").unwrap().render_plan.passes.is_empty());
    assert_eq!(
        js.eval("osc(10).nativeBlur().out()")
            .unwrap()
            .render_plan
            .passes
            .len(),
        2
    );
}

#[test]
fn regions_remain_explicit_bounded_frame_signals() {
    let mut js = runtime();
    let graph = js
        .eval("var brightness=region(o1,0,0,1,1,0);osc(brightness).out()")
        .unwrap();
    assert_eq!(graph.regions.len(), 1);
    let mut frame = JavascriptFrame::default();
    frame.regions[0] = 0.4;
    assert_eq!(js.frame(frame).unwrap()[0], 0.4);
    assert_eq!(js.eval("").unwrap().regions.len(), 1);
    for code in [
        "region(o0,-1,0,1,1)",
        "region(o0,0,0,2,1)",
        "region(o0,0,0,1,1,5)",
        "region(o0,0,0,0,1)",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
    }
    assert_eq!(js.eval("").unwrap().regions.len(), 1);
    js.eval("update=()=>region(o0)").unwrap();
    assert!(js.frame(frame).unwrap_err().contains("during frame"));
    let native =
        hydra_rust::eval("let level=region(o0,0.0,0.0,1.0,1.0,4);osc(level).out()").unwrap();
    assert!(
        native.shaders[0]
            .as_ref()
            .unwrap()
            .contains("iRegionValues[0]")
    );
}

#[test]
fn history_limit_is_configurable_and_rolls_back_with_the_graph() {
    let mut js = runtime();
    let result = js.eval("setHistoryLimit(4);history(o0,4).out()").unwrap();
    assert_eq!(result.render_plan.history_limit, 4);
    assert!(js.eval("setHistoryLimit(2)").is_err());
    assert_eq!(js.eval("").unwrap().render_plan.history_limit, 4);
    assert!(js.eval("setHistoryLimit(9)").is_err());
    let native = hydra_rust::eval("setHistoryLimit(4);history(o0,4).out()").unwrap();
    assert_eq!(result.shaders, native.shaders);
    assert!(hydra_rust::eval("setHistoryLimit(2);history(o0,4).out()").is_err());
    js.eval("history(o0,1).out();setHistoryLimit(1)").unwrap();
    assert_eq!(js.eval("").unwrap().render_plan.history, [0; 4]);
    assert_eq!(
        js.eval("").unwrap().shaders,
        hydra_rust::eval("src(o0).out()").unwrap().shaders
    );
}

#[test]
fn automatic_pass_limit_spans_all_outputs() {
    let mut js = runtime();
    let valid = js
        .eval("for(var i=0;i<4;i++)osc(10).blur().blur().blur().blur().out([o0,o1,o2,o3][i])")
        .unwrap();
    assert_eq!(valid.render_plan.passes.len(), 32);
    let error = js
        .eval("osc(10).blur().blur().blur().blur().blur().out()")
        .err()
        .unwrap();
    assert!(error.contains("pass limit"), "{error}");
    assert_eq!(js.eval("").unwrap().shaders, valid.shaders);
}

#[cfg(feature = "osc")]
#[test]
fn region_osc_output_is_explicit_rate_limited_and_uses_native_packets() {
    use hydra_rust::extended::RegionSender;
    let listener = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    listener
        .set_read_timeout(Some(std::time::Duration::from_millis(100)))
        .unwrap();
    let destination = listener.local_addr().unwrap().to_string();
    let mut js = runtime();
    let result = js
        .eval(&format!(
            "var level=region(o0);sendRegion(level,{},'/level',30);osc(10).out()",
            serde_json::to_string(&destination).unwrap()
        ))
        .unwrap();
    let mut sender = RegionSender::default();
    sender
        .configure(&result.region_outputs, result.regions.len())
        .unwrap();
    let mut values = [0.0; 16];
    values[0] = 0.75;
    sender.send(&values).unwrap();
    sender.send(&values).unwrap();
    let mut bytes = [0; 1024];
    let (count, _) = listener.recv_from(&mut bytes).unwrap();
    let (_, packet) = rosc::decoder::decode_udp(&bytes[..count]).unwrap();
    if let rosc::OscPacket::Message(message) = packet {
        assert_eq!(message.addr, "/level");
        assert!(matches!(message.args.as_slice(),[rosc::OscType::Float(value)] if *value==0.75));
    } else {
        panic!("Expected OSC message");
    }
    assert!(listener.recv_from(&mut bytes).is_err());
    assert!(
        js.eval("sendRegion(level,'localhost:9000','/level')")
            .is_err()
    );
    assert!(
        js.eval("sendRegion(level,'127.0.0.1:9000','/level',1000)")
            .is_err()
    );
    js.eval("stopRegionOutput()").unwrap();
    assert!(js.eval("").unwrap().region_outputs.is_empty());
}
#[test]
fn native_tileable_and_text_helpers_share_the_native_implementations() {
    for code in [
        "osc(10).tileable().repeat(3,3).out()",
        "osc(10).tileable(0.2).tileable(0.1).out()",
        "text('Hydra').out()",
        "strokeText('Hydra',{fontSize:'50%',lineWidth:'3px'}).out()",
        "fillStrokeText('Hydra',{fontSize:40,lineWidth:2}).out()",
    ] {
        let js = runtime().eval(code).unwrap();
        let native = hydra_rust::eval(code).unwrap();
        assert_eq!(js.shaders, native.shaders, "{code}");
        if let Some(text) = js.text_data {
            let native = native.text_data.unwrap();
            assert_eq!((text.width, text.height), (native.width, native.height));
            assert_eq!(text.pixels, native.pixels);
            assert!(
                text.pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] != 0)
            );
        }
    }
    let mut js = runtime();
    let graph = js.eval("osc(10).tileable(()=>time*0.1).out()").unwrap();
    let slot = callback_slot(graph.shaders[0].as_ref().unwrap());
    assert_eq!(
        js.frame(JavascriptFrame {
            time: 2.0,
            ..Default::default()
        })
        .unwrap()[slot],
        0.2
    );
    for code in [
        "osc(10).tileable(Infinity).out()",
        "text('Hydra',{fontSize:()=>40}).out()",
        "text('Hydra',{fontSize:'bad'}).out()",
        "text('x'.repeat(4097)).out()",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
    }
}

#[test]
fn buffer_clear_is_one_shot_and_rolls_back_with_failed_graphs() {
    let mut js = runtime();
    let original = js.eval("osc(10).out();o1.clear().setNearest()").unwrap();
    assert_eq!(original.buffer_clear, [false, true, false, false]);
    assert_eq!(js.eval("").unwrap().buffer_clear, [false; 4]);
    assert!(js.eval("o0.clear();osc('bad').out()").is_err());
    assert_eq!(js.eval("").unwrap().buffer_clear, [false; 4]);
    js.eval("o2.clear();solid(1).out()").unwrap();
    js.rollback_last_eval().unwrap();
    let restored = js.eval("").unwrap();
    assert_eq!(restored.shaders, original.shaders);
    assert_eq!(restored.buffer_clear, [false; 4]);
    js.eval("update=()=>o0.clear()").unwrap();
    assert!(
        js.frame(JavascriptFrame::default())
            .unwrap_err()
            .contains("during frame")
    );
}

#[cfg(feature = "fft")]
#[test]
fn audio_controls_queue_native_settings_and_preserve_bin_validation() {
    use hydra_rust::audio::{AudioSettings, BinProperty};
    use hydra_rust::eval::AudioRequest;
    let mut js = runtime();
    let mut settings = AudioSettings::default();
    settings.set_bins(6);
    settings.set(Some(5), BinProperty::Scale, 3.0).unwrap();
    js.sync_audio_settings(&settings).unwrap();
    let graph = js
        .eval(
            "if(a.bins[5].scale!==3)throw Error('settings');\
         a.setBins(8).setCutoff(0.1).setScale(2).setSmooth(0.5);\
         a.bins[7].scale=4;a.show();osc(()=>a.fft[7]).out()",
        )
        .unwrap();
    assert!(matches!(graph.audio_requests[0], AudioRequest::SetBins(8)));
    assert!(matches!(
        graph.audio_requests[4],
        AudioRequest::SetBin {
            index: 7,
            property: BinProperty::Scale,
            value: 4.0
        }
    ));
    assert!(js.uses_audio());
    assert_eq!(js.eval("").unwrap().audio_requests.len(), 0);
    assert!(js.eval("a.setBins(2);a.bins[7].scale=1").is_err());
    js.eval("if(a.bins.length!==8 || a.bins[7].scale!==4)throw Error('rollback')")
        .unwrap();
    for code in [
        "a.setBins(0)",
        "a.setBins(9)",
        "a.setBins(2.5)",
        "a.setSmooth(NaN)",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
    }
    js.eval("update=()=>a.setBins(4)").unwrap();
    assert!(
        js.frame(JavascriptFrame::default())
            .unwrap_err()
            .contains("during frame")
    );
}

#[cfg(feature = "midi")]
#[test]
fn midi_controls_bind_native_filtered_values_and_envelopes() {
    use hydra_rust::eval::MidiRequest;
    use hydra_rust::midi::{MidiBridge, MidiPoller};
    let mut js = runtime();
    let graph = js
        .eval(
            "midi.start().show();\
         solid(midi.cc(74,1,'Keys').smooth(1).range(0,2),\
         midi.velocity(60,1,'Keys'),midi.note(60,1,'Keys').adsr(0,0,1,0),1).out()",
        )
        .unwrap();
    assert!(matches!(graph.midi_requests[0], MidiRequest::Start));
    let bridge = MidiBridge::new();
    bridge.register_input(0, "Keys");
    let mut poller = MidiPoller::new(bridge.clone());
    for request in &graph.midi_requests {
        match request {
            MidiRequest::Bind { slot, binding } => bridge.bind(*slot, binding.clone()).unwrap(),
            MidiRequest::FilteredAdsrSlot {
                slot,
                note,
                filter,
                adsr,
            } => bridge.set_filtered_adsr_slot(*slot, *note, filter.clone(), *adsr),
            _ => {}
        }
    }
    bridge.control_change_from(0, 1, 74, 0.25);
    bridge.note_on_from(0, 1, 60, 0.75);
    let midi = poller.poll();
    let frame = JavascriptFrame {
        midi_values: midi.values,
        midi_envelope: midi.envelope,
        ..Default::default()
    };
    assert_eq!(&js.frame(frame).unwrap()[..3], &[0.5, 0.75, 0.75]);
    bridge.control_change_from(0, 2, 74, 1.0);
    let midi = poller.poll();
    assert_eq!(
        js.frame(JavascriptFrame {
            midi_values: midi.values,
            ..frame
        })
        .unwrap()[0],
        0.5
    );
    assert!(
        !js.eval("")
            .unwrap()
            .midi_requests
            .iter()
            .any(|r| matches!(r, MidiRequest::Start))
    );
    for code in [
        "midi.cc(128)",
        "midi.note(60,16)",
        "midi.cc(1).smooth(2)",
        "midi.note(1).adsr(0,0,2,0)",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
    }
    assert_eq!(js.frame(frame).unwrap()[0], 0.5);
}

#[cfg(feature = "osc")]
#[test]
fn osc_controls_bind_deduplicated_native_slots() {
    use hydra_rust::eval::OscRequest;
    let mut js = runtime();
    let graph = js.eval(
        "_osc.start(57102).show();solid(_osc.get('/level',1).scale(2),_osc.get('/level',1),0,1).out()"
    ).unwrap();
    assert!(matches!(graph.osc_requests[0], OscRequest::Start(57102)));
    assert_eq!(
        graph
            .osc_requests
            .iter()
            .filter(|r| matches!(r, OscRequest::Bind { .. }))
            .count(),
        1
    );
    let mut frame = JavascriptFrame::default();
    frame.osc[0] = 0.25;
    assert_eq!(&js.frame(frame).unwrap()[..2], &[0.5, 0.25]);
    for code in ["_osc.start(0)", "_osc.get('bad')", "_osc.get('/x',256)"] {
        assert!(js.eval(code).is_err(), "{code}");
    }
    js.eval("update=()=>_osc.get('/x')").unwrap();
    assert!(js.frame(frame).unwrap_err().contains("during frame"));
}

#[test]
fn sources_only_queue_explicit_feature_gated_requests() {
    for (code, enabled) in [
        ("s2.initCam(1);src(s2).out()", cfg!(feature = "webcam")),
        (
            "s2.initImage('local.png');src(s2).out()",
            cfg!(feature = "image_url"),
        ),
        (
            "s2.initGif('local.gif');src(s2).out()",
            cfg!(feature = "image_url"),
        ),
        (
            "s2.initVideo('local.mp4');src(s2).out()",
            cfg!(feature = "video"),
        ),
        (
            "s2.initStream('127.0.0.1:9000');src(s2).out()",
            cfg!(feature = "stream"),
        ),
    ] {
        let mut js = runtime();
        if enabled {
            let result = js.eval(code).unwrap();
            assert_eq!(
                result.shaders,
                hydra_rust::eval("src(s2).out()").unwrap().shaders
            );
            #[cfg(any(
                feature = "webcam",
                feature = "image_url",
                feature = "video",
                feature = "stream"
            ))]
            {
                assert_eq!(result.source_requests.len(), 1);
                assert!(js.eval("").unwrap().source_requests.is_empty());
            }
        } else {
            assert!(js.eval(code).err().unwrap().contains("requires"));
        }
    }
    #[cfg(feature = "webcam")]
    {
        let mut js = runtime();
        for code in ["s0.initCam(-1)", "s0.initCam(0.5)"] {
            assert!(js.eval(code).is_err());
        }
        js.eval("update=()=>s0.initCam()").unwrap();
        assert!(
            js.frame(JavascriptFrame::default())
                .unwrap_err()
                .contains("during frame")
        );
    }
}

#[test]
fn callbacks_and_update_run_once_per_frame_without_shader_rebuild() {
    let mut js = runtime();
    let graph = js
        .eval("var count=0; update=()=>count++; osc(()=>time+count).out()")
        .unwrap();
    assert!(graph.shaders[0].as_ref().unwrap().contains("iJsValues[0]"));
    let a = js
        .frame(JavascriptFrame {
            time: 2.0,
            ..Default::default()
        })
        .unwrap();
    let b = js
        .frame(JavascriptFrame {
            time: 4.0,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(a[0], 3.0);
    assert_eq!(b[0], 6.0);
    let unchanged = js.eval("").unwrap();
    assert_eq!(graph.shaders[0], unchanged.shaders[0]);
}

#[test]
fn old_buffers_keep_their_callback_slots() {
    let mut js = runtime();
    let first = js.eval("osc(()=>time).out(o0)").unwrap();
    let second = js.eval("solid(()=>time*2).out(o1)").unwrap();
    assert_eq!(first.shaders[0], second.shaders[0]);
    assert!(second.shaders[1].as_ref().unwrap().contains("iJsValues[1]"));
    let values = js
        .frame(JavascriptFrame {
            time: 3.0,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(&values[..2], &[3.0, 6.0]);
}

#[test]
fn callbacks_from_overwritten_graphs_stop_running() {
    let mut js = runtime();
    js.eval("osc(()=>{throw Error('obsolete')}).out()").unwrap();
    js.eval("solid(()=>time).out()").unwrap();
    let values = js
        .frame(JavascriptFrame {
            time: 3.0,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(&values[..2], &[0.0, 3.0]);
}

fn callback_slot(shader: &str) -> usize {
    shader
        .split("iJsValues[")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn reevaluations_reuse_only_inactive_slots_and_preserve_rollback() {
    let mut js = runtime();
    let pinned = js.eval("solid(()=>1000).out(o1)").unwrap().shaders[1].clone();
    let input = JavascriptFrame {
        time: 5.0,
        ..Default::default()
    };
    let mut last = None;
    for value in 0..260 {
        let graph = js.eval(&format!("osc(()=>time+{value}).out(o0)")).unwrap();
        assert_eq!(graph.shaders[1], pinned);
        let slot = callback_slot(graph.shaders[0].as_ref().unwrap());
        assert!((1..=2).contains(&slot));
        let values = js.frame(input).unwrap();
        assert_eq!(values[0], 1000.0);
        assert_eq!(values[slot], 5.0 + value as f32);
        last = Some(graph.shaders[0].clone());
    }
    let original = last.unwrap();
    let original_slot = callback_slot(original.as_ref().unwrap());
    let reused = js.eval("osc(()=>999).out(o0)").unwrap();
    let reused_slot = callback_slot(reused.shaders[0].as_ref().unwrap());
    assert_ne!(original_slot, reused_slot);
    js.rollback_last_eval().unwrap();
    let values = js.frame(input).unwrap();
    assert_eq!(values[original_slot], 264.0);
    assert_eq!(values[reused_slot], 0.0);
    assert_eq!(js.eval("").unwrap().shaders[0], original);
    assert!(
        js.eval("osc(()=>888).out(o0);throw Error('failed reuse')")
            .is_err()
    );
    assert_eq!(js.frame(input).unwrap()[original_slot], 264.0);
    let next = js.eval("osc(()=>777).out(o0)").unwrap();
    assert_eq!(
        callback_slot(next.shaders[0].as_ref().unwrap()),
        reused_slot
    );
    assert_eq!(js.frame(input).unwrap()[reused_slot], 777.0);
}

#[test]
fn saved_nodes_restore_their_callbacks_after_slot_reuse() {
    let mut js = runtime();
    js.eval("var stored=osc(()=>11);stored.out()").unwrap();
    js.eval("osc(()=>22).out()").unwrap();
    js.eval("osc(()=>33).out()").unwrap();
    let graph = js.eval("stored.out()").unwrap();
    let slot = callback_slot(graph.shaders[0].as_ref().unwrap());
    assert_eq!(js.frame(JavascriptFrame::default()).unwrap()[slot], 11.0);
}

#[test]
fn frame_failures_cancel_queued_jobs_before_the_next_frame() {
    for (hook, failure) in [
        (
            "update=()=>{if(calls++===0){again();throw Error('frame failed')}}",
            "frame failed",
        ),
        ("update=()=>{if(calls++===0)again()}", "job limit"),
        (
            "callback=()=>{if(calls++===0){again();throw Error('callback failed')}return calls}",
            "callback failed",
        ),
    ] {
        let mut js = runtime();
        js.eval(&format!(
            "var calls=0;function again(){{Promise.resolve().then(again)}};\
             var callback=()=>calls;{hook};osc(()=>callback()).out()"
        ))
        .unwrap();
        assert!(
            js.frame(JavascriptFrame::default())
                .unwrap_err()
                .contains(failure)
        );
        assert_eq!(js.frame(JavascriptFrame::default()).unwrap()[0], 2.0);
    }
}

#[test]
fn failed_eval_and_renderer_rollback_preserve_callbacks_and_update() {
    let mut js = runtime();
    let original = js
        .eval("var n=0;update=()=>n++; osc(()=>time+n).out()")
        .unwrap();
    assert!(
        js.eval("update=()=>{n+=100};solid(()=>99).out();throw Error('broken')")
            .err()
            .unwrap()
            .contains("broken")
    );
    assert_eq!(
        js.frame(JavascriptFrame {
            time: 2.0,
            ..Default::default()
        })
        .unwrap()[0],
        3.0
    );
    assert_eq!(js.eval("").unwrap().shaders[0], original.shaders[0]);
    js.eval("solid(()=>55).out()").unwrap();
    js.rollback_last_eval().unwrap();
    assert_eq!(js.eval("").unwrap().shaders[0], original.shaders[0]);
    assert!(js.eval("setFunction({name:'bad',type:'src',inputs:[{name:'st',type:'float'}],glsl:'return vec4(1.);'});bad().out()").is_err());
    assert_eq!(js.eval("").unwrap().shaders[0], original.shaders[0]);
}

#[test]
fn custom_functions_cover_aliases_and_many_defaults() {
    let mut js = runtime();
    let result = js.eval(r#"
      setFunction({name:"customSource",type:"src",inputs:Array.from({length:7},(_,i)=>({name:"p"+i,type:"float",default:i})),
        glsl:"return vec4(_st / resolution, st.x+p6+time, 1.0);"});
      setFunction({name:"customCoord",type:"coord",glsl:"return _st + st;"});
      setFunction({name:"customColor",type:"color",glsl:"return _c0 * c0;"});
      setFunction({name:"customCombine",type:"combine",glsl:"return _c0 + _c1 + c0 + c1;"});
      setFunction({name:"customMod",type:"combineCoord",glsl:"return _st + _c0.xy + st + c0.xy;"});
      customSource().customCoord().customColor().customCombine(solid()).customMod(osc()).out();
    "#).unwrap();
    let shader = result.shaders[0].as_ref().unwrap();
    for alias in [
        "vec2 st = _st",
        "vec4 c0 = _c0",
        "vec4 c1 = _c1",
        "float hydraJsInput6",
    ] {
        assert!(shader.contains(alias), "{shader}");
    }
    assert!(shader.contains("0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0"));
    assert!(shader.contains("_st / iResolution"));
    assert!(shader.contains("st.x+hydraJsInput6+iTime"));
}

#[test]
fn underscore_names_use_safe_shader_identifiers_and_local_parameter_aliases() {
    let mut js = runtime();
    let graph = js.eval(
        "setFunction({name:'_glsl_ext_NODE_0',type:'src',inputs:[{name:'__value',type:'float',default:0.25}],\
         glsl:'/* vec2 st; */ vec2 uv = _st, st = uv * 2.0; return vec4(st,__value,1.0);'});\
         _glsl_ext_NODE_0().out()"
    ).unwrap();
    let shader = graph.shaders[0].as_ref().unwrap();
    assert!(shader.contains("hydraJsFunction0"));
    assert!(shader.contains("hydraJsInput0"));
    assert!(!shader.contains("vec2 st = _st"));
    assert!(!shader.contains("__value"));
    for name in ["gl_bad", "bad-name", "1bad"] {
        assert!(
            js.eval(&format!(
                "setFunction({{name:{},type:'src',glsl:'return vec4(1.0);'}})",
                serde_json::to_string(name).unwrap()
            ))
            .is_err()
        );
    }
}

#[test]
fn large_registries_emit_only_functions_used_by_each_output() {
    let mut js = runtime();
    let graph = js.eval(
        "for(var i=0;i<600;i++)setFunction({name:'_entry'+i,type:'color',glsl:'return _c0 * '+i+'.0;'});\
         setFunction({name:'unusedBroken',type:'src',glsl:'return invalid;'});\
         osc(10)._entry599().out(o0);solid(1)._entry1().out(o1)"
    ).unwrap();
    let first = graph.shaders[0].as_ref().unwrap();
    let second = graph.shaders[1].as_ref().unwrap();
    assert!(first.contains("hydraJsFunction599("));
    assert!(!first.contains("hydraJsFunction1("));
    assert!(second.contains("hydraJsFunction1("));
    assert!(!second.contains("hydraJsFunction599("));
    assert!(!first.contains("invalid"));
    assert!(
        js.eval("unusedBroken().out(o2)").unwrap().shaders[2]
            .as_ref()
            .unwrap()
            .contains("invalid")
    );
    js.rollback_last_eval().unwrap();
    assert_eq!(js.eval("").unwrap().shaders, graph.shaders);
    let error = js.eval("for(var i=600;i<4097;i++)setFunction({name:'_entry'+i,type:'color',glsl:'return _c0;'});")
        .err().unwrap();
    assert!(error.contains("custom function limit"), "{error}");
    assert_eq!(js.eval("").unwrap().shaders, graph.shaders);
}

#[test]
fn sampler_defaults_and_explicit_native_buffers_remain_type_checked() {
    let mut js = runtime();
    let graph = js.eval(
        "setFunction({name:'_sample',type:'src',inputs:[{name:'_tex',type:'sampler2D',default:o2}],\
         glsl:'return texture2D(_tex,_st);'});\
         _sample().out(o0);_sample(s3).out(o1)"
    ).unwrap();
    assert!(
        graph.shaders[0]
            .as_ref()
            .unwrap()
            .contains("hydraJsFunction0(st, iBuffer2)")
    );
    assert!(
        graph.shaders[1]
            .as_ref()
            .unwrap()
            .contains("hydraJsFunction0(st, iSource3)")
    );
    assert!(
        graph.shaders[0]
            .as_ref()
            .unwrap()
            .contains("texture(hydraJsInput0,_st)")
    );
    for code in [
        "_sample(1).out()",
        "_sample(()=>o0).out()",
        "_sample(osc(1)).out()",
        "setFunction({name:'badSampler',type:'src',inputs:[{name:'tex',type:'sampler2D',default:1}],glsl:'return vec4(1.0);'})",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
    }
    assert_eq!(js.eval("").unwrap().shaders, graph.shaders);
}

#[test]
fn transforms_metadata_can_be_copied_and_edited_without_changing_the_original_graph() {
    let mut js = runtime();
    let graph = js.eval(
        "var original=osc(10).rotate(0.2);\
         var copy=Object.assign({},original);Object.setPrototypeOf(copy,Object.getPrototypeOf(original));\
         copy.transforms=Array.from(original.transforms);\
         copy.transforms[0]={...copy.transforms[0],userArgs:[7,0.1,0]};\
         if(original.transforms[0].transform.name!=='osc')throw Error('metadata');\
         if(original.transforms[0].userArgs[0]!==10)throw Error('copy changed original');\
         var pattern=[10,20];var patterned=osc(pattern);\
         if(patterned.transforms[0].userArgs[0]!==pattern)throw Error('pattern metadata');\
         patterned.out(o2);\
         original.out(o0);copy.brightness(0.5).out(o1)"
    ).unwrap();
    assert_eq!(
        graph.shaders[0],
        hydra_rust::eval("osc(10).rotate(0.2).out()")
            .unwrap()
            .shaders[0]
    );
    assert_eq!(
        graph.shaders[1],
        hydra_rust::eval("osc(7).rotate(0.2).brightness(0.5).out(o1)")
            .unwrap()
            .shaders[1]
    );
}

#[test]
fn custom_functions_can_replace_native_functions_and_rollback_restores_them() {
    let mut js = runtime();
    let original = js.eval("osc(7).out()").unwrap();
    let replaced = js
        .eval(
            "setFunction({name:'osc',type:'src',\
             inputs:[{name:'value',type:'float',default:0.25}],\
             glsl:'return vec4(value);'});osc().out()",
        )
        .unwrap();
    let shader = replaced.shaders[0].as_ref().unwrap();
    assert!(shader.contains("vec4 hydraJsFunction0("));
    assert!(shader.contains("hydraJsFunction0(st, 0.25)"));
    js.rollback_last_eval().unwrap();
    assert_eq!(js.eval("").unwrap().shaders[0], original.shaders[0]);
    let restored = js.eval("osc(8,0.1,0).out()").unwrap();
    assert!(
        !restored.shaders[0]
            .as_ref()
            .unwrap()
            .contains("hydraJsFunction")
    );
    assert!(
        js.eval(
            "setFunction({name:'osc',type:'coord',inputs:[],glsl:'return _st;'});\
             throw Error('failed override')",
        )
        .is_err()
    );
    assert!(js.eval("osc(9).out()").is_ok());
}

#[test]
fn library_extensions_mutate_array_and_node_prototypes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/javascript");
    let mut js = JavascriptRuntime::new(root.clone()).unwrap();
    let script = std::fs::read_to_string(root.join("live.js")).unwrap();
    let graph = js.eval(&script).unwrap();
    assert!(
        graph.shaders[0]
            .as_ref()
            .unwrap()
            .contains("sin((_st.x + _st.y) * hydraJsInput0)")
    );
    let first = js
        .frame(JavascriptFrame {
            time: 0.0,
            tempo: 120.0,
            ..Default::default()
        })
        .unwrap();
    let next = js
        .frame(JavascriptFrame {
            time: 1.0,
            tempo: 120.0,
            ..Default::default()
        })
        .unwrap();
    assert_ne!(first, next);
}

struct LibraryRoot(PathBuf);

impl LibraryRoot {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "javascript-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, code: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
}
impl Drop for LibraryRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reset_recovers_from_exhaustion_and_rejects_invalid_limits_without_losing_state() {
    let mut js = runtime();
    js.eval("var retained=7;osc(retained).out()").unwrap();
    let invalid = JavascriptSettings {
        execution_ms: 0,
        ..Default::default()
    };
    assert!(js.reset(invalid).unwrap_err().contains("between"));
    assert!(js.eval("osc(retained).out()").is_ok());
    assert!(
        js.eval("var exhausted=[];while(true)exhausted.push(new Array(100000).fill(1))")
            .is_err()
    );
    js.reset(JavascriptSettings::default()).unwrap();
    assert!(js.eval("if(typeof retained!=='undefined')throw Error('state survived reset');osc(()=>time).out()").is_ok());
    assert_eq!(
        js.frame(JavascriptFrame {
            time: 2.0,
            ..Default::default()
        })
        .unwrap()[0],
        2.0
    );
}

#[test]
fn configured_script_library_and_promise_limits_are_enforced() {
    let root = LibraryRoot::new();
    root.write("large.js", &" ".repeat(1025));
    let settings = JavascriptSettings {
        execution_ms: 20,
        script_kib: 1,
        library_kib: 1,
        pending_jobs: 1,
        ..Default::default()
    };
    let mut js = JavascriptRuntime::with_settings(root.0.clone(), settings).unwrap();
    assert!(
        js.eval(&" ".repeat(1025))
            .err()
            .unwrap()
            .contains("byte limit")
    );
    assert!(
        js.eval("loadScript('large.js')")
            .err()
            .unwrap()
            .contains("byte limit")
    );
    assert!(js.eval("while(true){}").is_err());
    js.eval("Promise.resolve().then(()=>osc(3).out())").unwrap();
    assert!(
        js.eval("Promise.resolve().then(()=>{}).then(()=>{})")
            .err()
            .unwrap()
            .contains("job limit")
    );
    let invalid = JavascriptSettings {
        library_kib: 2048,
        loaded_mib: 1,
        ..Default::default()
    };
    assert!(JavascriptRuntime::with_settings(root.0.clone(), invalid).is_err());
}

#[test]
fn classic_scripts_and_modules_share_the_total_loaded_text_limit() {
    let root = LibraryRoot::new();
    root.write("classic.js", &" ".repeat(512 * 1024));
    root.write(
        "module.mjs",
        &format!("export const value=1;{}", " ".repeat(512 * 1024 - 21)),
    );
    let settings = JavascriptSettings {
        library_kib: 512,
        loaded_mib: 1,
        ..Default::default()
    };
    let mut js = JavascriptRuntime::with_settings(root.0.clone(), settings).unwrap();
    js.eval("loadScript('classic.js')").unwrap();
    js.eval("import {value} from './module.mjs';osc(value).out()")
        .unwrap();
    assert!(
        js.eval("loadScript('classic.js')")
            .err()
            .unwrap()
            .contains("runtime byte limit")
    );
}

#[test]
fn named_library_errors_and_console_logging_use_native_diagnostics() {
    struct Capture;
    static CAPTURE: Capture = Capture;
    static INITIALIZE: std::sync::Once = std::sync::Once::new();
    static MESSAGES: std::sync::Mutex<Vec<(log::Level, String)>> =
        std::sync::Mutex::new(Vec::new());
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target() == "hydra_rust::javascript" {
                MESSAGES
                    .lock()
                    .unwrap()
                    .push((record.level(), record.args().to_string()));
            }
        }
        fn flush(&self) {}
    }
    INITIALIZE.call_once(|| {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
    let root = LibraryRoot::new();
    root.write("sub/one.js", "loadScript('two.js')");
    root.write("sub/two.js", "\nthrow Error('library failure')");
    let mut js = JavascriptRuntime::new(root.0.clone()).unwrap();
    js.eval("console.log('diagnostics-check',3,null);console.warn('diagnostics-warning');console.error('diagnostics-error')").unwrap();
    let messages = MESSAGES.lock().unwrap();
    assert!(messages.contains(&(log::Level::Info, "diagnostics-check 3 null".into())));
    assert!(messages.contains(&(log::Level::Warn, "diagnostics-warning".into())));
    assert!(messages.contains(&(log::Level::Error, "diagnostics-error".into())));
    drop(messages);
    let error = js.eval("loadScript('sub/one.js')").err().unwrap();
    assert!(error.contains("sub/two.js:2"), "{error}");
    let error = js
        .eval_named("throw Error('sketch failure')", &root.0.join("main.js"))
        .err()
        .unwrap();
    assert!(error.contains("main.js:1"), "{error}");
    root.write(
        "bad.mjs",
        "\nthrow Error('module failure');export const value=1;",
    );
    let error = js
        .eval("import {value} from './bad.mjs';osc(value).out()")
        .err()
        .unwrap();
    assert!(error.contains("bad.mjs:2"), "{error}");
}

#[test]
fn local_modules_support_relative_imports_caching_and_reset() {
    let root = LibraryRoot::new();
    root.write("lib/value.mjs", "export const offset=1");
    root.write("lib/pulse.js", "import {offset} from './value.mjs';globalThis.moduleLoads=(globalThis.moduleLoads||0)+1;export function pulse(t){return t+offset}");
    let mut js = JavascriptRuntime::new(root.0.clone()).unwrap();
    let code = "import {pulse} from './lib/pulse.js';osc(()=>pulse(time)).out()";
    for _ in 0..2 {
        let graph = js.eval(code).unwrap();
        let slot = callback_slot(graph.shaders[0].as_ref().unwrap());
        assert_eq!(
            js.frame(JavascriptFrame {
                time: 2.0,
                ..Default::default()
            })
            .unwrap()[slot],
            3.0
        );
    }
    js.eval("if(moduleLoads!==1)throw Error('module was not cached')")
        .unwrap();
    root.write("lib/value.mjs", "export const offset=9");
    js.reset(JavascriptSettings::default()).unwrap();
    js.eval(code).unwrap();
    assert_eq!(
        js.frame(JavascriptFrame {
            time: 2.0,
            ..Default::default()
        })
        .unwrap()[0],
        11.0
    );
    js.eval(
        "var globalState=7;var text='export import';var object={export:1};osc(globalState).out()",
    )
    .unwrap();
    assert!(js.eval("osc(globalState).out()").is_ok());
    for _ in 0..2 {
        js.eval_module_named(
            "let sceneValue=3;osc(sceneValue).out()",
            &root.0.join("scene.js"),
        )
        .unwrap();
    }
    js.eval("if(typeof sceneValue!=='undefined')throw Error('module scope leaked')")
        .unwrap();
    js.eval("var pattern=/export {example} import/;let lexicalState=9;osc(lexicalState).out()")
        .unwrap();
    assert!(
        js.eval("let lexicalState=10;osc(lexicalState).out()")
            .is_err()
    );
    assert!(
        js.eval("osc(lexicalState).out()").unwrap().shaders[0]
            .as_ref()
            .unwrap()
            .contains("9.0")
    );
    js.eval("var runs=0;runs++;import /* comment */ ('./lib/value.mjs').then(()=>osc(runs).out())")
        .unwrap();
    assert!(
        js.eval("osc(runs).out()").unwrap().shaders[0]
            .as_ref()
            .unwrap()
            .contains("1.0")
    );
    js.eval_named(
        "const value=await Promise.resolve(3);osc(value).out()",
        &root.0.join("await.mjs"),
    )
    .unwrap();
    assert!(
        js.eval_named("await new Promise(()=>{});", &root.0.join("pending.mjs"))
            .err()
            .unwrap()
            .contains("unresolved")
    );
    assert!(
        js.eval("osc(4).out()")
            .err()
            .unwrap()
            .contains("reset the interpreter")
    );
    js.reset(JavascriptSettings::default()).unwrap();
    assert!(js.eval("osc(4).out()").is_ok());
}

#[test]
fn module_imports_reject_network_packages_escaping_paths_and_unsupported_files() {
    let root = LibraryRoot::new();
    let outside = LibraryRoot::new();
    outside.write("outside.mjs", "export const value=1");
    root.write("value.json", "{\"value\":1}");
    let mut js = JavascriptRuntime::new(root.0.clone()).unwrap();
    let original = js.eval("osc(()=>time).out()").unwrap();
    for code in [
        "import 'https://example.com/library.js'",
        "import 'package-name'",
        "import '../../../Cargo.toml'",
        "import './missing.js'",
        "import './value.json'",
    ] {
        assert!(js.eval(code).is_err(), "{code}");
        assert_eq!(js.eval("").unwrap().shaders[0], original.shaders[0]);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.0.join("outside.mjs"), root.0.join("escape.mjs"))
            .unwrap();
        assert!(
            js.eval("import './escape.mjs'")
                .err()
                .unwrap()
                .contains("escapes")
        );
    }
}

#[test]
fn local_loading_is_relative_bounded_and_rejects_cycles_and_escape() {
    let root = LibraryRoot::new();
    root.write("sub/one.js", "loadScript('two.js');");
    root.write("sub/two.js", "globalThis.libraryNumber=8;");
    root.write("cycle.js", "loadScript('cycle.js')");
    root.write("bad.js", "throw Error('library failed')");
    root.write("large.js", &" ".repeat(1024 * 1024 + 1));
    let mut js = JavascriptRuntime::new(root.0.clone()).unwrap();
    let graph = js
        .eval("loadScript('sub/one.js');osc(libraryNumber).out()")
        .unwrap();
    assert!(graph.shaders[0].as_ref().unwrap().contains("8.0"));
    for (code, error) in [
        ("loadScript('cycle.js')", "cycle"),
        ("loadScript('../../Cargo.toml')", "escapes"),
        ("loadScript('missing.js')", "loadScript"),
        ("loadScript('bad.js')", "library failed"),
        ("loadScript('large.js')", "limit"),
    ] {
        assert!(js.eval(code).err().unwrap().contains(error), "{code}");
        assert_eq!(js.eval("").unwrap().shaders[0], graph.shaders[0]);
    }
}

#[test]
fn interrupts_and_callback_limits_preserve_previous_graph() {
    let mut js = runtime();
    let graph = js.eval("osc(()=>time).out()").unwrap();
    assert!(js.eval("while(true){}").is_err());
    assert_eq!(js.eval("").unwrap().shaders[0], graph.shaders[0]);
    assert!(
        js.eval("for(let i=0;i<130;i++)solid(()=>i).out()")
            .err()
            .unwrap()
            .contains("slot limit")
    );
    assert_eq!(js.eval("").unwrap().shaders[0], graph.shaders[0]);
    assert!(js.eval("function recurse(){recurse()} recurse()").is_err());
    assert!(
        js.eval("let oversized=[];while(true)oversized.push(new Array(100000).fill(1))")
            .is_err()
    );
    assert_eq!(
        js.frame(JavascriptFrame {
            time: 7.0,
            ..Default::default()
        })
        .unwrap()[0],
        7.0
    );
    assert!(js.eval(&" ".repeat(1024 * 1024 + 1)).is_err());
}

#[test]
fn frame_inputs_audio_errors_and_explicit_unsupported_methods() {
    let mut js = runtime();
    js.eval("osc(()=>a.fft[2]+a.onset+a.beat+a.bpm+mouse.x+width+beat+phase).out()")
        .unwrap();
    let frame = JavascriptFrame {
        resolution: [10.0, 20.0],
        mouse: [3.0, 4.0],
        beat: 5.0,
        phase: 6.0,
        fft: [0.0, 0.0, 7.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        audio_onset: 8.0,
        audio_beat: 9.0,
        audio_bpm: 11.0,
        ..Default::default()
    };
    assert_eq!(js.frame(frame).unwrap()[0], 59.0);
    assert!(js.uses_audio());
    js.eval("osc(()=>{throw Error('callback failed')}).out()")
        .unwrap();
    assert!(js.frame(frame).unwrap_err().contains("callback failed"));
    js.eval("osc(()=>NaN).out()").unwrap();
    assert!(js.frame(frame).unwrap_err().contains("finite"));
    assert!(js.eval("osc('not a number').out()").is_err());
    assert!(
        js.eval("new Node([{name:'osc',args:[{type:'injection',value:'evil'}],other:null}]).out()")
            .is_err()
    );
    for code in [
        "s0.initScreen()",
        "midi.value()",
        "fetch('https://example.com')",
    ] {
        assert!(js.eval(code).err().unwrap().contains("unsupported"));
    }
    for (code, enabled) in [
        ("s0.initCam()", cfg!(feature = "webcam")),
        ("a.setBins(4)", cfg!(feature = "fft")),
        ("midi.start()", cfg!(feature = "midi")),
        ("_osc.start()", cfg!(feature = "osc")),
    ] {
        if enabled {
            js.eval(code).unwrap();
        } else {
            assert!(js.eval(code).err().unwrap().contains("requires"));
        }
    }
}

#[test]
fn promise_jobs_cannot_change_graphs_during_frames() {
    let mut js = runtime();
    let original = js
        .eval("update=()=>Promise.resolve().then(()=>solid(1).out());osc(()=>time).out()")
        .unwrap();
    assert!(
        js.frame(JavascriptFrame::default())
            .unwrap_err()
            .contains("graph changes during frame")
    );
    assert_eq!(
        js.eval("update=()=>{}").unwrap().shaders[0],
        original.shaders[0]
    );
    assert_eq!(
        js.frame(JavascriptFrame {
            time: 3.0,
            ..Default::default()
        })
        .unwrap()[0],
        3.0
    );
}

#[test]
fn promises_run_bounded_jobs_and_report_rejections() {
    let mut js = runtime();
    let graph = js.eval("Promise.resolve().then(()=>osc(3).out())").unwrap();
    assert!(graph.shaders[0].as_ref().unwrap().contains("3.0"));
    assert!(
        js.eval("Promise.resolve().then(()=>{throw Error('async failure')})")
            .err()
            .unwrap()
            .contains("async failure")
    );
    js.eval("Promise.reject(Error('handled')).catch(()=>{});")
        .unwrap();
    assert!(
        js.eval("Promise.resolve().then(()=>{while(true){}})")
            .is_err()
    );
    js.frame(JavascriptFrame::default()).unwrap();
    assert!(
        js.eval("function again(){Promise.resolve().then(again)} again()")
            .err()
            .unwrap()
            .contains("job limit")
    );
    assert_eq!(js.frame(JavascriptFrame::default()).unwrap()[0], 0.0);
    assert!(
        js.eval("(async()=>{while(true)await Promise.resolve()})()")
            .err()
            .unwrap()
            .contains("job limit")
    );
    js.frame(JavascriptFrame::default()).unwrap();
    js.eval("var loaded=false; (async()=>{await loadScript('examples/javascript/libraries/helpers.js');loaded=true;osc(loaded?8:0).out();})()").unwrap();
}
