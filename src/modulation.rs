use rhai::{Dynamic, Engine, ImmutableString};
use serde::Deserialize;

use crate::eval::GlslExpr;

#[derive(Clone)]
pub(crate) struct Toolkit;

type RhaiResult = Result<GlslExpr, Box<rhai::EvalAltResult>>;

pub(crate) fn static_number(value: Dynamic) -> Result<f64, Box<rhai::EvalAltResult>> {
    let value = value
        .as_float()
        .ok()
        .or_else(|| value.as_int().ok().map(|v| v as f64))
        .ok_or("Modulation parameters require static numbers")?;
    number(value)?;
    Ok(value)
}

fn lfo(wave: ImmutableString, rate: Dynamic, phase: Dynamic, base: ImmutableString) -> RhaiResult {
    Ok(GlslExpr(
        Signal::Lfo {
            wave: Waveform::parse(&wave)?,
            rate: static_number(rate)?,
            phase: static_number(phase)?,
            base: Timebase::parse(&base)?,
        }
        .glsl()?,
    ))
}

fn random_signal(drift: bool, rate: Dynamic, seed: Dynamic, base: ImmutableString) -> RhaiResult {
    let rate = static_number(rate)?;
    let seed = static_number(seed)?;
    let base = Timebase::parse(&base)?;
    let signal = if drift {
        Signal::Drift { rate, seed, base }
    } else {
        Signal::SampleHold { rate, seed, base }
    };
    Ok(GlslExpr(signal.glsl()?))
}

fn clock(rate: Dynamic, duty: Dynamic, base: ImmutableString) -> RhaiResult {
    Ok(GlslExpr(
        Signal::Clock {
            rate: static_number(rate)?,
            duty: static_number(duty)?,
            base: Timebase::parse(&base)?,
        }
        .glsl()?,
    ))
}

fn envelope(attack: Dynamic, decay: Dynamic, period: Dynamic, base: ImmutableString) -> RhaiResult {
    Ok(GlslExpr(
        Signal::Envelope {
            attack: static_number(attack)?,
            decay: static_number(decay)?,
            period: static_number(period)?,
            base: Timebase::parse(&base)?,
        }
        .glsl()?,
    ))
}

