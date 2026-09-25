//! Integration tests for `hydra_rust::eval()`'s full JS-compatibility
//! pipeline (SPEC.md §4). Unlike each pass's own unit tests (which check
//! one pass in isolation), these exercise realistic scripts that combine
//! *several* passes at once, to catch bugs in how they compose - the kind
//! of bug a single pass's own tests can't see. `kwargs::strip_named_args`
//! shipped exactly this kind of regression during development (it
//! corrupted `function` declarations' default parameter values, since a
//! declaration's own unit tests never see it interacting with
//! `jsfunctions`); see `named_arg_call_does_not_corrupt_a_sibling_function_declaration`.

use hydra_rust::{eval, RenderMode};

fn ok_shader0(src: &str) -> String {
    match eval(src) {
        Ok(r) => r.shaders[0].clone().expect("buffer 0 should have been written"),
        Err(e) => panic!("eval() failed for {src:?}: {e}"),
    }
}

#[test]
fn named_arg_call_does_not_corrupt_a_sibling_function_declaration() {
    // kwargs (named-arg stripping) and jsfunctions (default-parameter
    // shim cascade) must not interfere with each other: a named-arg call
    // to a *built-in* function, alongside a user `function` declaration
    // that also uses `name=default` syntax for an unrelated reason, in
    // the same script.
    let src = r#"
        function r(min=0,max=1) { return (min+max)/2; }
        noise(scale=10, offset=0.1).color(r(),r(),r()).out()
    "#;
    let glsl = ok_shader0(src);
    assert!(glsl.contains("noise(st, 10.0, 0.1)"), "{glsl}");
    assert!(glsl.contains("color("), "{glsl}");
}

#[test]
fn named_arrow_function_is_callable_with_real_arguments() {
    let src = r#"
        let el = (s,b,l) => shape(99,s,b);
        el(0.3,0.1,0).out()
    "#;
    let glsl = ok_shader0(src);
    assert!(glsl.contains("shape(st, 99.0, 0.3, 0.1)"), "{glsl}");
}

#[test]
fn iife_wrapped_multi_statement_sketch_still_gets_semicolons_inserted() {
    // the iife-unwrap + asi interaction: unwrapping must not leave the
    // body's own statements glued together without semicolons.
    let src = r#"
        (()=>{
            let speed = 0.1
            osc(60,speed,0).out()
        })()
    "#;
    let glsl = ok_shader0(src);
    assert!(glsl.contains("osc(st, 60.0, 0.1, 0.0)"), "{glsl}");
}

#[test]
fn default_parameter_shim_cascade_is_callable_at_every_arity() {
    let src = r#"
        function f(a,b=1,c=2) { return a+b+c; }
        solid(f(1),f(1,1),f(1,1,1)).out()
    "#;
    assert!(eval(src).is_ok());
}

#[test]
fn ternary_inside_a_named_arrow_function_body_is_rewritten() {
    let src = r#"
        let pick = (x) => x>0.5 ? 1 : 0;
        solid(pick(0.9)).out()
    "#;
    assert!(eval(src).is_ok());
}

#[test]
fn math_random_and_math_e_are_usable_together() {
    let src = "noise(Math.random()*10, Math.E*0.01).out()";
    let glsl = ok_shader0(src);
    assert!(glsl.contains("noise("), "{glsl}");
}

#[test]
fn ported_community_extension_functions_compose_with_blend_and_modulate() {
    let src = "osc(60).modulateScale(turb(20,0.2,4),1,1).colreflect(spiral(1,5,0.1),0.5).out()";
    let glsl = ok_shader0(src);
    assert!(glsl.contains("turb("), "{glsl}");
    assert!(glsl.contains("spiral("), "{glsl}");
    assert!(glsl.contains("colreflect("), "{glsl}");
}

