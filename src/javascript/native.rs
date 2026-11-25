use serde::Deserialize;

use crate::eval::EvalResult;

#[cfg(feature = "fft")]
pub(super) fn audio_settings(settings: &crate::audio::AudioSettings) -> serde_json::Value {
    serde_json::json!({
        "bins": settings.bins,
        "defaults": {"cutoff":settings.defaults.cutoff,"scale":settings.defaults.scale,"smooth":settings.defaults.smooth},
        "settings": settings.settings.iter().map(|bin|
            serde_json::json!({"cutoff":bin.cutoff,"scale":bin.scale,"smooth":bin.smooth})
        ).collect::<Vec<_>>()
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Command {
    #[cfg(feature = "webcam")]
    Cam { slot: usize, index: u32 },
    #[cfg(feature = "image_url")]
    Image { slot: usize, url: String },
    #[cfg(feature = "image_url")]
    Gif { slot: usize, url: String },
    #[cfg(feature = "video")]
    Video { slot: usize, url: String },
    #[cfg(feature = "stream")]
    Stream { slot: usize, addr: String },
    #[cfg(feature = "fft")]
    Audio {
        method: String,
        value: Option<f32>,
        index: Option<usize>,
    },
    #[cfg(feature = "midi")]
    MidiControl { method: String },
    #[cfg(feature = "midi")]
    MidiBind {
        slot: usize,
        value: String,
        index: Option<u8>,
        channel: Option<u8>,
        input: Option<Input>,
        smooth: Option<f32>,
    },
    #[cfg(feature = "midi")]
    MidiAdsr {
        slot: usize,
        note: i64,
        channel: Option<u8>,
        input: Option<Input>,
        adsr: [f32; 4],
    },
    #[cfg(feature = "osc")]
    OscControl { method: String, port: Option<u16> },
    #[cfg(feature = "osc")]
    OscBind {
        slot: usize,
        address: String,
        index: usize,
    },
    #[serde(other)]
    Unsupported,
}

#[cfg(feature = "midi")]
#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Input {
    Index(usize),
    Name(String),
}

#[cfg(feature = "midi")]
fn filter(channel: Option<u8>, input: &Option<Input>) -> Result<crate::midi::MidiFilter, String> {
    if channel.is_some_and(|channel| channel > 15) {
        return Err("MIDI channel must be in 0..15".into());
    }
    Ok(crate::midi::MidiFilter {
        channel,
        input: match input {
            None => crate::midi::InputFilter::Any,
            Some(Input::Index(index)) => crate::midi::InputFilter::Index(*index),
            Some(Input::Name(name)) if !name.is_empty() => {
                crate::midi::InputFilter::Name(name.clone())
            }
            _ => return Err("MIDI input name must not be empty".into()),
        },
    })
}

pub(super) fn apply(commands: &[Command], _result: &mut EvalResult) -> Result<(), String> {
    #[cfg(any(
        feature = "webcam",
        feature = "image_url",
        feature = "video",
        feature = "stream",
        feature = "fft",
        feature = "midi",
        feature = "osc"
    ))]
    let result = _result;
    if commands.len() > 256 {
        return Err("Native request limit exceeded".into());
    }
    #[cfg(not(any(
        feature = "webcam",
        feature = "image_url",
        feature = "video",
        feature = "stream",
        feature = "fft",
        feature = "midi",
        feature = "osc"
    )))]
    if !commands.is_empty() {
        return Err("Native requests require their matching Cargo features".into());
    }
    #[cfg(any(
        feature = "webcam",
        feature = "image_url",
        feature = "video",
        feature = "stream",
        feature = "fft",
        feature = "midi",
        feature = "osc"
    ))]
    for command in commands {
        match command {
            #[cfg(feature = "webcam")]
            Command::Cam { slot, index } if *slot < 4 => {
                result
                    .source_requests
                    .push(crate::eval::SourceRequest::InitCam {
                        slot: *slot,
                        camera_index: *index,
                    })
            }
            #[cfg(feature = "image_url")]
            Command::Image { slot, url } if *slot < 4 && !url.is_empty() => result
                .source_requests
                .push(crate::eval::SourceRequest::InitImage {
                    slot: *slot,
                    url: url.clone(),
                }),
            #[cfg(feature = "image_url")]
            Command::Gif { slot, url } if *slot < 4 && !url.is_empty() => result
                .source_requests
                .push(crate::eval::SourceRequest::InitGif {
                    slot: *slot,
                    url: url.clone(),
                }),
            #[cfg(feature = "video")]
            Command::Video { slot, url } if *slot < 4 && !url.is_empty() => result
                .source_requests
                .push(crate::eval::SourceRequest::InitVideo {
                    slot: *slot,
                    url: url.clone(),
                }),
            #[cfg(feature = "stream")]
            Command::Stream { slot, addr } if *slot < 4 && !addr.is_empty() => result
                .source_requests
                .push(crate::eval::SourceRequest::InitStream {
                    slot: *slot,
                    addr: addr.clone(),
                }),
            #[cfg(feature = "fft")]
            Command::Audio {
                method,
                value,
                index,
            } => {
                use crate::audio::{AudioSettings, BinProperty, NUM_FFT_BINS};
                use crate::eval::AudioRequest;
                let request = match method.as_str() {
                    "show" if value.is_none() && index.is_none() => AudioRequest::Show,
                    "hide" if value.is_none() && index.is_none() => AudioRequest::Hide,
                    "setBins" if index.is_none() => {
                        let n = value.ok_or("Missing audio bin count")?;
                        if !n.is_finite() || n.fract() != 0.0 || n < 1.0 || n > NUM_FFT_BINS as f32
                        {
                            return Err("Audio bin count must be in 1..8".into());
                        }
                        AudioRequest::SetBins(n as usize)
                    }
                    "cutoff" | "scale" | "smooth" => {
                        let value = value.ok_or("Missing audio setting")?;
                        let property = match method.as_str() {
                            "cutoff" => BinProperty::Cutoff,
                            "scale" => BinProperty::Scale,
                            _ => BinProperty::Smooth,
                        };
                        let mut settings = AudioSettings::default();
                        settings.set_bins(NUM_FFT_BINS);
                        settings.set(*index, property, value)?;
                        match index {
                            Some(index) => AudioRequest::SetBin {
                                index: *index,
                                property,
                                value,
                            },
                            None => match property {
                                BinProperty::Cutoff => AudioRequest::SetCutoff(value),
                                BinProperty::Scale => AudioRequest::SetScale(value),
                                BinProperty::Smooth => AudioRequest::SetSmooth(value),
                            },
                        }
                    }
                    _ => return Err("Invalid audio request".into()),
                };
                result.audio_requests.push(request);
            }
            #[cfg(feature = "midi")]
            Command::MidiControl { method } => {
                use crate::eval::MidiRequest;
                result.midi_requests.push(match method.as_str() {
                    "start" => MidiRequest::Start,
                    "pause" => MidiRequest::Pause,
                    "show" => MidiRequest::Show,
                    "hide" => MidiRequest::Hide,
                    _ => return Err("Invalid MIDI control".into()),
                });
            }
            #[cfg(feature = "midi")]
            Command::MidiBind {
                slot,
                value,
                index,
                channel,
                input,
                smooth,
            } => {
                use crate::midi::{MidiBinding, MidiValueKind, NUM_MIDI_VALUES};
                if *slot >= NUM_MIDI_VALUES
                    || index.is_some_and(|index| index > 127)
                    || smooth
                        .is_some_and(|smooth| !smooth.is_finite() || !(0.0..=1.0).contains(&smooth))
                {
                    return Err("Invalid MIDI binding".into());
                }
                let kind = match value.as_str() {
                    "note" => MidiValueKind::Note,
                    "velocity" => MidiValueKind::Velocity,
                    "cc" => MidiValueKind::Cc,
                    "aft" if index.is_none() => MidiValueKind::ChannelAftertouch,
                    "aft" => MidiValueKind::Aftertouch,
                    _ => return Err("Invalid MIDI value".into()),
                };
                if smooth.is_some() && kind != MidiValueKind::Cc {
                    return Err("Only MIDI controller values support smoothing".into());
                }
                result.midi_requests.push(crate::eval::MidiRequest::Bind {
                    slot: *slot,
                    binding: MidiBinding {
                        kind,
                        index: *index,
                        filter: filter(*channel, input)?,
                        smooth: *smooth,
                    },
                });
            }
            #[cfg(feature = "midi")]
            Command::MidiAdsr {
                slot,
                note,
                channel,
                input,
                adsr,
            } => {
                if *slot >= crate::midi::NUM_MIDI_ENVELOPES
                    || !(-1..=127).contains(note)
                    || adsr.iter().any(|v| !v.is_finite() || *v < 0.0)
                    || adsr[2] > 1.0
                {
                    return Err("Invalid MIDI envelope".into());
                }
                result
                    .midi_requests
                    .push(crate::eval::MidiRequest::FilteredAdsrSlot {
                        slot: *slot,
                        note: *note,
                        filter: filter(*channel, input)?,
                        adsr: *adsr,
                    });
            }
            #[cfg(feature = "osc")]
            Command::OscControl { method, port } => {
                use crate::eval::OscRequest;
                result.osc_requests.push(match method.as_str() {
                    "start" if port != &Some(0) => {
                        OscRequest::Start(port.unwrap_or(crate::osc::DEFAULT_OSC_PORT))
                    }
                    "pause" if port.is_none() => OscRequest::Pause,
                    "show" if port.is_none() => OscRequest::Show,
                    "hide" if port.is_none() => OscRequest::Hide,
                    _ => return Err("Invalid OSC control".into()),
                });
            }
            #[cfg(feature = "osc")]
            Command::OscBind {
                slot,
                address,
                index,
            } if *slot < crate::osc::NUM_OSC_SLOTS
                && address.starts_with('/')
                && address.len() <= 1024
                && *index < 256 =>
            {
                result.osc_requests.push(crate::eval::OscRequest::Bind {
                    slot: *slot,
                    address: address.clone(),
                    arg_index: *index,
                });
            }
            _ => return Err("Invalid native request or required feature is disabled".into()),
        }
    }
    Ok(())
}
