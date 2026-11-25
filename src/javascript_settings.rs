use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JavascriptSettings {
    pub execution_ms: u64,
    pub memory_mib: usize,
    pub stack_kib: usize,
    pub script_kib: usize,
    pub library_kib: usize,
    pub loaded_mib: usize,
    pub pending_jobs: usize,
}

impl Default for JavascriptSettings {
    fn default() -> Self {
        Self {
            execution_ms: 200,
            memory_mib: 32,
            stack_kib: 512,
            script_kib: 1024,
            library_kib: 1024,
            loaded_mib: 16,
            pending_jobs: 256,
        }
    }
}

impl JavascriptSettings {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, min, max) in [
            ("Execution time (milliseconds)", self.execution_ms, 1, 2000),
            ("Heap memory (mebibytes)", self.memory_mib as u64, 8, 256),
            ("Stack memory (kibibytes)", self.stack_kib as u64, 128, 2048),
            ("Script size (kibibytes)", self.script_kib as u64, 1, 4096),
            ("Library size (kibibytes)", self.library_kib as u64, 1, 4096),
            (
                "Total library text (mebibytes)",
                self.loaded_mib as u64,
                1,
                64,
            ),
            ("Pending jobs", self.pending_jobs as u64, 1, 4096),
        ] {
            if !(min..=max).contains(&value) {
                return Err(format!("{name} must be between {min} and {max}"));
            }
        }
        if self.library_kib > self.loaded_mib * 1024 {
            return Err("Library size cannot exceed the total library text limit".into());
        }
        Ok(())
    }
}