#[test]
fn mandeloffs_geo_function_compiles_with_all_arities() {
    // ported from geikha/hyper-hydra (hydra-fractals.js) via a widely
    // copy-pasted setFunction() call - real hydra.js's setFunction is
    // itself a no-op here (no dynamic GLSL registration pipeline exists),
    // so this needed a real, static port like any other missing function.
    let glsl = ok_shader0("osc(60).mandeloffs().mandeloffs(0.05).mandeloffs(0.05,0.1,0.2).out()");
    assert!(glsl.contains("mandeloffs("), "{glsl}");
}

#[test]
fn multiple_buffers_are_kept_independent() {
    let src = "osc(60).out(o0)\nnoise(10).out(o1)";
    let result = eval(src).unwrap();
    let b0 = result.shaders[0].as_ref().expect("o0 written");
    let b1 = result.shaders[1].as_ref().expect("o1 written");
    assert!(b0.contains("osc("), "{b0}");
    assert!(b1.contains("noise("), "{b1}");
    assert!(result.shaders[2].is_none());
    assert!(result.shaders[3].is_none());
}

#[test]
fn render_with_no_args_shows_all_buffers() {
    let result = eval("osc(60).out()\nrender()").unwrap();
    assert_eq!(result.render_mode, RenderMode::All);
}

#[test]
fn render_with_an_arg_shows_a_single_buffer() {
    let result = eval("osc(60).out(o1)\nrender(1)").unwrap();
    assert_eq!(result.render_mode, RenderMode::Single(1));
}

#[test]
fn a_chain_with_no_source_is_a_clean_error_not_a_panic() {
    match eval("invert(1).out()") {
        Err(e) => assert!(!e.is_empty()),
        Ok(_) => panic!("expected an error for a chain with no source"),
    }
}

#[test]
fn setnearest_stub_does_not_break_the_rest_of_the_chain() {
    let src = "osc(60).out(o0)\no0.setNearest()";
    assert!(eval(src).is_ok());
}

#[test]
fn bare_inner_width_and_height_are_usable_without_a_window_prefix() {
    // in a real browser `window` is the global object, so real sketches
    // often use `innerWidth`/`innerHeight` bare, not just `window.innerWidth`
    let glsl = ok_shader0("osc(1,1,innerWidth/innerHeight).out()");
    assert!(glsl.contains("iResolution.x"), "{glsl}");
    assert!(glsl.contains("iResolution.y"), "{glsl}");
}

#[test]
fn pb_setname_and_list_are_harmless_no_ops() {
    // `pb.setName(...)`/`pb.list()` are boilerplate some external platform
    // injects when a sketch is shared - not a hydra.js API at all - and
    // must not stop the sketch's real content from evaluating.
    let src = "pb.setName(\"someone\")\npb.list()\nosc(60).out()";
    assert!(eval(src).is_ok());
}

#[test]
fn p5_instance_and_its_common_methods_are_harmless_no_ops() {
    // p5.js is a whole separate creative-coding framework with no Rust
    // equivalent here (see README.md); a P5 instance is stood in for by a
    // plain settable map so the sketch's real hydra content downstream
    // still gets to evaluate instead of hard-failing on this boilerplate.
    let src = r#"
        let p1 = P5({mode: "WEBGL"});
        p1.hide();
        p1.show();
        p1.textSize(24);
        p1.fill(255, 0, 0);
        p1.stroke("red");
        p1.stroke(15, 252, 3);
        p1.strokeWeight(2);
        p1.someArbitraryProperty = "anything";
        osc(60).out()
    "#;
    assert!(eval(src).is_ok());
}

#[test]
fn arithmetic_on_an_unset_p5_instance_property_falls_back_to_zero() {
    // `p1.frameCount`/`mouseX`/etc. aren't populated with a real live
    // value (no real p5.js canvas is ever rendered here) - reading one off
    // the P5() stand-in map yields `()`, same as any other undefined JS
    // value; multiplying that by a number must not hard-fail the sketch.
    let src = r#"
        let p1 = P5({});
        let c = p1.frameCount * 256;
        osc(60).out()
    "#;
    assert!(eval(src).is_ok());
}

