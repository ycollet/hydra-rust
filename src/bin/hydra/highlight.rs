use std::ops::Range;

use eframe::egui;
use egui::text::{LayoutJob, LayoutSection};
use egui::{Color32, FontId, TextFormat};
use regex::Regex;

// OneDark theme
const THEME: [Color32; 10] = [
    Color32::from_rgb(198, 120, 221), // 0 Keyword (blend/modulate) — purple
    Color32::from_rgb(97, 175, 239),  // 1 Builtin (sources) — blue
    Color32::from_rgb(86, 182, 194),  // 2 Operator (geo) — cyan
    Color32::from_rgb(209, 154, 102), // 3 Number — orange
    Color32::from_rgb(152, 195, 121), // 4 String — green
    Color32::from_rgb(92, 99, 112),   // 5 Comment — gray
    Color32::from_rgb(229, 192, 123), // 6 Variable (color ops) — gold
    Color32::from_rgb(224, 148, 120), // 7 Symbol (rhai keywords) — salmon
    Color32::from_rgb(224, 208, 120), // 8 Special — bright yellow
    Color32::from_rgb(140, 140, 140), // 9 Punctuation — dim
];

const DEFAULT_FG: Color32 = Color32::from_rgb(171, 178, 191);

pub struct HydraHighlighter {
    regex: Regex,
    group_categories: Vec<usize>,
}

impl HydraHighlighter {
    pub fn new() -> Self {
        let rules: &[(&str, usize)] = &[
            (r"//[^\n]*", 5),
            (r"/\*[^*]*\*+(?:[^/*][^*]*\*+)*/", 5),
            (r"\b\d+(\.\d+)?\b", 3),
            (r"\b(?:osc|noise|voronoi|shape|gradient|solid|rings|checker|src|text)\b", 1),
            (r"\b(?:add|mult|blend|diff|layer|mask|sub|modulate|modulateScale|modulateRotate|modulateRepeat|modulateRepeatX|modulateRepeatY|modulateKaleid|modulateScrollX|modulateScrollY|modulatePixelate|modulateHue)\b", 0),
            (r"\b(?:rotate|scale|scroll|kaleid|pixelate|repeat|scrollX|scrollY|repeatX|repeatY|polar|cart|fold)\b", 2),
            (r"\b(?:color|invert|contrast|brightness|saturate|hue|posterize|luma|colorama|shift|thresh)\b", 6),
            (r"\b(?:out|r|g|b|render|o0|o1|o2|o3|s0|s1|s2|s3|time|beat|tempo|phase|mouseX|mouseY|fast|smooth|offset)\b", 8),
            (r"\b(?:let|const|if|else|while|loop|for|in|fn|return|true|false|hush|initCam)\b", 7),
            (r"[+\-*/%]=?|[=!<>]=|&&|\|\||!", 2),
            (r"[.(),;]", 9),
        ];

        let mut parts = Vec::new();
        let mut categories = Vec::new();
        for (i, (pattern, cat)) in rules.iter().enumerate() {
            parts.push(format!("(?P<g{i}>{pattern})"));
            categories.push(*cat);
        }

        let regex = Regex::new(&parts.join("|")).expect("valid syntax regex");
        Self { regex, group_categories: categories }
    }

    fn tokenize<'a>(&'a self, text: &'a str) -> impl Iterator<Item = (Range<usize>, usize)> + 'a {
        self.regex.captures_iter(text).filter_map(|caps| {
            for (i, cat) in self.group_categories.iter().enumerate() {
                if let Some(m) = caps.name(&format!("g{i}")) {
                    return Some((m.start()..m.end(), *cat));
                }
            }
            None
        })
    }

    pub fn layout_job(&self, text: &str, font_id: &FontId, text_bg: Color32) -> LayoutJob {
        let default_fmt = TextFormat {
            font_id: font_id.clone(),
            color: DEFAULT_FG,
            background: text_bg,
            ..Default::default()
        };

        let mut job = LayoutJob {
            text: text.to_owned(),
            ..Default::default()
        };

        let spans: Vec<(Range<usize>, usize)> = self.tokenize(text).collect();

        if spans.is_empty() {
            job.sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: 0..text.len(),
                format: default_fmt,
            });
        } else {
            let mut pos = 0;
            for (range, cat) in &spans {
                if range.start > pos {
                    job.sections.push(LayoutSection {
                        leading_space: 0.0,
                        byte_range: pos..range.start,
                        format: default_fmt.clone(),
                    });
                }
                job.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: range.clone(),
                    format: TextFormat {
                        font_id: font_id.clone(),
                        color: THEME[*cat],
                        background: text_bg,
                        ..Default::default()
                    },
                });
                pos = range.end;
            }
            if pos < text.len() {
                job.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: pos..text.len(),
                    format: default_fmt,
                });
            }
        }

        job
    }
}
