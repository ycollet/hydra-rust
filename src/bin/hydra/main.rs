mod app;
mod highlight;

use std::path::PathBuf;

use eframe::egui;

/// Parsed CLI flags. `-i/--input` is a named alternative to the historical
/// bare positional argument (`hydra some.hydra`), kept working for
/// backward compatibility - the first unrecognized bare argument is used
/// as `input` if no flag has already set it.
#[derive(Debug, Default, PartialEq, Eq)]
struct CliArgs {
    input: Option<PathBuf>,
    bank_save: Option<PathBuf>,
    bank_load: Option<PathBuf>,
    slot_save: Option<PathBuf>,
    slot_load: Option<PathBuf>,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> CliArgs {
    let mut out = CliArgs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-i" | "--input" => out.input = args.next().map(PathBuf::from),
            "-bs" | "--bank-save" => out.bank_save = args.next().map(PathBuf::from),
            "-bl" | "--bank-load" => out.bank_load = args.next().map(PathBuf::from),
            "-ss" | "--slot-save" => out.slot_save = args.next().map(PathBuf::from),
            "-sl" | "--slot-load" => out.slot_load = args.next().map(PathBuf::from),
            other if out.input.is_none() => out.input = Some(PathBuf::from(other)),
            _ => {}
        }
    }
    out
}

fn main() -> eframe::Result {
    env_logger::init();

    let cli = parse_args(std::env::args().skip(1));

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Hydra")
            .with_maximized(true)
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Hydra",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::HydraApp::new(
                cc,
                cli.input,
                cli.bank_save,
                cli.bank_load,
                cli.slot_save,
                cli.slot_load,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_is_all_none() {
        assert_eq!(parse_args(s(&[])), CliArgs::default());
    }

    #[test]
    fn bare_positional_argument_sets_input() {
        assert_eq!(
            parse_args(s(&["some.hydra"])),
            CliArgs { input: Some("some.hydra".into()), ..Default::default() }
        );
    }

    #[test]
    fn named_input_flag_short_and_long() {
        assert_eq!(
            parse_args(s(&["-i", "a.hydra"])).input,
            Some(PathBuf::from("a.hydra"))
        );
        assert_eq!(
            parse_args(s(&["--input", "a.hydra"])).input,
            Some(PathBuf::from("a.hydra"))
        );
    }

    #[test]
    fn bank_save_and_load_flags() {
        let args = parse_args(s(&["-bs", "out.bhr", "-bl", "in.bhr"]));
        assert_eq!(args.bank_save, Some(PathBuf::from("out.bhr")));
        assert_eq!(args.bank_load, Some(PathBuf::from("in.bhr")));
    }

    #[test]
    fn long_bank_flags() {
        let args = parse_args(s(&["--bank-save", "out.bhr", "--bank-load", "in.bhr"]));
        assert_eq!(args.bank_save, Some(PathBuf::from("out.bhr")));
        assert_eq!(args.bank_load, Some(PathBuf::from("in.bhr")));
    }

    #[test]
    fn only_the_first_bare_argument_becomes_input() {
        // a second bare argument (not a recognized flag) is silently
        // ignored rather than overwriting an already-set input.
        let args = parse_args(s(&["first.hydra", "second.hydra"]));
        assert_eq!(args.input, Some(PathBuf::from("first.hydra")));
    }

    #[test]
    fn flags_can_be_combined_with_a_bare_positional_input() {
        let args = parse_args(s(&["-bl", "in.bhr", "some.hydra"]));
        assert_eq!(args.bank_load, Some(PathBuf::from("in.bhr")));
        assert_eq!(args.input, Some(PathBuf::from("some.hydra")));
    }

    #[test]
    fn slot_save_and_load_flags() {
        let args = parse_args(s(&["-ss", "out.shr", "-sl", "in.shr"]));
        assert_eq!(args.slot_save, Some(PathBuf::from("out.shr")));
        assert_eq!(args.slot_load, Some(PathBuf::from("in.shr")));
    }

    #[test]
    fn long_slot_flags() {
        let args = parse_args(s(&["--slot-save", "out.shr", "--slot-load", "in.shr"]));
        assert_eq!(args.slot_save, Some(PathBuf::from("out.shr")));
        assert_eq!(args.slot_load, Some(PathBuf::from("in.shr")));
    }

    #[test]
    fn bank_and_slot_flags_are_independent() {
        let args = parse_args(s(&["-bl", "bank.bhr", "-sl", "slot.shr"]));
        assert_eq!(args.bank_load, Some(PathBuf::from("bank.bhr")));
        assert_eq!(args.slot_load, Some(PathBuf::from("slot.shr")));
    }
}
