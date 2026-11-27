use super::{CODE_EDITOR_ID, HydraApp, egui};

impl HydraApp {
    /// Evaluates just the editor's current line - `Ctrl`/`Cmd`+`Enter` -
    /// mirroring upstream Hydra's `editor: eval line` (CodeMirror's
    /// `getLine(cursor.line)`). A no-op if the cursor's line is blank.
    pub(super) fn evaluate_line(&mut self, ctx: &egui::Context) {
        let line = current_line(&self.code, cursor_char_index(ctx));
        if !line.trim().is_empty() {
            self.evaluate_code(&line);
        }
    }

    /// Evaluates the contiguous "block" of non-blank lines around the
    /// editor's cursor - `Alt`+`Enter` - mirroring upstream Hydra's
    /// `editor: eval block` (CodeMirror's `getCurrentBlock()`): expands
    /// from the cursor's line upward and downward through every
    /// non-blank neighboring line, stopping at the first blank line (or a
    /// file boundary) on each side. A no-op if the cursor's line is
    /// blank.
    pub(super) fn evaluate_block(&mut self, ctx: &egui::Context) {
        let block = current_block(&self.code, cursor_char_index(ctx));
        if !block.trim().is_empty() {
            self.evaluate_code(&block);
        }
    }

    /// Evaluates the editor's current text selection instead of the whole
    /// script - `Ctrl`/`Cmd`+`Alt`+`Enter`. Falls back to a full
    /// `evaluate()` when there's no selection (or it's empty/whitespace),
    /// since there's nothing sensible to run otherwise.
    pub(super) fn evaluate_selection(&mut self, ctx: &egui::Context) {
        let selected = egui::TextEdit::load_state(ctx, egui::Id::new(CODE_EDITOR_ID))
            .and_then(|state| state.cursor.char_range())
            .and_then(|range| {
                let (lo, hi) = (
                    range.primary.index.min(range.secondary.index),
                    range.primary.index.max(range.secondary.index),
                );
                if lo == hi {
                    None
                } else {
                    self.code
                        .chars()
                        .take(hi)
                        .skip(lo)
                        .collect::<String>()
                        .into()
                }
            })
            .filter(|s: &String| !s.trim().is_empty());

        match selected {
            Some(code) => self.evaluate_code(&code),
            None => self.evaluate(),
        }
    }

    /// Toggles `//` line comments on the editor's current selection -
    /// `Ctrl`/`Cmd`+`/` - the same way most code editors do. A partial
    /// selection is expanded out to cover every line it touches first;
    /// with no selection at all, only the cursor's current line is
    /// toggled. If every non-blank touched line is already commented,
    /// removes one leading `//` (plus one following space, if present)
    /// from each; otherwise adds `// ` to each (blank lines are left
    /// alone either way, so toggling twice is always a no-op on them).
    pub(super) fn toggle_comment_selection(&mut self, ctx: &egui::Context) {
        let id = egui::Id::new(CODE_EDITOR_ID);
        let n = self.code.chars().count();

        let range = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range());
        let (lo, hi) = match range {
            Some(r) => (
                r.primary.index.min(r.secondary.index),
                r.primary.index.max(r.secondary.index),
            ),
            None => (0, 0),
        };
        let lo = lo.min(n);
        let hi = hi.min(n);

        let (new_code, new_lo, new_hi) = toggle_line_comments(&self.code, lo, hi);
        self.code = new_code;

        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(new_lo),
                egui::text::CCursor::new(new_hi),
            )));
        state.store(ctx, id);
    }
}

/// Char-index `(start, end)` span of each line in `chars`, `end` excluding
/// the line's own trailing `\n` - shared by `toggle_line_comments` and the
/// eval-line/eval-block helpers below.
fn line_spans(chars: &[char]) -> Vec<(usize, usize)> {
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            lines.push((start, i));
            start = i + 1;
        }
    }
    lines.push((start, chars.len()));
    lines
}

/// Returns the editor's current cursor position as a char index into
/// `self.code`, looked up from the persisted `TextEdit` state (the same
/// one `evaluate_selection`/`toggle_comment_selection` use). Falls back
/// to `0` when there's no tracked cursor yet (e.g. before the editor has
/// been focused).
pub(super) fn cursor_char_index(ctx: &egui::Context) -> usize {
    egui::TextEdit::load_state(ctx, egui::Id::new(CODE_EDITOR_ID))
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.primary.index)
        .unwrap_or(0)
}