pub(crate) fn register(engine: &mut Engine) {
    engine.register_fn("lfo", |_: Toolkit, wave: ImmutableString| {
        lfo(wave, 1.0.into(), 0.0.into(), "seconds".into())
    });
    engine.register_fn("lfo", |_: Toolkit, wave: ImmutableString, rate: Dynamic| {
        lfo(wave, rate, 0.0.into(), "seconds".into())
    });
    engine.register_fn(
        "lfo",
        |_: Toolkit, wave: ImmutableString, rate: Dynamic, phase: Dynamic| {
            lfo(wave, rate, phase, "seconds".into())
        },
    );
    engine.register_fn(
        "lfo",
        |_: Toolkit,
         wave: ImmutableString,
         rate: Dynamic,
         phase: Dynamic,
         base: ImmutableString| { lfo(wave, rate, phase, base) },
    );
    for (name, drift) in [("sampleHold", false), ("drift", true)] {
        engine.register_fn(name, move |_: Toolkit| {
            random_signal(drift, 1.0.into(), 0.0.into(), "seconds".into())
        });
        engine.register_fn(name, move |_: Toolkit, rate: Dynamic| {
            random_signal(drift, rate, 0.0.into(), "seconds".into())
        });
        engine.register_fn(name, move |_: Toolkit, rate: Dynamic, seed: Dynamic| {
            random_signal(drift, rate, seed, "seconds".into())
        });
        engine.register_fn(
            name,
            move |_: Toolkit, rate: Dynamic, seed: Dynamic, base: ImmutableString| {
                random_signal(drift, rate, seed, base)
            },
        );
    }
    engine.register_fn("clock", |_: Toolkit| {
        clock(1.0.into(), 0.5.into(), "seconds".into())
    });
    engine.register_fn("clock", |_: Toolkit, rate: Dynamic| {
        clock(rate, 0.5.into(), "seconds".into())
    });
    engine.register_fn("clock", |_: Toolkit, rate: Dynamic, duty: Dynamic| {
        clock(rate, duty, "seconds".into())
    });
    engine.register_fn(
        "clock",
        |_: Toolkit, rate: Dynamic, duty: Dynamic, base: ImmutableString| clock(rate, duty, base),
    );
    engine.register_fn(
        "envelope",
        |_: Toolkit, attack: Dynamic, decay: Dynamic, period: Dynamic| {
            envelope(attack, decay, period, "seconds".into())
        },
    );
    engine.register_fn(
        "envelope",
        |_: Toolkit, attack: Dynamic, decay: Dynamic, period: Dynamic, base: ImmutableString| {
            envelope(attack, decay, period, base)
        },
    );
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Timebase {
    Seconds,
    Beats,
}

impl Timebase {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "seconds" => Ok(Self::Seconds),
            "beats" => Ok(Self::Beats),
            _ => Err("Modulation timebase must be seconds or beats".into()),
        }
    }

    fn glsl(self) -> &'static str {
        match self {
            Self::Seconds => "iTime",
            Self::Beats => "(iTime * iTempo / 60.0)",
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Waveform {
    Sine,
    Triangle,
    Saw,
    Square,
}

impl Waveform {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "sine" => Ok(Self::Sine),
            "triangle" => Ok(Self::Triangle),
            "saw" => Ok(Self::Saw),
            "square" => Ok(Self::Square),
            _ => Err("Modulation waveform must be sine, triangle, saw, or square".into()),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Signal {
    Number {
        value: f64,
    },
    Lfo {
        wave: Waveform,
        rate: f64,
        phase: f64,
        base: Timebase,
    },
    Envelope {
        attack: f64,
        decay: f64,
        period: f64,
        base: Timebase,
    },
    SampleHold {
        rate: f64,
        seed: f64,
        base: Timebase,
    },
    Drift {
        rate: f64,
        seed: f64,
        base: Timebase,
    },
    Clock {
        rate: f64,
        duty: f64,
        base: Timebase,
    },
    Add {
        left: Box<Signal>,
        right: Box<Signal>,
    },
    Sub {
        left: Box<Signal>,
        right: Box<Signal>,
    },
    Mul {
        left: Box<Signal>,
        right: Box<Signal>,
    },
    Div {
        signal: Box<Signal>,
        divisor: f64,
    },
    Range {
        signal: Box<Signal>,
        low: f64,
        high: f64,
    },
}

pub(crate) fn number(value: f64) -> Result<String, String> {
    if !value.is_finite() || !(value as f32).is_finite() {
        return Err("Modulation parameters require finite graphics numbers".into());
    }
    let text = value.to_string();
    Ok(
        if text.contains('.') || text.contains('e') || text.contains('E') {
            text
        } else {
            format!("{text}.0")
        },
    )
}

fn rate(value: f64) -> Result<String, String> {
    if !(0.0001..=1000.0).contains(&value) {
        return Err("Modulation rate must be in 0.0001..1000 cycles per time unit".into());
    }
    number(value)
}

fn seed(value: f64) -> Result<String, String> {
    if !(0.0..=65535.0).contains(&value) || value.fract() != 0.0 {
        return Err("Modulation seed must be an integer in 0..65535".into());
    }
    number(value)
}

impl Signal {
    pub(crate) fn glsl(&self) -> Result<String, String> {
        self.compile(0, &mut 64)
    }

    fn compile(&self, depth: usize, remaining: &mut usize) -> Result<String, String> {
        if depth > 16 || *remaining == 0 {
            return Err("Modulation expression exceeds 64 nodes or 16 nesting levels".into());
        }
        *remaining -= 1;
        match self {
            Self::Number { value } => number(*value),
            Self::Lfo {
                wave,
                rate: frequency,
                phase,
                base,
            } => {
                let frequency = rate(*frequency)?;
                number(*phase)?;
                let position = format!(
                    "({} * {frequency} + {})",
                    base.glsl(),
                    number(phase.rem_euclid(1.0))?
                );
                Ok(match wave {
                    Waveform::Sine => {
                        format!("(sin(fract({position}) * 6.28318530718) * 0.5 + 0.5)")
                    }
                    Waveform::Triangle => format!("(1.0 - abs(fract({position}) * 2.0 - 1.0))"),
                    Waveform::Saw => format!("fract({position})"),
                    Waveform::Square => format!("(1.0 - step(0.5, fract({position})))"),
                })
            }
            Self::Envelope {
                attack,
                decay,
                period,
                base,
            } => {
                // Decimal durations can round above an otherwise equal period.
                let rounding = period.abs() * f64::EPSILON * 2.0;
                if !(0.0..=1_000_000.0).contains(attack)
                    || !(0.0..=1_000_000.0).contains(decay)
                    || !(0.0001..=1_000_000.0).contains(period)
                    || attack + decay <= 0.0
                    || attack + decay > period + rounding
                    || (*attack > 0.0 && *attack < 0.0001)
                    || (*decay > 0.0 && *decay < 0.0001)
                {
                    return Err("Modulation envelope requires nonnegative attack/decay and period >= their positive sum; nonzero durations must be in 0.0001..1000000".into());
                }
                Ok(format!(
                    "_hydra_mod_envelope({}, {}, {}, {})",
                    base.glsl(),
                    number(*attack)?,
                    number(*decay)?,
                    number(*period)?
                ))
            }
            Self::SampleHold {
                rate: frequency,
                seed: value,
                base,
            } => Ok(format!(
                "_hydra_mod_random(floor({} * {}), {})",
                base.glsl(),
                rate(*frequency)?,
                seed(*value)?
            )),
            Self::Drift {
                rate: frequency,
                seed: value,
                base,
            } => Ok(format!(
                "_hydra_mod_drift({} * {}, {})",
                base.glsl(),
                rate(*frequency)?,
                seed(*value)?
            )),
            Self::Clock {
                rate: frequency,
                duty,
                base,
            } => {
                if !(0.0..=1.0).contains(duty) {
                    return Err("Modulation clock duty must be in 0..1".into());
                }
                Ok(format!(
                    "(1.0 - step({}, fract({} * {})))",
                    number(*duty)?,
                    base.glsl(),
                    rate(*frequency)?
                ))
            }
            Self::Add { left, right } | Self::Sub { left, right } | Self::Mul { left, right } => {
                let op = match self {
                    Self::Add { .. } => "+",
                    Self::Sub { .. } => "-",
                    _ => "*",
                };
                Ok(format!(
                    "({} {op} {})",
                    left.compile(depth + 1, remaining)?,
                    right.compile(depth + 1, remaining)?
                ))
            }
            Self::Div { signal, divisor } => {
                let divisor_text = number(*divisor)?;
                if *divisor == 0.0 || (*divisor as f32) == 0.0 {
                    return Err("Modulation divisor must be nonzero at graphics precision".into());
                }
                Ok(format!(
                    "({} / {divisor_text})",
                    signal.compile(depth + 1, remaining)?
                ))
            }
            Self::Range { signal, low, high } => Ok(format!(
                "(({}) * ({} - {}) + {})",
                signal.compile(depth + 1, remaining)?,
                number(*high)?,
                number(*low)?,
                number(*low)?
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_compilation_checks_expression_size_even_without_the_javascript_builder() {
        fn tree(depth: usize) -> Signal {
            if depth == 0 {
                Signal::Number { value: 1.0 }
            } else {
                Signal::Add {
                    left: Box::new(tree(depth - 1)),
                    right: Box::new(tree(depth - 1)),
                }
            }
        }
        assert!(tree(5).glsl().is_ok());
        assert!(tree(6).glsl().unwrap_err().contains("64 nodes"));
        let mut signal = Signal::Number { value: 1.0 };
        for _ in 0..16 {
            signal = Signal::Range {
                signal: Box::new(signal),
                low: 0.0,
                high: 1.0,
            };
        }
        assert!(signal.glsl().is_ok());
        signal = Signal::Range {
            signal: Box::new(signal),
            low: 0.0,
            high: 1.0,
        };
        assert!(signal.glsl().unwrap_err().contains("16 nesting"));
    }
}
