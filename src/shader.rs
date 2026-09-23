use crate::glsl;

const VERTEX_SHADER: &str = "\
#version 330 core
layout(location = 0) in vec2 a_position;
out vec2 v_uv;
void main() {
  gl_Position = vec4(a_position, 0.0, 1.0);
  v_uv = a_position * 0.5 + 0.5;
}";

const FRAGMENT_PREAMBLE: &str = "\
#version 330 core
precision highp float;
uniform float iTime;
uniform vec2 iResolution;
uniform vec2 iMouse;
uniform float iBeat;
uniform float iTempo;
uniform float iPhase;
uniform float iFft[8]; // must match audio::NUM_FFT_BINS
uniform float iMidiNote[128]; // must match midi::NUM_MIDI_NOTES
uniform float iMidiVelocity[128]; // must match midi::NUM_MIDI_NOTES
uniform float iMidiCC[128]; // must match midi::NUM_MIDI_CC
uniform float iMidiCCSmoothed[128]; // must match midi::NUM_MIDI_CC
uniform float iMidiEnvelope[16]; // must match midi::NUM_MIDI_ENVELOPES
uniform float iMidiAftertouch[128]; // must match midi::NUM_MIDI_NOTES
uniform float iMidiChannelAftertouch;
uniform sampler2D iBuffer0;
uniform sampler2D iBuffer1;
uniform sampler2D iBuffer2;
uniform sampler2D iBuffer3;
uniform sampler2D iText0;
uniform sampler2D iSource0;
uniform sampler2D iSource1;
uniform sampler2D iSource2;
uniform sampler2D iSource3;
out vec4 fragColor;
";

const FRAGMENT_MAIN_WRAP: &str = "
void main() {
  vec2 uv = gl_FragCoord.xy / iResolution.xy;
  vec2 st = vec2(uv.x, uv.y);
  mainImage(fragColor, st);
}";

pub const DEFAULT_SHADER: &str = "\
void mainImage(out vec4 c, in vec2 st) {
  c = vec4(0.0, 0.0, 0.0, 1.0);
}";

pub fn vertex_source() -> &'static str {
    VERTEX_SHADER
}

pub fn fragment_source(user_code: &str) -> String {
    format!(
        "{preamble}\n{library}\n{user_code}\n{main}",
        preamble = FRAGMENT_PREAMBLE,
        library = glsl::LIBRARY,
        main = FRAGMENT_MAIN_WRAP,
    )
}

pub fn display_fragment_source() -> String {
    "\
#version 330 core
precision highp float;
uniform sampler2D iBuffer0;
in vec2 v_uv;
out vec4 fragColor;
void main() {
  fragColor = texture(iBuffer0, v_uv);
}"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn braces_balanced(s: &str) -> bool {
        let mut depth = 0i32;
        for c in s.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                return false;
            }
        }
        depth == 0
    }

    #[test]
    fn fragment_source_assembles_preamble_library_and_user_code_in_order() {
        let out = fragment_source("void mainImage(out vec4 c, in vec2 st) { c = vec4(1.0); }");
        let preamble_pos = out.find("#version 330 core").unwrap();
        let library_pos = out.find("_noise").expect("library should be included");
        let user_pos = out.find("mainImage").unwrap();
        let main_pos = out.rfind("void main()").unwrap();
        assert!(preamble_pos < library_pos, "{out}");
        assert!(library_pos < user_pos, "{out}");
        assert!(user_pos < main_pos, "{out}");
    }

    #[test]
    fn fragment_source_has_balanced_braces() {
        let out = fragment_source(DEFAULT_SHADER);
        assert!(braces_balanced(&out), "{out}");
    }

    #[test]
    fn fragment_source_calls_mainimage_from_the_real_main() {
        let out = fragment_source(DEFAULT_SHADER);
        assert!(out.contains("mainImage(fragColor, st)"), "{out}");
    }

    #[test]
    fn default_shader_defines_mainimage() {
        assert!(DEFAULT_SHADER.contains("void mainImage("));
        assert!(braces_balanced(DEFAULT_SHADER));
    }

    #[test]
    fn vertex_source_is_a_complete_shader() {
        let src = vertex_source();
        assert!(src.contains("#version"));
        assert!(src.contains("void main()"));
        assert!(braces_balanced(src));
    }

    #[test]
    fn display_fragment_source_is_a_complete_shader() {
        let src = display_fragment_source();
        assert!(src.contains("#version"));
        assert!(src.contains("iBuffer0"));
        assert!(braces_balanced(&src));
    }
}
