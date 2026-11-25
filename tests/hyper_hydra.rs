#![cfg(feature = "javascript")]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use hydra_rust::javascript::JavascriptRuntime;

const REVISION: &str = "e48aacbe775cd6731c50cc327ed86cd7e7beab6c";

struct ShaderFile(PathBuf);

impl Drop for ShaderFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            eprintln!("Shader cleanup failed: {error}");
        }
    }
}

fn validate_shader(shader: &str) -> bool {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "hydra-hyper-{}-{}.frag",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let temporary = ShaderFile(path.clone());
    file.write_all(hydra_rust::shader::fragment_source(shader).as_bytes())
        .unwrap();
    let output = Command::new("glslangValidator")
        .args(["-S", "frag"])
        .arg(&path)
        .output()
        .expect("Install glslangValidator to run upstream compatibility checks");
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }
    drop(temporary);
    output.status.success()
}

#[test]
#[ignore = "Requires HYDRA_HYPER_HYDRA_ROOT and glslangValidator"]
fn checked_upstream_modules_execute_and_compile() {
    let root = PathBuf::from(
        std::env::var_os("HYDRA_HYPER_HYDRA_ROOT")
            .expect("Set HYDRA_HYPER_HYDRA_ROOT to a local Hyper-Hydra checkout"),
    );
    let revision = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(revision.status.success());
    assert_eq!(String::from_utf8(revision.stdout).unwrap().trim(), REVISION);
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["diff", "--quiet", "HEAD"])
            .status()
            .unwrap()
            .success()
    );

    let cases = [
        ("hydra-abbreviations.js", "osc(10).bright(0.2).rot(0.1).o()"),
        (
            "hydra-arithmetics.js",
            "osc(10).add(0.2).sin().range(0,1).min(osc(3)).out()",
        ),
        ("hydra-arrays.js", "osc([10,20].add(5)).out()"),
        (
            "hydra-blend.js",
            "solid(0.4,0.2,0.1,1).screen(osc(8),0.5).out()",
        ),
        (
            "hydra-colorspaces.js",
            "osc(10).hsv.hOffset(0.1).yuv.uMult(0.5).out()",
        ),
        (
            "hydra-colorspaces.js",
            "osc(10).hsv.hWith(tex=>tex.invert()).out()",
        ),
        ("hydra-convolutions.js", "sharpen(o0,0.25,1,1).out()"),
        ("hydra-convolutions.js", "sharpenLuma(s1,0.25,1,1).out()"),
        ("hydra-fractals.js", "osc(10).mirrorX().out()"),
        ("hydra-glsl.js", "osc(10).glslCoord('uv * 2.0').out()"),
        (
            "hydra-glsl.js",
            "glsl('vec4(uv,0.0,1.0)').glslColor('c0 * 0.5').out()",
        ),
        (
            "hydra-glsl.js",
            "osc(10).glslCombine('mix(c0,c1,0.5)',solid(1)).glslCombineCoord('uv+c0.xy',osc(2)).out()",
        ),
        ("hydra-glsl.js", "osc(10).glslHsv('hsv.x += 0.2;').out()"),
        ("hydra-swizzle.js", "osc(10).bgra.r.out()"),
        ("hydra-vec4.js", "vec4(vec3(0.2,0.4,0.6),1).out()"),
        ("hydra-vec4.js", "vec4(vec2(0.2,0.4),vec2(0.6,1)).out()"),
    ];
    for (library, sketch) in cases {
        let mut runtime = JavascriptRuntime::new(root.clone()).unwrap();
        let result = runtime
            .eval(&format!(
                "loadScript({});{sketch}",
                serde_json::to_string(library).unwrap()
            ))
            .unwrap_or_else(|error| panic!("{library}: {error}"));
        assert!(result.shaders.iter().any(Option::is_some), "{library}");
        for shader in result.shaders.iter().flatten() {
            assert!(validate_shader(shader), "{library}: {sketch}");
        }
    }
    let mut combined = JavascriptRuntime::new(root.clone()).unwrap();
    for library in [
        "hydra-arithmetics.js",
        "hydra-colorspaces.js",
        "hydra-convolutions.js",
        "hydra-glsl.js",
        "hydra-swizzle.js",
        "hydra-blend.js",
    ] {
        combined
            .eval(&format!(
                "loadScript({})",
                serde_json::to_string(library).unwrap()
            ))
            .unwrap();
    }
    let graph = combined
        .eval("osc(10).glslCoord('uv*2.0').hsv.hWith(tex=>tex.sin()).bgra.out()")
        .unwrap();
    assert!(validate_shader(graph.shaders[0].as_ref().unwrap()));
    let mut runtime = JavascriptRuntime::new(root).unwrap();
    let blend = runtime
        .eval("loadScript('hydra-blend.js');solid(0.5).phoenix(solid(0.25)).out()")
        .unwrap();
    assert!(!validate_shader(blend.shaders[0].as_ref().unwrap()));
}
