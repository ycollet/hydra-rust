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
fn hydratext_config_assignments_are_harmless_no_ops() {
    // the hydra-text.js community extension's config object - real
    // sketches set arbitrary properties on it before calling the
    // extension's own (unsupported) text-rendering function.
    let src = "hydraText.font = \"serif\";\nhydraText.lineWidth = \"2%\";\nosc(60).out()";
    assert!(eval(src).is_ok());
}
