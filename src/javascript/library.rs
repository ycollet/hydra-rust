use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Exception, Module, module::Declared};

use crate::javascript_settings::JavascriptSettings;

#[derive(Clone)]
pub(super) struct Libraries {
    pub root: PathBuf,
    settings: JavascriptSettings,
    total: Rc<Cell<usize>>,
    pub loading: Rc<RefCell<Vec<PathBuf>>>,
}

impl Libraries {
    pub fn new(root: PathBuf, settings: JavascriptSettings) -> Self {
        Self {
            root,
            settings,
            total: Rc::new(Cell::new(0)),
            loading: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn resolve_path(&self, base: &Path, name: &str) -> Result<PathBuf, String> {
        if name.contains("://") || name.starts_with("data:") || name.starts_with("node:") {
            return Err("Remote libraries and built-in package imports are unsupported".into());
        }
        let path = base
            .join(name)
            .canonicalize()
            .map_err(|error| format!("library {name}: {error}"))?;
        if !path.starts_with(&self.root) {
            return Err("Library path escapes library root".into());
        }
        Ok(path)
    }

    pub fn read(&self, path: &Path) -> Result<String, String> {
        let metadata = path
            .metadata()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "{}: libraries must be regular files",
                path.display()
            ));
        }
        let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "{}: libraries must be regular files",
                path.display()
            ));
        }
        let limit = self.settings.library_kib * 1024;
        if metadata.len() > limit as u64 {
            return Err(format!(
                "{}: library file byte limit exceeded",
                path.display()
            ));
        }
        let mut code = String::new();
        file.take(limit as u64 + 1)
            .read_to_string(&mut code)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if code.len() > limit {
            return Err(format!(
                "{}: library file byte limit exceeded",
                path.display()
            ));
        }
        let total = self
            .total
            .get()
            .checked_add(code.len())
            .ok_or("Library runtime byte limit exceeded")?;
        if total > self.settings.loaded_mib * 1024 * 1024 {
            return Err("Library runtime byte limit exceeded".into());
        }
        self.total.set(total);
        Ok(code)
    }
}

impl Resolver for Libraries {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if attributes.is_some() {
            return Err(Exception::throw_message(
                ctx,
                "Module import attributes are unsupported",
            ));
        }
        if !name.starts_with("./") && !name.starts_with("../") && !Path::new(name).is_absolute() {
            return Err(Exception::throw_message(
                ctx,
                "Use a local relative module path",
            ));
        }
        let base = Path::new(base).parent().unwrap_or(&self.root);
        let path = self
            .resolve_path(base, name)
            .map_err(|error| Exception::throw_message(ctx, &error))?;
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("js" | "mjs")
        ) {
            return Err(Exception::throw_message(
                ctx,
                "Only .js and .mjs modules are supported",
            ));
        }
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| Exception::throw_message(ctx, "Module paths must be valid UTF-8"))
    }
}

impl Loader for Libraries {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let path = self
            .resolve_path(&self.root, name)
            .map_err(|error| Exception::throw_message(ctx, &error))?;
        let code = self
            .read(&path)
            .map_err(|error| Exception::throw_message(ctx, &error))?;
        Module::declare(ctx.clone(), name, code)
    }
}