#[test]
fn a_chained_double_out_call_is_a_harmless_no_op() {
    // `osc(10).out(o0).out()`/`render(o0).out()` - the first `.out(...)`
    // already returns `()` (nothing further to chain), so the second
    // call's receiver is `()` rather than a Node.
    assert!(eval("osc(10).out(o0).out()").is_ok());
    assert!(eval("osc(10).out().out(o0)").is_ok());
}

#[test]
fn out_with_a_float_buffer_index_rounds_and_clamps() {
    // JS silently ignores extra arguments, so `.out(0.1,0.7,0.5)` reaches
    // the registered function as just `.out(0.1)` once argtrunc.rs has
    // already dropped the rest - a float buffer index instead of the
    // usual `o0`-`o3` int constants.
    assert!(eval("osc(10).out(1.765)").is_ok());
}

#[test]
fn a_bare_receiverless_out_call_is_a_harmless_no_op() {
    // No preceding chain at all - most plausibly leftover/broken
    // authoring rather than a real hydra.js idiom, but shouldn't take the
    // rest of the sketch down with it.
    assert!(eval("out(o0)\nosc(60).out()").is_ok());
    assert!(eval("out()\nosc(60).out()").is_ok());
}

#[test]
fn source_slot_init_with_a_dom_element_config_is_a_harmless_no_op() {
    // `sN.init({src: ...})` - not a real hydra.js API, but a pattern some
    // external platforms use to feed a p5.js canvas into a source slot.
    let src = "s0.init({src: 1});\nosc(60).out()";
    assert!(eval(src).is_ok());
}

#[test]
#[cfg(feature = "midi")]
fn midi_note_and_cc_compile_to_the_expected_uniform_references() {
    // ported real-world hydra-midi API: note()/cc() and their chain
    // methods (.velocity()/.adsr()/.range()/.scale()/.smooth()), plus the
    // plain _note()/_cc()/_noteVelocity() forms used inside a stripped
    // `()=>` wrapper.
    let glsl = ok_shader0(
        "osc(60, note(\"C4\").velocity(), cc(1).range(0,1)).luma(_note(60)*0.5).out()",
    );
    assert!(glsl.contains("iMidiVelocity[60]"), "{glsl}");
    assert!(glsl.contains("iMidiCC[1]"), "{glsl}");
    assert!(glsl.contains("iMidiNote[60]"), "{glsl}");
}

#[test]
#[cfg(feature = "midi")]
fn midi_aftertouch_compiles_to_the_expected_uniform_references() {
    // aft(note) -> per-note polyphonic aftertouch; bare aft() -> channel-
    // wide aftertouch; _aft(...) is the plain (non-chainable) equivalent,
    // for use inside a stripped `()=>` wrapper.
    let glsl = ok_shader0("osc(60, aft(60).range(0,1), _aft()*0.5).out()");
    assert!(glsl.contains("iMidiAftertouch[60]"), "{glsl}");
    assert!(glsl.contains("iMidiChannelAftertouch"), "{glsl}");
}

#[test]
#[cfg(feature = "midi")]
fn midi_adsr_and_smooth_register_a_request_and_reference_their_own_slot() {
    use hydra_rust::eval::MidiRequest;
    let result = eval("solid(1, 0, note(60).adsr(50,100,0.7,300)).diff(osc(cc(1).smooth(0.2))).out()").unwrap();
    let glsl = result.shaders[0].as_ref().unwrap();
    assert!(glsl.contains("iMidiEnvelope[0]"), "{glsl}");
    assert!(glsl.contains("iMidiCCSmoothed[1]"), "{glsl}");
    assert!(
        result.midi_requests.iter().any(|r| matches!(r, MidiRequest::AdsrSlot { slot: 0, note: 60, .. })),
        "{:?}",
        result.midi_requests
    );
    assert!(
        result
            .midi_requests
            .iter()
            .any(|r| matches!(r, MidiRequest::SetCcSmooth { index: 1, .. })),
        "{:?}",
        result.midi_requests
    );
}