/// Returns the single line of `code` containing char-index `pos` (clamped
/// to `code`'s length) - the exact text `Ctrl`/`Cmd`+`Enter`'s "eval line"
/// evaluates, mirroring upstream Hydra's `editor: eval line`.
pub(super) fn current_line(code: &str, pos: usize) -> String {
    let chars: Vec<char> = code.chars().collect();
    let pos = pos.min(chars.len());
    let lines = line_spans(&chars);
    let idx = lines
        .iter()
        .position(|&(_, e)| pos <= e)
        .unwrap_or(lines.len() - 1);
    let (s, e) = lines[idx];
    chars[s..e].iter().collect()
}

/// Returns the contiguous "block" of non-blank lines around char-index
/// `pos` in `code` - the exact text `Alt`+`Enter`'s "eval block"
/// evaluates, mirroring upstream Hydra's `getCurrentBlock()`: starting
/// from the cursor's own line, expands upward and downward through every
/// non-blank neighboring line, stopping at the first blank line (or a
/// file boundary) on each side. If the cursor's own line is blank, the
/// block is just that (blank) line - no expansion - matching upstream.
pub(super) fn current_block(code: &str, pos: usize) -> String {
    let chars: Vec<char> = code.chars().collect();
    let pos = pos.min(chars.len());
    let lines = line_spans(&chars);
    let idx = lines
        .iter()
        .position(|&(_, e)| pos <= e)
        .unwrap_or(lines.len() - 1);

    let is_blank = |(s, e): (usize, usize)| chars[s..e].iter().all(|c| c.is_whitespace());

    let mut first = idx;
    let mut last = idx;
    if !is_blank(lines[idx]) {
        while first > 0 && !is_blank(lines[first - 1]) {
            first -= 1;
        }
        while last + 1 < lines.len() && !is_blank(lines[last + 1]) {
            last += 1;
        }
    }

    let (s, _) = lines[first];
    let (_, e) = lines[last];
    chars[s..e].iter().collect()
}

/// Toggles `//` line comments on every line a `[lo, hi)` char-index
/// selection touches (expanding a partial selection out to whole lines
/// first), returning the new source text plus an updated `(lo, hi)`
/// char-index range covering the touched lines in the new text - pulled
/// out of `toggle_comment_selection` as a plain, egui-free function so it
/// can be unit tested directly. See that method's doc comment for the
/// exact behavior (blank-line handling, un/comment detection, etc).
pub(super) fn toggle_line_comments(code: &str, lo: usize, hi: usize) -> (String, usize, usize) {
    let chars: Vec<char> = code.chars().collect();
    let n = chars.len();
    let lo = lo.min(n);
    let hi = hi.min(n);

    let lines = line_spans(&chars);

    let line_at = |pos: usize| -> usize {
        lines
            .iter()
            .position(|&(_, e)| pos <= e)
            .unwrap_or(lines.len() - 1)
    };
    let first_line = line_at(lo);
    let mut last_line = line_at(hi);
    // A selection ending exactly at the start of a later line (e.g.
    // a whole-line selection via Shift+Down) doesn't drag that line
    // in too - matches how other editors treat this boundary.
    if hi > lo && last_line > first_line && hi == lines[last_line].0 {
        last_line -= 1;
    }

    let is_blank = |(s, e): (usize, usize)| chars[s..e].iter().all(|c| c.is_whitespace());
    let is_commented = |(s, e): (usize, usize)| {
        let indent = chars[s..e].iter().take_while(|c| c.is_whitespace()).count();
        chars[s + indent..e]
            .iter()
            .collect::<String>()
            .starts_with("//")
    };
    let all_commented = (first_line..=last_line)
        .map(|i| lines[i])
        .all(|span| is_blank(span) || is_commented(span));

    let mut out = String::with_capacity(code.len() + 8);
    let mut out_len = 0usize;
    let mut new_lo = 0usize;
    let mut new_hi = 0usize;

    for (i, &(s, e)) in lines.iter().enumerate() {
        if i == first_line {
            new_lo = out_len;
        }
        let span_chars = &chars[s..e];
        if (first_line..=last_line).contains(&i) && !is_blank((s, e)) {
            let indent = span_chars.iter().take_while(|c| c.is_whitespace()).count();
            let (pre, rest) = span_chars.split_at(indent);
            out.extend(pre.iter());
            out_len += pre.len();
            let rest_str: String = rest.iter().collect();
            if all_commented {
                let stripped = rest_str
                    .strip_prefix("// ")
                    .or_else(|| rest_str.strip_prefix("//"))
                    .unwrap_or(&rest_str);
                out.push_str(stripped);
                out_len += stripped.chars().count();
            } else {
                out.push_str("// ");
                out.push_str(&rest_str);
                out_len += 3 + rest_str.chars().count();
            }
        } else {
            out.extend(span_chars.iter());
            out_len += span_chars.len();
        }
        if i == last_line {
            new_hi = out_len;
        }
        if i + 1 < lines.len() {
            out.push('\n');
            out_len += 1;
        }
    }

    (out, new_lo, new_hi)
}
