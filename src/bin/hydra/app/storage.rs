use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{JavascriptSettings, ParameterValues};

#[derive(Serialize, Deserialize)]
pub(super) struct Session {
    #[serde(default)]
    pub(super) parameters: ParameterValues,
    pub(super) code: String,
    #[serde(default)]
    pub(super) javascript: bool,
    #[serde(default)]
    pub(super) javascript_module: bool,
    #[serde(default)]
    pub(super) library_root: Option<PathBuf>,
    #[serde(default)]
    pub(super) javascript_settings: JavascriptSettings,
    pub(super) tempo: f32,
    pub(super) font_size: f32,
    pub(super) text_opacity: f32,
    pub(super) current_file: Option<PathBuf>,
    /// Scene banks (see `Bank`) - `#[serde(default)]` so a session file
    /// saved before this feature existed still loads (missing field, not
    /// a parse error).
    #[serde(default)]
    pub(super) banks: Vec<Bank>,
    #[serde(default)]
    pub(super) current_bank: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            parameters: ParameterValues::default(),
            code: String::new(),
            javascript: false,
            javascript_module: false,
            library_root: None,
            javascript_settings: JavascriptSettings::default(),
            tempo: 120.0,
            font_size: 20.0,
            text_opacity: 0.55,
            current_file: None,
            banks: Vec::new(),
            current_bank: 0,
        }
    }
}

pub(super) const NUM_BANKS: usize = 4;
pub(super) const SLOTS_PER_BANK: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ScriptLanguage {
    #[default]
    Rhai,
    Javascript,
}

impl ScriptLanguage {
    pub(super) fn selected(javascript: bool) -> Self {
        if javascript {
            Self::Javascript
        } else {
            Self::Rhai
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Scene {
    pub(super) code: String,
    pub(super) language: Option<ScriptLanguage>,
    pub(super) module: bool,
    pub(super) parameters: ParameterValues,
}

impl<'de> Deserialize<'de> for Scene {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StoredScene {
            Legacy(String),
            Tagged {
                code: String,
                #[serde(default)]
                language: Option<ScriptLanguage>,
                #[serde(default)]
                module: bool,
                #[serde(default)]
                parameters: ParameterValues,
            },
        }
        Ok(match StoredScene::deserialize(deserializer)? {
            StoredScene::Legacy(code) => Self {
                code,
                language: None,
                module: false,
                parameters: ParameterValues::default(),
            },
            StoredScene::Tagged {
                code,
                language,
                module,
                parameters,
            } => Self {
                code,
                language,
                module,
                parameters,
            },
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub(super) struct Bank {
    #[serde(default = "empty_slots")]
    pub(super) slots: Vec<Option<Scene>>,
}

impl Default for Bank {
    fn default() -> Self {
        Self {
            slots: empty_slots(),
        }
    }
}

pub(super) fn empty_slots() -> Vec<Option<Scene>> {
    vec![None; SLOTS_PER_BANK]
}

impl Bank {
    pub(super) fn assign_legacy_language(&mut self, language: ScriptLanguage) {
        for scene in self.slots.iter_mut().flatten() {
            scene.language.get_or_insert(language);
        }
    }
}

/// Keep older exports readable while newer exports carry each scene's language.
#[derive(Serialize, Deserialize)]
pub(super) struct BankFile {
    pub(super) version: u32,
    pub(super) slots: Vec<Option<Scene>>,
}

impl BankFile {
    pub(super) fn into_bank(self, legacy_language: ScriptLanguage) -> Result<Bank, String> {
        if !matches!(self.version, 1..=3) {
            return Err(format!("Unsupported bank version: {}", self.version));
        }
        if self.slots.len() != SLOTS_PER_BANK {
            return Err(format!("A bank must contain {SLOTS_PER_BANK} slots"));
        }
        let mut bank = Bank { slots: self.slots };
        bank.assign_legacy_language(legacy_language);
        Ok(bank)
    }
}

/// Slot exports keep language metadata without depending on a bank or slot index.
#[derive(Serialize, Deserialize)]
pub(super) struct SlotFile {
    pub(super) version: u32,
    pub(super) code: String,
    #[serde(default)]
    pub(super) language: Option<ScriptLanguage>,
    #[serde(default)]
    pub(super) module: bool,
    #[serde(default)]
    pub(super) parameters: ParameterValues,
}

impl SlotFile {
    pub(super) fn into_scene(self, legacy_language: ScriptLanguage) -> Result<Scene, String> {
        if !matches!(self.version, 1..=3) {
            return Err(format!("Unsupported slot version: {}", self.version));
        }
        Ok(Scene {
            code: self.code,
            language: Some(self.language.unwrap_or(legacy_language)),
            module: self.module,
            parameters: self.parameters,
        })
    }
}

pub(super) fn session_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".hydra-rust.json")
}

impl Session {
    pub(super) fn load() -> Self {
        match std::fs::read_to_string(session_path()) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(session) => session,
                Err(error) => {
                    log::warn!("Cannot restore Hydra session: {error}");
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("Cannot read Hydra session: {error}");
                Self::default()
            }
        }
    }

    pub(super) fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(session_path(), json);
        }
    }
}

/// Wraps `current + delta` into `0..n`, in either direction - `-1` from `0`
/// lands on `n - 1`, matching how `Alt+Left`/`Alt+Right` should cycle
/// through the (fixed-size) bank list.
pub(super) fn wrap_bank_index(current: usize, delta: i32, n: usize) -> usize {
    (current as i32 + delta).rem_euclid(n as i32) as usize
}