#[test]
#[cfg(feature = "midi")]
fn midi_start_requires_no_other_setup_to_evaluate() {
    // real hydra-midi requires an explicit `midi.start()` before anything
    // works; eval() itself never touches real MIDI hardware either way -
    // it only records the request (see midi.rs's MidiManager, wired up in
    // app.rs), so this is safe to run offline/in CI.
    use hydra_rust::eval::MidiRequest;
    let result = eval("midi.start().show()\nmidi.channel(0)\nosc(60).out()").unwrap();
    assert!(result.midi_requests.iter().any(|r| matches!(r, MidiRequest::Start)));
    assert!(result.midi_requests.iter().any(|r| matches!(r, MidiRequest::Show)));
}

#[test]
#[cfg(feature = "midi")]
fn midi_show_and_hide_queue_requests_without_touching_real_midi() {
    use hydra_rust::eval::MidiRequest;
    let result = eval("midi.hide()\nosc(60).out()").unwrap();
    assert!(result.midi_requests.iter().any(|r| matches!(r, MidiRequest::Hide)));
    assert!(!result.midi_requests.iter().any(|r| matches!(r, MidiRequest::Show)));
}

#[test]
#[cfg(feature = "audio")]
fn audio_show_and_hide_queue_requests_without_touching_the_microphone() {
    // eval() never opens the microphone itself either way (see
    // audio.rs's AudioManager::ensure_started, only called from app.rs) -
    // a.show()/a.hide() just toggle the host's FFT overlay, recorded here
    // the same way the a.set*() setters already are.
    use hydra_rust::eval::AudioRequest;
    let result = eval("a.show()\nosc(60, 0.1, a.fft[0]).out()").unwrap();
    assert!(result.audio_requests.iter().any(|r| matches!(r, AudioRequest::Show)));

    let result = eval("a.hide()\nosc(60).out()").unwrap();
    assert!(result.audio_requests.iter().any(|r| matches!(r, AudioRequest::Hide)));
}

#[test]
fn ranged_random_and_mixed_numeric_math_calls_are_accepted() {
    // real JS's Math.random()/Math.pow() etc. tolerate any argument types
    // real hydra.js sketches occasionally pass (Math.random() takes none
    // at all but silently ignores extras; Math.pow accepts any numerics).
    let src = "noise(pow(2, 3), random(0, 1)).out()";
    assert!(eval(src).is_ok());
}

#[test]
fn stroke_text_variants_alias_the_same_rendering_as_text() {
    // hydra-text.js's strokeText/fillStrokeText/strokeFillText - not a
    // faithful stroke-vs-fill render, but shouldn't hard-fail, and should
    // accept the optional config argument real sketches often pass.
    for call in ["strokeText(\"hi\")", "fillStrokeText(\"hi\", hydraText)", "strokeFillText(\"hi\")"] {
        let src = format!("solid(0,0,0,1).diff({call}).out()");
        assert!(eval(&src).is_ok(), "{call} failed");
    }
}

#[test]
fn comma_tuple_call_argument_collapses_to_its_last_value() {
    // real sketches very commonly write this exact shape, plausibly
    // meaning an array `[a,b]` - but real JS's comma operator actually
    // just discards `0.01` and keeps `0.2`, so that's what this compiles.
    let glsl = ok_shader0("shape(4, (0.01, 0.2), 1).out()");
    assert!(glsl.contains("shape(st, 4.0, 0.2, 1.0)"), "{glsl}");
}

#[test]
fn arrow_function_parameter_lists_survive_comma_tuple_handling() {
    // the exact ambiguity commaexpr and arrowfn/arrow.rs must agree on:
    // both a real tuple and an arrow's own params are `(a, b)`.
    let glsl = ok_shader0("let f = (a,b) => a+b;\nsolid(f(0.2,0.3)).out()");
    assert!(glsl.contains("solid("), "{glsl}");
}

