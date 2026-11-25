use crate::eval::{Arg, Node, Op, compile_node};

pub const MAX_PASSES: usize = 32;
pub const MAX_HISTORY: usize = 8;
pub const MAX_TEXTURE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_REGIONS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub enum Effect {
    Blur,
    Sharpen,
    Sobel,
    Emboss,
    Dilate,
    Erode,
}

impl Effect {
    pub(crate) fn parse(name: &str) -> Result<Self, String> {
        match name {
            "blur" => Ok(Self::Blur),
            "sharpen" => Ok(Self::Sharpen),
            "sobel" => Ok(Self::Sobel),
            "emboss" => Ok(Self::Emboss),
            "dilate" => Ok(Self::Dilate),
            "erode" => Ok(Self::Erode),
            _ => Err("Unknown neighborhood effect".into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub passes: Vec<String>,
    pub history: [usize; 4],
    pub history_limit: usize,
}

impl Default for RenderPlan {
    fn default() -> Self {
        Self {
            passes: Vec::new(),
            history: [0; 4],
            history_limit: MAX_HISTORY,
        }
    }
}

pub(crate) fn compile(node: &Node, plan: &mut RenderPlan) -> Result<String, String> {
    compile_node(&lower(node, plan, 0)?)
}

fn lower_arg(arg: &Arg, plan: &mut RenderPlan, depth: usize) -> Result<Arg, String> {
    Ok(match arg {
        Arg::Texture { node, channel } => Arg::Texture {
            node: Box::new(lower(node, plan, depth + 1)?),
            channel: *channel,
        },
        _ => arg.clone(),
    })
}

fn lower(node: &Node, plan: &mut RenderPlan, depth: usize) -> Result<Node, String> {
    if depth >= 16 {
        return Err("nesting too deep (max 16)".into());
    }
    let mut ops = Vec::new();
    for op in &node.ops {
        let op = match op {
            Op::Effect {
                other,
                effect,
                radius,
            } => {
                if !ops.is_empty() {
                    return Err("Effect must wrap a complete graph".into());
                }
                let source = compile_node(&lower(other, plan, depth + 1)?)?;
                let radius = lower_arg(radius, plan, depth)?;
                if matches!(radius, Arg::Texture { .. }) {
                    return Err("Effect radius requires a number or frame signal".into());
                }
                let input = push_pass(plan, source)?;
                let output = push_pass(plan, effect_shader(*effect, input, &radius))?;
                Op::Source {
                    func: "pass_src".into(),
                    args: vec![Arg::Lit(output as f64)],
                }
            }
            Op::Source { func, args } => {
                if func == "history_src" {
                    if let [Arg::Lit(output), Arg::Lit(delay)] = args.as_slice() {
                        if !output.is_finite()
                            || output.fract() != 0.0
                            || !(0.0..=3.0).contains(output)
                            || !delay.is_finite()
                            || delay.fract() != 0.0
                            || !(1.0..=plan.history_limit.min(MAX_HISTORY) as f64).contains(delay)
                        {
                            return Err("Invalid output history index or frame delay".into());
                        }
                        if *delay > 1.0 {
                            plan.history[*output as usize] =
                                plan.history[*output as usize].max(*delay as usize);
                        }
                    } else {
                        return Err("History requires static output and frame indices".into());
                    }
                }
                Op::Source {
                    func: func.clone(),
                    args: args
                        .iter()
                        .map(|arg| lower_arg(arg, plan, depth))
                        .collect::<Result<_, _>>()?,
                }
            }
            Op::Geo { func, args } => Op::Geo {
                func: func.clone(),
                args: args
                    .iter()
                    .map(|arg| lower_arg(arg, plan, depth))
                    .collect::<Result<_, _>>()?,
            },
            Op::Color { func, args } => Op::Color {
                func: func.clone(),
                args: args
                    .iter()
                    .map(|arg| lower_arg(arg, plan, depth))
                    .collect::<Result<_, _>>()?,
            },
            Op::Blend { func, other, args } => Op::Blend {
                func: func.clone(),
                other: lower(other, plan, depth + 1)?,
                args: args
                    .iter()
                    .map(|arg| lower_arg(arg, plan, depth))
                    .collect::<Result<_, _>>()?,
            },
            Op::Modulate { func, other, args } => Op::Modulate {
                func: func.clone(),
                other: lower(other, plan, depth + 1)?,
                args: args
                    .iter()
                    .map(|arg| lower_arg(arg, plan, depth))
                    .collect::<Result<_, _>>()?,
            },
            Op::Tileable { other, width } => Op::Tileable {
                other: lower(other, plan, depth + 1)?,
                width: lower_arg(width, plan, depth)?,
            },
        };
        ops.push(op);
    }
    Ok(Node { ops })
}

fn push_pass(plan: &mut RenderPlan, shader: String) -> Result<usize, String> {
    if plan.passes.len() >= MAX_PASSES {
        return Err("Automatic render pass limit exceeded".into());
    }
    let index = plan.passes.len();
    plan.passes.push(shader);
    Ok(index)
}

fn effect_shader(effect: Effect, input: usize, radius: &Arg) -> String {
    let radius = match radius {
        Arg::Lit(value) => format!("{value:.9}"),
        Arg::Expr(expr) => expr.clone(),
        Arg::Texture { .. } => unreachable!(),
    };
    let (initial, accumulate, result) = match effect {
        Effect::Blur => ("vec4 sum = vec4(0);", "sum += pixel;", "sum / 9.0"),
        Effect::Sharpen => (
            "vec4 sum = vec4(0);",
            "sum += pixel;",
            "vec4(center.rgb * 2.0 - sum.rgb / 9.0, center.a)",
        ),
        Effect::Emboss => (
            "vec3 sum = vec3(0);",
            "sum += pixel.rgb * float(x + y);",
            "vec4(sum * 0.25 + 0.5, center.a)",
        ),
        Effect::Sobel => (
            "float gx = 0.0; float gy = 0.0;",
            "float luma = dot(pixel.rgb,vec3(0.2126,0.7152,0.0722)); gx += luma * float(x) * (y == 0 ? 2.0 : 1.0); gy += luma * float(y) * (x == 0 ? 2.0 : 1.0);",
            "vec4(vec3(length(vec2(gx,gy))), center.a)",
        ),
        Effect::Dilate => ("vec4 sum = vec4(0);", "sum = max(sum,pixel);", "sum"),
        Effect::Erode => ("vec4 sum = vec4(1);", "sum = min(sum,pixel);", "sum"),
    };
    format!(
        "uniform sampler2D iPass{input};\nvoid mainImage(out vec4 c, in vec2 st) {{\n\
      vec2 jump = vec2(clamp({radius}, 0.0, 32.0)) / iResolution;\n\
      vec4 center = texture(iPass{input},st);\n{initial}\n\
      for(int y=-1;y<=1;y++) for(int x=-1;x<=1;x++) {{\n\
        vec4 pixel = texture(iPass{input}, clamp(st + vec2(x,y)*jump,0.0,1.0));\n{accumulate}\n\
      }}\nc = {result};\n}}"
    )
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub output: usize,
    pub rect: [f32; 4],
    pub channel: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionOutput {
    pub slot: usize,
    pub destination: String,
    pub address: String,
    pub rate: f32,
}

impl RegionOutput {
    pub fn validate(&self, regions: usize) -> Result<std::net::SocketAddr, String> {
        if self.slot >= regions
            || self.slot >= MAX_REGIONS
            || regions > MAX_REGIONS
            || !self.address.starts_with('/')
            || self.address.contains('\0')
            || self.address.len() > 1024
            || !self.rate.is_finite()
            || !(1.0..=60.0).contains(&self.rate)
        {
            return Err("Invalid region OSC destination, address, slot, or rate".into());
        }
        self.destination
            .parse()
            .map_err(|_| "OSC output requires a numeric IP address and port".into())
            .and_then(|address: std::net::SocketAddr| {
                if address.port() == 0 {
                    Err("OSC destination port must be nonzero".into())
                } else {
                    Ok(address)
                }
            })
    }
}

#[cfg(feature = "osc")]
#[derive(Default)]
pub struct RegionSender {
    bindings: Vec<(
        RegionOutput,
        std::net::UdpSocket,
        std::net::SocketAddr,
        Option<std::time::Instant>,
    )>,
}

#[cfg(feature = "osc")]
impl RegionSender {
    pub fn configure(&mut self, outputs: &[RegionOutput], regions: usize) -> Result<(), String> {
        if outputs.len() > MAX_REGIONS {
            return Err("Region OSC output limit exceeded".into());
        }
        let mut bindings = Vec::new();
        for output in outputs {
            let destination = output.validate(regions)?;
            let local = if destination.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            };
            let socket = std::net::UdpSocket::bind(local)
                .map_err(|error| format!("OSC output socket: {error}"))?;
            socket
                .set_nonblocking(true)
                .map_err(|error| format!("OSC output socket: {error}"))?;
            bindings.push((output.clone(), socket, destination, None));
        }
        self.bindings = bindings;
        Ok(())
    }

    pub fn send(&mut self, values: &[f32; MAX_REGIONS]) -> Result<(), String> {
        let now = std::time::Instant::now();
        for (output, socket, destination, last) in &mut self.bindings {
            if last.is_some_and(|last| now.duration_since(last).as_secs_f32() < 1.0 / output.rate) {
                continue;
            }
            let value = values[output.slot];
            if !value.is_finite() {
                return Err("Region OSC output must be finite".into());
            }
            let packet = rosc::OscPacket::Message(rosc::OscMessage {
                addr: output.address.clone(),
                args: vec![rosc::OscType::Float(value)],
            });
            let bytes = rosc::encoder::encode(&packet)
                .map_err(|error| format!("OSC output encoding: {error}"))?;
            socket
                .send_to(&bytes, *destination)
                .map_err(|error| format!("OSC output to {destination}: {error}"))?;
            *last = Some(now);
        }
        Ok(())
    }
}
impl Region {
    pub fn validate(&self) -> Result<(), String> {
        let [x, y, width, height] = self.rect;
        if self.output >= 4
            || self.channel > 4
            || self.rect.iter().any(|v| !v.is_finite())
            || x < 0.0
            || y < 0.0
            || width <= 0.0
            || height <= 0.0
            || x + width > 1.0
            || y + height > 1.0
        {
            return Err(
                "Region requires an output, normalized rectangle, and channel in 0..4".into(),
            );
        }
        Ok(())
    }
}