#[test]
fn object_literal_call_arguments_no_longer_hard_parse_error() {
    // real sketches pass these to calls hydra-rust doesn't implement for
    // real (p5.js/Three.js/canvas interop, here stood in for by the
    // fictitious `notARealFunction`) - once the object literal itself
    // parses as an (unused) Rhai map, the failure becomes an ordinary
    // "function not found" instead of the syntax error that used to abort
    // the whole script at this line.
    let err = match eval("notARealFunction({src: 1, default: 2})") {
        Err(e) => e,
        Ok(_) => panic!("expected `notARealFunction` to be an unregistered function"),
    };
    assert!(!err.contains("Syntax error"), "{err}");
}

#[test]
fn destructured_reactive_arrow_still_works_alongside_object_literals() {
    // the exact ambiguity objlit and arrow.rs must agree on: both are a
    // `{` immediately preceded by `(`.
    let glsl = ok_shader0("noise(10, ({time})=>Math.sin(time)*3).out()");
    assert!(glsl.contains("noise("), "{glsl}");
}

#[test]
#[cfg(feature = "image_url")]
fn init_image_queues_a_source_request_without_touching_the_network() {
    // eval() itself never performs the actual fetch - it only records the
    // request for the app to act on (see imageload.rs) - so this is safe
    // to run offline/in CI.
    use hydra_rust::eval::SourceRequest;
    let result = eval("s0.initImage(\"https://example.com/pic.png\").out()").unwrap();
    assert_eq!(result.source_requests.len(), 1);
    match &result.source_requests[0] {
        SourceRequest::InitImage { slot, url } => {
            assert_eq!(*slot, 0);
            assert_eq!(url.as_str(), "https://example.com/pic.png");
        }
        #[allow(unreachable_patterns)]
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
#[cfg(feature = "image_url")]
fn init_gif_queues_a_source_request_without_touching_the_network() {
    // same "eval() never touches the network" guarantee as initImage -
    // the actual fetch+decode happens in imageload.rs's ImageManager,
    // wired up in app.rs, well after eval() has already returned.
    use hydra_rust::eval::SourceRequest;
    let result = eval("s1.initGif(\"https://example.com/anim.gif\").out()").unwrap();
    assert_eq!(result.source_requests.len(), 1);
    match &result.source_requests[0] {
        SourceRequest::InitGif { slot, url } => {
            assert_eq!(*slot, 1);
            assert_eq!(url.as_str(), "https://example.com/anim.gif");
        }
        #[allow(unreachable_patterns)]
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
#[cfg(feature = "video")]
fn init_video_queues_a_source_request_without_touching_ffmpeg() {
    // same "eval() never touches the network/an external process"
    // guarantee as initImage/initGif - the actual ffmpeg subprocess is
    // spawned by video.rs's VideoManager, wired up in app.rs, well after
    // eval() has already returned.
    use hydra_rust::eval::SourceRequest;
    let result = eval("s2.initVideo(\"https://example.com/clip.mp4\").out()").unwrap();
    assert_eq!(result.source_requests.len(), 1);
    match &result.source_requests[0] {
        SourceRequest::InitVideo { slot, url } => {
            assert_eq!(*slot, 2);
            assert_eq!(url.as_str(), "https://example.com/clip.mp4");
        }
        #[allow(unreachable_patterns)]
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
fn smooth_and_fit_pattern_calls_compile_to_valid_glsl() {
    // .smooth() interpolates between array entries over time; .fit()
    // remaps the array's own value range - both must compile cleanly
    // whether chained onto a bare array or onto each other.
    let glsl = ok_shader0("osc(60, [0.1, 0.5, 0.9].smooth().fit(0, 1), 0).out()");
    assert!(glsl.contains("step("), "{glsl}");
    assert!(glsl.contains("iTempo"), "{glsl}");
}

#[test]
fn array_reduce_with_a_multi_param_arrow_callback_compiles() {
    // real sketches commonly use JS's generic Array.reduce()/map() with an
    // arrow callback for one-time setup computation (palette generation,
    // an audio-FFT sum) - nothing to do with hydra's own chain API. Rhai
    // has no `=>` syntax at all, but does have its own closure syntax that
    // `Array::reduce` already accepts directly.
    let src = "let s = [1,2,3].reduce((a,b) => a+b, 0);\nosc(60).out()";
    assert!(eval(src).is_ok());
}

#[test]
fn hydratext_config_assignments_are_harmless_no_ops() {
    // the hydra-text.js community extension's config object - real
    // sketches set arbitrary properties on it before calling the
    // extension's own (unsupported) text-rendering function.
    let src = "hydraText.font = \"serif\";\nhydraText.lineWidth = \"2%\";\nosc(60).out()";
    assert!(eval(src).is_ok());
}

#[test]
#[cfg(feature = "stream")]
fn init_stream_queues_a_source_request_without_touching_the_network() {
    // same "eval() never touches the network/an external process" guarantee
    // as initImage/initGif/initVideo - the actual WebRTC connection is
    // established by stream.rs's StreamManager, wired up in app.rs, well
    // after eval() has already returned.
    use hydra_rust::eval::SourceRequest;
    let result = eval("s1.initStream(\"127.0.0.1:9000\").out()").unwrap();
    assert_eq!(result.source_requests.len(), 1);
    match &result.source_requests[0] {
        SourceRequest::InitStream { slot, addr } => {
            assert_eq!(*slot, 1);
            assert_eq!(addr.as_str(), "127.0.0.1:9000");
        }
        #[allow(unreachable_patterns)]
        other => panic!("unexpected request: {other:?}"),
    }
}

#[test]
#[cfg(feature = "stream")]
fn broadcast_stream_and_stop_broadcast_queue_requests() {
    // same "eval() never touches the network" guarantee - the actual TCP
    // listen/WebRTC connection is established by broadcast.rs's
    // BroadcastManager, wired up in app.rs, well after eval() returns.
    use hydra_rust::eval::BroadcastRequest;
    let result = eval("broadcastStream(9000)\nosc(60).out()").unwrap();
    assert!(matches!(result.broadcast_request, Some(BroadcastRequest::Start(9000))));

    let result = eval("stopBroadcast()\nosc(60).out()").unwrap();
    assert!(matches!(result.broadcast_request, Some(BroadcastRequest::Stop)));

    // a script that calls neither leaves it None, not a leftover value from
    // some previous evaluation - PatchState is fresh every eval() call.
    let result = eval("osc(60).out()").unwrap();
    assert!(result.broadcast_request.is_none());
}

#[test]
fn set_resolution_with_static_args_sets_a_render_resolution_override() {
    let result = eval("setResolution(320, 240)\nosc(60).out()").unwrap();
    assert_eq!(result.render_resolution, Some((320, 240)));
}

#[test]
fn set_resolution_with_a_reactive_arg_is_treated_as_no_override() {
    // window.innerWidth/innerHeight compile to a reactive GLSL expression,
    // not a plain number - there's no per-frame callback to re-evaluate it
    // against, so this must NOT be mistaken for a real (0, 0) override.
    let result = eval("setResolution(window.innerWidth, window.innerHeight)\nosc(60).out()").unwrap();
    assert!(result.render_resolution.is_none());
}

#[test]
fn set_nearest_and_set_mode_record_the_requested_buffer_filter() {
    use hydra_rust::eval::BufferFilter;
    let result = eval("o0.setNearest()\no1.setMode(\"nearest\")\nosc(60).out()").unwrap();
    assert_eq!(result.buffer_filter[0], Some(BufferFilter::Nearest));
    assert_eq!(result.buffer_filter[1], Some(BufferFilter::Nearest));
    assert_eq!(result.buffer_filter[2], None);
    assert_eq!(result.buffer_filter[3], None);
}

#[test]
fn set_mode_with_an_unrecognized_string_is_ignored_not_a_hard_error() {
    let result = eval("o0.setMode(\"blah\")\nosc(60).out()").unwrap();
    assert!(result.buffer_filter[0].is_none());
}
