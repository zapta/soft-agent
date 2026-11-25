//! debug_build: the engine's build with the debugger inserted
//! (docs/debugger.md, Build integration).
//!
//! 1. A yosys pre-pass on the user's sources flattens the design and
//!    exposes the debug file's nets as ports of the top module (`expose`,
//!    and `expose -cut` for nets the debugger drives), writing
//!    debug_src/design_debug.v.
//! 2. soft-agent writes debug_src/debug_top.v (the design, the debugger and
//!    the override muxes), debug_src/debug.pcf (the user's constraints plus
//!    the UART pins) and debug_src/logix_debugger.v.
//! 3. The engine's own build tool builds those into the build folder, and
//!    debug_map.json goes next to the bitstream.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use logix_engine::catalog::{Family, VariantChoice};
use logix_engine::errors::{Category, Diagnostic, ToolError};
use logix_engine::state::EngineState;
use logix_engine::tools::build as engine_build;
use logix_engine::tools::{Call, EngineTool};

use super::config::{self, DebugConfig, OutputMode};
use super::protocol::{self, DebugMap, MapInput, MapOutput};

/// The debugger's Verilog, written into each debug build.
const DEBUGGER_V: &str = include_str!("hdl/logix_debugger.v");

/// The folder in the build folder for the generated sources.
const GEN_FOLDER: &str = "debug_src";
/// The generated top module, and the debugger's UART ports.
const TOP: &str = "logix_debug_top";
const TX_PORT: &str = "logix_debug_tx";
const RX_PORT: &str = "logix_debug_rx";

pub struct DebugBuild;

impl EngineTool for DebugBuild {
    const NAME: &'static str = "debug_build";
    const DESCRIPTION: &'static str = "Builds an FPGA design with the Logix debugger in it, so a \
        host can read and drive the design's nets on the board while it runs (debug_open, \
        debug_read, debug_write). Takes the same fields as build, plus debug_file: a JSONC \
        file in the source folder naming the nets to read (input) and drive (output), by \
        instance path such as u_ctrl.state, and the clock the debugger runs on. The debugger \
        talks over the board's UART, so the design must not use the UART pins. The design's \
        own sources are unchanged; a normal build of them has no debugger. Writes \
        debug_map.json into the build folder, which the debug tools read. Version 1 supports \
        iCE40 boards. After it, program_board programs the bitstream as for any build.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = DebugBuildInput;
    type Output = DebugBuildOutput;
}

/// What to build, as for build, plus the debug file.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugBuildInput {
    /// The target board's ID, as for build.
    pub board: String,
    /// The board variant, as for build.
    #[serde(default)]
    pub board_variant: Option<VariantChoice>,
    /// The host's own boards folder, as for build.
    #[serde(default)]
    pub custom_boards_folder: Option<String>,
    /// Name of the design's top-level module, such as main.
    pub top_module: String,
    /// The folder with the design files, as an absolute path.
    pub source_folder: String,
    /// The HDL files to build (.v or .sv), relative to the source folder.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The constraint file (.pcf), relative to the source folder.
    pub constraint_file: String,
    /// The debug configuration file, relative to the source folder, such as
    /// debug.jsonc.
    pub debug_file: String,
    /// Where the build writes its files, as an absolute path; as for build.
    /// Use a folder of its own, such as _build_debug, so the normal build
    /// keeps its own.
    pub build_folder: String,
    /// Verilog macro definitions, as for build.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// Let the tools print more detail. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// The debug build's result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugBuildOutput {
    /// The engine build's result, as build returns it (its report covers
    /// the design and the debugger together).
    pub build: Value,
    /// The debug map, relative to the build folder: debug_map.json.
    pub debug_map: String,
    /// The build ID the debugger reports, 8 hex digits.
    pub build_id: String,
    /// The nets the debugger reads.
    pub inputs: Vec<MapInput>,
    /// The nets the debugger drives.
    pub outputs: Vec<MapOutput>,
    /// The debugger's own share, synthesized alone.
    pub debugger: DebuggerUse,
}

/// The debugger's resources, from synthesizing it alone (an estimate:
/// placing it with the design can pack it a little differently).
#[derive(Debug, Serialize, JsonSchema)]
pub struct DebuggerUse {
    pub luts: u32,
    pub flip_flops: u32,
    /// For people, such as "The debugger uses about 460 LUTs and 250
    /// flip-flops of that."
    pub report: String,
}

pub async fn run(
    state: &EngineState,
    input: DebugBuildInput,
    call: &Call,
) -> Result<DebugBuildOutput, ToolError> {
    // The request, checked as build checks it.
    engine_build::check_top_module(&input.top_module)?;
    engine_build::check_defines(&input.defines)?;
    let src = engine_build::check_source_folder(&input.source_folder)?;
    if input.source_files.is_empty() {
        return Err(ToolError::request("source_files is empty."));
    }
    for f in &input.source_files {
        engine_build::check_source_file(&src, "source_files", f)?;
    }
    engine_build::check_source_file(&src, "constraint_file", &input.constraint_file)?;
    engine_build::check_source_file(&src, "debug_file", &input.debug_file)?;
    let out = engine_build::check_build_folder(&input.build_folder, &src)?;

    let config_text = read(&src.join(&input.debug_file), "the debug file")?;
    let config = config::parse(&config_text)
        .and_then(|c| in_top(c, &input.top_module))
        .map_err(|e| file_error(&input.debug_file, &e))?;

    state.ensure_catalog(call).await?;
    let custom = logix_engine::custom_boards::load(input.custom_boards_folder.as_deref())?;
    let board = logix_engine::board::resolve(&input.board, &input.board_variant, &custom)?;
    if board.fpga.family != Family::Ice40 {
        return Err(ToolError::request(format!(
            "debug_build supports iCE40 boards only in version 1, and {} is another family.",
            board.name
        )));
    }
    if !input.constraint_file.ends_with(".pcf") {
        return Err(ToolError::request(
            "constraint_file must be a .pcf file for an iCE40 board.",
        ));
    }
    let pcf_text = read(&src.join(&input.constraint_file), "the constraint file")?;
    let pins = pcf_pins(&pcf_text);
    let link = uplink(&config, &board, &pins, &input.constraint_file)?;

    // The toolchain, as the engine's build gets it.
    let needed = state
        .packages
        .needing_download(&[logix_engine::packages::CORE])
        .await?;
    if !needed.is_empty() {
        let phases = logix_engine::progress::Phases::new(call.progress.clone(), needed.len());
        state
            .packages
            .ensure(&needed, state.auto_download, &phases, &call.cancel)
            .await?;
    }
    let bin = state.packages.bin_dir(logix_engine::build::TOOLCHAIN)?;

    let gen_dir = out.join(GEN_FOLDER);
    std::fs::create_dir_all(&gen_dir)
        .map_err(|e| ToolError::engine(format!("Can't create {}: {e}", gen_dir.display())))?;

    // 1. The pre-pass.
    let pin_ports = pin_ports(&config, &pins, &input.constraint_file)?;
    let nets: Vec<_> = unique_nets(&config)
        .into_iter()
        .filter(|(n, _)| !pin_ports.contains_key(n))
        .collect();
    let mut design = prepass(&input, &nets, &src, &gen_dir, &bin, call).await?;
    for pin in pin_ports.keys() {
        design.widths.insert(pin.clone(), 1);
    }
    design.pin_ports = pin_ports;

    // 2. The layout, the build ID and the generated files.
    let mut map = layout(&input, &config, &design, &link)?;
    let design_v = read(&gen_dir.join("design_debug.v"), "the pre-pass output")?;
    let mut hasher = Sha256::new();
    hasher.update(design_v.as_bytes());
    hasher.update(serde_json::to_string(&map).unwrap_or_default().as_bytes());
    let digest = hasher.finalize();
    let id = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    map.build_id = format!("{id:08x}");

    let top_v = debug_top(&input.top_module, &design, &map, &link, id);
    write(&gen_dir.join("debug_top.v"), &top_v)?;
    // The map names nets as the debug file does, from the top module.
    for i in &mut map.inputs {
        if !i.net.starts_with(PIN_PREFIX) {
            i.net = format!("{}.{}", input.top_module, i.net);
        }
    }
    for o in &mut map.outputs {
        o.net = format!("{}.{}", input.top_module, o.net);
    }
    write(&gen_dir.join("logix_debugger.v"), DEBUGGER_V)?;
    let pcf = format!(
        "{}\n# Added by debug_build: the debugger's UART.\nset_io {TX_PORT} {}\nset_io {RX_PORT} {}\n",
        pcf_text.trim_end(),
        link.tx,
        link.rx
    );
    write(&gen_dir.join("debug.pcf"), &pcf)?;

    // 3. The engine's build of the generated files.
    let mut args = serde_json::Map::new();
    args.insert("board".into(), input.board.clone().into());
    if let Some(v) = &input.board_variant {
        args.insert(
            "board_variant".into(),
            serde_json::to_value(v).unwrap_or_default(),
        );
    }
    if let Some(c) = &input.custom_boards_folder {
        args.insert("custom_boards_folder".into(), c.clone().into());
    }
    args.insert("top_module".into(), TOP.into());
    args.insert(
        "source_folder".into(),
        gen_dir.to_string_lossy().to_string().into(),
    );
    args.insert(
        "source_files".into(),
        serde_json::json!(["design_debug.v", "debug_top.v", "logix_debugger.v"]),
    );
    args.insert("constraint_file".into(), "debug.pcf".into());
    args.insert(
        "build_folder".into(),
        out.to_string_lossy().to_string().into(),
    );
    args.insert("verbose".into(), input.verbose.into());
    let result = logix_engine::tools::call(state, "build", Some(args), call)
        .await
        .ok_or_else(|| ToolError::engine("The engine has no build tool."))?;
    let value = result.structured_content.clone().unwrap_or_default();
    if result.is_error == Some(true) {
        return Err(serde_json::from_value::<ToolError>(value.clone())
            .unwrap_or_else(|_| ToolError::engine(format!("The engine's build failed: {value}"))));
    }

    write(
        &out.join(protocol::MAP_FILE),
        &(serde_json::to_string_pretty(&map).unwrap_or_default() + "\n"),
    )?;
    let debugger = debugger_use(&map, &link, &gen_dir, &bin, call).await;

    Ok(DebugBuildOutput {
        build: value,
        debug_map: protocol::MAP_FILE.into(),
        build_id: map.build_id.clone(),
        inputs: map.inputs,
        outputs: map.outputs,
        debugger,
    })
}

/// What a path to a package pin starts with: pin.18 is the pin the
/// constraint file puts at 18.
const PIN_PREFIX: &str = "pin.";

/// For each pin.<pin> input, the top-level port bit the constraint file
/// puts on that pin, as a Verilog expression such as sw[0].
fn pin_ports(
    config: &DebugConfig,
    pins: &BTreeMap<String, String>,
    pcf_name: &str,
) -> Result<BTreeMap<String, String>, ToolError> {
    let mut out = BTreeMap::new();
    for i in &config.input {
        let Some(pin) = i.net.strip_prefix(PIN_PREFIX) else {
            continue;
        };
        let port = pins
            .iter()
            .find(|(_, p)| p.as_str() == pin)
            .map(|(port, _)| port.clone())
            .ok_or_else(|| {
                ToolError::new(
                    Category::Design,
                    format!(
                        "{}: {pcf_name} puts no port on pin {pin}. A pin path reads the port the constraint file places there.",
                        i.net
                    ),
                )
            })?;
        let expr = match port.split_once('[') {
            Some((base, bit)) => format!("{}[{bit}", vid(base)),
            None => vid(&port),
        };
        out.insert(i.net.clone(), expr);
    }
    Ok(out)
}

/// The debug file's nets without the top module's name, which every path
/// starts with (main.u_ctrl.state), as the pre-pass names them
/// (u_ctrl.state). Host names default to the whole path.
fn in_top(mut config: DebugConfig, top: &str) -> Result<DebugConfig, String> {
    if top == "pin" {
        return Err(
            "The top module is called pin, which debug paths use for package pins (pin.18): rename the top module to debug it."
                .into(),
        );
    }
    for o in &config.output {
        if o.net.starts_with(PIN_PREFIX) {
            return Err(format!(
                "{}: version 1 reads pins but can't drive them. Drive an internal net instead.",
                o.net
            ));
        }
    }
    let strip = |net: &mut String, name: &mut Option<String>| -> Result<(), String> {
        if net.starts_with(PIN_PREFIX) {
            name.get_or_insert_with(|| net.clone());
            return Ok(());
        }
        let rest = net
            .strip_prefix(top)
            .and_then(|r| r.strip_prefix('.'))
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
                format!(
                    "The net {net:?} must start with the top module's name, {top}: write {top}.{net}."
                )
            })?;
        let rest = rest.to_string();
        name.get_or_insert_with(|| net.clone());
        *net = rest;
        Ok(())
    };
    for i in &mut config.input {
        strip(&mut i.net, &mut i.name)?;
    }
    for o in &mut config.output {
        strip(&mut o.net, &mut o.name)?;
    }
    Ok(config)
}

// --- The uplink -------------------------------------------------------------

struct Link {
    tx: String,
    rx: String,
    baud: u32,
    clock_mhz: f64,
    clks_per_bit: u32,
}

/// The UART's pins and speed, and the clock's frequency, checked.
fn uplink(
    config: &DebugConfig,
    board: &logix_engine::board::ResolvedBoard,
    pins: &BTreeMap<String, String>,
    pcf_name: &str,
) -> Result<Link, ToolError> {
    let resources = serde_json::to_value(&board.resources).unwrap_or_default();
    let of_type = |t: &str| {
        resources
            .as_object()
            .and_then(|m| m.values().find(|r| r["type"] == t).cloned())
    };
    let up = &config.uplink;
    let (tx, rx) = match &up.resource {
        Some(name) => {
            let uarts: Vec<String> = resources
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter(|(_, r)| r["type"] == "uart")
                        .map(|(k, _)| k.clone())
                        .collect()
                })
                .unwrap_or_default();
            let known = if uarts.is_empty() {
                format!(
                    "{} has no UART in the catalog: give tx and rx pins instead",
                    board.name
                )
            } else {
                format!("{}'s UARTs: {}", board.name, uarts.join(", "))
            };
            let r = &resources[name.as_str()];
            if r.is_null() {
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "uplink.resource {name:?} isn't a resource of {}. {known}.",
                        board.name
                    ),
                ));
            }
            if r["type"] != "uart" {
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "uplink.resource {name:?} is of type {}, not uart. {known}.",
                        r["type"].as_str().unwrap_or("resource")
                    ),
                ));
            }
            let pin = |f: &str| r[f].as_str().map(str::to_string);
            match (pin("tx"), pin("rx")) {
                (Some(tx), Some(rx)) => (tx, rx),
                _ => {
                    return Err(ToolError::request(format!(
                        "The catalog's {name} on {} lacks its tx or rx pin: give tx and rx instead.",
                        board.name
                    )));
                }
            }
        }
        None => (
            up.tx.clone().unwrap_or_default(),
            up.rx.clone().unwrap_or_default(),
        ),
    };
    let baud = up.baud.unwrap_or(115_200);
    if tx == rx {
        return Err(ToolError::request(
            "uplink.tx and uplink.rx are the same pin.",
        ));
    }
    // Decided (Q6): the design can't use the debugger's UART pins.
    for (port, pin) in pins {
        if *pin == tx || *pin == rx {
            return Err(ToolError::new(
                Category::Design,
                format!(
                    "{pcf_name} puts {port} on pin {pin}, which the debugger's UART needs. Version 1 can't share the UART with the design: remove {port} from the debug build's design, or give the debugger other pins in uplink."
                ),
            ));
        }
    }
    // The clock: the board's, when the clock port is on its pin.
    let Some(clock_pin) = pins.get(&config.clock) else {
        return Err(ToolError::new(
            Category::Design,
            format!(
                "The debug file's clock {:?} isn't a port in {pcf_name}: name the design's clock input.",
                config.clock
            ),
        ));
    };
    let board_clock = of_type("clock")
        .filter(|c| c["pin"].as_str() == Some(clock_pin.as_str()))
        .and_then(|c| c["mhz"].as_f64());
    let clock_mhz = match (config.clock_mhz, board_clock) {
        (Some(m), _) => m,
        (None, Some(m)) => m,
        (None, None) => {
            return Err(ToolError::request(format!(
                "The clock {} is on pin {clock_pin}, which isn't the board's clock in the catalog: give its frequency as clock_mhz in the debug file.",
                config.clock
            )));
        }
    };
    let ratio = clock_mhz * 1e6 / baud as f64;
    let clks_per_bit = ratio.round() as u32;
    let off = (ratio - clks_per_bit as f64).abs() / ratio;
    if clks_per_bit < 8 || off > 0.02 {
        return Err(ToolError::request(format!(
            "A {clock_mhz} MHz clock can't make {baud} baud closely enough: choose another baud in uplink."
        )));
    }
    Ok(Link {
        tx,
        rx,
        baud,
        clock_mhz,
        clks_per_bit,
    })
}

/// Port to pin, from a .pcf's set_io lines.
pub fn pcf_pins(text: &str) -> BTreeMap<String, String> {
    let mut pins = BTreeMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"set_io") {
            continue;
        }
        let rest: Vec<&str> = words[1..]
            .iter()
            .copied()
            .filter(|w| !w.starts_with('-'))
            .collect();
        if let [port, pin, ..] = rest.as_slice() {
            pins.insert(port.to_string(), pin.to_string());
        }
    }
    pins
}

// --- The pre-pass -----------------------------------------------------------

/// Every net the file names, once, with whether the debugger drives it and
/// how.
fn unique_nets(config: &DebugConfig) -> Vec<(String, Option<OutputMode>)> {
    let mut nets: Vec<(String, Option<OutputMode>)> = Vec::new();
    for i in &config.input {
        if !nets.iter().any(|(n, _)| *n == i.net) {
            nets.push((i.net.clone(), None));
        }
    }
    for o in &config.output {
        match nets.iter_mut().find(|(n, _)| *n == o.net) {
            Some((_, m)) => *m = Some(o.mode),
            None => nets.push((o.net.clone(), Some(o.mode))),
        }
    }
    nets
}

/// A top-level port of the design after the pre-pass.
#[derive(Debug, Clone)]
struct Port {
    name: String,
    direction: String,
    width: usize,
    /// Its declared range, such as "[3:0] ", or "" for one bit.
    range: String,
}

struct Design {
    /// The top module's own ports, as the user's top declares them.
    ports: Vec<Port>,
    /// Widths of the nets the file names.
    widths: BTreeMap<String, usize>,
    /// The pin.<pin> inputs' port bits (pin_ports).
    pin_ports: BTreeMap<String, String>,
}

/// A yosys select pattern for a net. Brackets are wildcards in patterns, so
/// they become ?, and the count check catches any other match.
fn pattern(net: &str) -> String {
    format!("w:{}", net.replace(['[', ']'], "?"))
}

async fn prepass(
    input: &DebugBuildInput,
    nets: &[(String, Option<OutputMode>)],
    src: &Path,
    gen_dir: &Path,
    bin: &Path,
    call: &Call,
) -> Result<Design, ToolError> {
    let q = |p: &Path| format!("\"{}\"", p.to_string_lossy());
    let mut script = format!(
        "read_verilog -lib +/ice40/cells_sim.v; hierarchy -top {top}; proc; flatten; \
         select {top}; write_json -selected {pre}; select -clear; ",
        top = input.top_module,
        pre = q(&gen_dir.join("pre.json"))
    );
    for (net, _) in nets {
        let _ = write!(script, "select -assert-count 1 {}; ", pattern(net));
    }
    let reads: Vec<String> = nets.iter().map(|(n, _)| pattern(n)).collect();
    let _ = write!(script, "expose {}; ", reads.join(" "));
    let cuts: Vec<String> = nets
        .iter()
        .filter(|(_, m)| m.is_some())
        .map(|(n, _)| pattern(n))
        .collect();
    if !cuts.is_empty() {
        let _ = write!(script, "expose -cut {}; ", cuts.join(" "));
    }
    let _ = write!(
        script,
        "write_verilog -noattr {v}; select {top}; write_json -selected {post}",
        v = q(&gen_dir.join("design_debug.v")),
        top = input.top_module,
        post = q(&gen_dir.join("post.json"))
    );
    let mut args: Vec<OsString> = vec![
        "-l".into(),
        gen_dir.join("prepass.log").into(),
        "-p".into(),
        script.into(),
        "-DSYNTHESIZE".into(),
    ];
    if !input.verbose {
        args.insert(0, "-q".into());
    }
    for (name, value) in &input.defines {
        args.push(format!("-D{name}={value}").into());
    }
    args.extend(input.source_files.iter().map(OsString::from));
    let program = logix_engine::commands::program_in(bin, "yosys");
    let run = logix_engine::runner::run_tool(&program, &args, src, bin, &call.cancel)
        .await
        .map_err(|e| match e {
            logix_engine::runner::RunError::Cancelled => {
                ToolError::cancelled("The debug build was cancelled.")
            }
            logix_engine::runner::RunError::Spawn(e) => {
                ToolError::setup(format!("Can't run yosys: {e}"))
            }
        })?;
    if !run.status.success() {
        return Err(prepass_error(&run.output, nets, input, src));
    }

    let pre = read_json(&gen_dir.join("pre.json"))?;
    let post = read_json(&gen_dir.join("post.json"))?;
    let top = &input.top_module;
    let own: Vec<String> = pre["modules"][top]["ports"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let module = &post["modules"][top];
    let port = |name: &str| -> Option<Port> {
        let p = &module["ports"][name];
        let direction = p["direction"].as_str()?.to_string();
        let width = p["bits"].as_array()?.len();
        let n = &module["netnames"][name];
        let offset = n["offset"].as_u64().unwrap_or(0) as usize;
        let upto = n["upto"].as_u64() == Some(1);
        let range = if width == 1 && offset == 0 {
            String::new()
        } else if upto {
            format!("[{}:{}] ", offset, offset + width - 1)
        } else {
            format!("[{}:{}] ", offset + width - 1, offset)
        };
        Some(Port {
            name: name.to_string(),
            direction,
            width,
            range,
        })
    };
    let ports: Vec<Port> = own.iter().filter_map(|n| port(n)).collect();
    let mut widths = BTreeMap::new();
    for (net, mode) in nets {
        let is_port = own.contains(net);
        if is_port && mode.is_some() {
            return Err(ToolError::new(
                Category::Design,
                format!(
                    "The debug file drives {}.{net}, a port of the top module: version 1 can drive internal nets only.",
                    input.top_module
                ),
            ));
        }
        let found = port(net).filter(|p| is_port || p.direction == "output");
        let cut_ok = mode.is_none() || port(&format!("{net}.i")).is_some();
        match found {
            Some(p) if cut_ok => {
                widths.insert(net.clone(), p.width);
            }
            _ => {
                return Err(ToolError::new(
                    Category::Design,
                    format!(
                        "The pre-pass couldn't expose {}.{net}. It may have no driver or be a constant: debug another net.",
                        input.top_module
                    ),
                ));
            }
        }
    }
    Ok(Design {
        ports,
        widths,
        pin_ports: BTreeMap::new(),
    })
}

/// The pre-pass's failure, in the user's terms.
fn prepass_error(
    output: &str,
    nets: &[(String, Option<OutputMode>)],
    input: &DebugBuildInput,
    src: &Path,
) -> ToolError {
    let missing = output.lines().find_map(|l| {
        let rest = l.split("Assertion failed: selection contains ").nth(1)?;
        let count = rest.split_whitespace().next()?.to_string();
        let pat = rest.rsplit(": ").next()?.trim().to_string();
        let net = nets.iter().find(|(n, _)| pattern(n) == pat)?.0.clone();
        Some((net, count))
    });
    if let Some((net, count)) = missing {
        let what = if count == "0" {
            "isn't in the design".to_string()
        } else {
            format!("matches {count} nets")
        };
        return ToolError::new(
            Category::Design,
            format!(
                "{}: the net {}.{net} {what}. Name a whole net by its instance path, starting with the top module, such as {}.u_ctrl.state.",
                input.debug_file, input.top_module, input.top_module
            ),
        )
        .with_details(output);
    }
    let parsed = logix_engine::build::parse::parse_yosys(output, &src.to_string_lossy());
    let first = parsed
        .errors
        .first()
        .map(|d| d.text.clone())
        .unwrap_or_else(|| "yosys failed".into());
    let mut err = ToolError::new(Category::Design, format!("The design has errors. {first}"))
        .with_details(output);
    err.diagnostics = parsed.errors;
    err
}

// --- Layout and the generated top ------------------------------------------

fn layout(
    input: &DebugBuildInput,
    config: &DebugConfig,
    design: &Design,
    link: &Link,
) -> Result<DebugMap, ToolError> {
    let mut inputs = Vec::new();
    let mut offset = 0;
    let shape = |net: &str, bus: bool, width: usize| -> Result<(), ToolError> {
        let wrong = match (bus, width) {
            (false, w) if w != 1 => Some(format!("is {w} bits: name it with bus, not net")),
            (true, 1) => Some("is 1 bit: name it with net, not bus".to_string()),
            _ => None,
        };
        match wrong {
            Some(why) => Err(ToolError::new(
                Category::Design,
                format!(
                    "{}: {} {why}.",
                    input.debug_file,
                    if net.starts_with(PIN_PREFIX) {
                        net.to_string()
                    } else {
                        format!("{}.{net}", input.top_module)
                    }
                ),
            )),
            None => Ok(()),
        }
    };
    for i in &config.input {
        let width = design.widths[&i.net];
        shape(&i.net, !i.bus.is_empty(), width)?;
        inputs.push(MapInput {
            name: i.name.clone().unwrap_or_else(|| i.net.clone()),
            net: i.net.clone(),
            width,
            offset,
        });
        offset += width;
    }
    let read_bits = offset;
    let mut outputs = Vec::new();
    let mut offset = 0;
    let mut enables = 0;
    for o in &config.output {
        let width = design.widths[&o.net];
        shape(&o.net, !o.bus.is_empty(), width)?;
        let enable_bit = (o.mode == OutputMode::Override).then(|| {
            enables += 1;
            enables - 1
        });
        outputs.push(MapOutput {
            name: o.name.clone().unwrap_or_else(|| o.net.clone()),
            net: o.net.clone(),
            width,
            offset,
            mode: o.mode.as_str().into(),
            enable_bit,
        });
        offset += width;
    }
    if config
        .output
        .iter()
        .filter(|o| o.mode == OutputMode::Override)
        .count()
        != outputs.iter().filter(|o| o.enable_bit.is_some()).count()
    {
        return Err(ToolError::engine("The override enables don't add up."));
    }
    Ok(DebugMap {
        format: protocol::MAP_FORMAT.into(),
        protocol: protocol::PROTOCOL_VERSION,
        build_id: String::new(),
        board: input.board.clone(),
        top_module: input.top_module.clone(),
        clock: config.clock.clone(),
        clock_mhz: link.clock_mhz,
        baud: link.baud,
        uplink_resource: config.uplink.resource.clone(),
        tx_pin: link.tx.clone(),
        rx_pin: link.rx.clone(),
        read_bytes: read_bits.max(1).div_ceil(8),
        write_bytes: offset.max(1).div_ceil(8),
        enable_bytes: enables.max(1).div_ceil(8),
        inputs,
        outputs,
    })
}

/// A Verilog identifier for a name, escaped when it isn't a plain one.
fn vid(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain {
        name.to_string()
    } else {
        format!("\\{name} ")
    }
}

fn debug_top(user_top: &str, design: &Design, map: &DebugMap, link: &Link, id: u32) -> String {
    let read_w = map.inputs.iter().map(|i| i.width).sum::<usize>();
    let write_w = map.outputs.iter().map(|o| o.width).sum::<usize>();
    let enables = map
        .outputs
        .iter()
        .filter(|o| o.enable_bit.is_some())
        .count();
    let own = |n: &str| design.ports.iter().any(|p| p.name == n);

    // One wire per exposed design net, named by its order.
    let mut wires: Vec<(String, usize)> = Vec::new();
    let mut wire_of = BTreeMap::new();
    for net in map
        .inputs
        .iter()
        .map(|i| &i.net)
        .chain(map.outputs.iter().map(|o| &o.net))
    {
        if !own(net) && !design.pin_ports.contains_key(net) && !wire_of.contains_key(net) {
            wire_of.insert(net.clone(), format!("logix_dbg_net{}", wires.len()));
            wires.push((net.clone(), design.widths[net]));
        }
    }
    let signal = |net: &str| {
        design
            .pin_ports
            .get(net)
            .or(wire_of.get(net))
            .cloned()
            .unwrap_or_else(|| vid(net))
    };

    let mut v = String::new();
    let _ = writeln!(
        v,
        "// Generated by soft-agent's debug_build: {user_top} with the Logix debugger.\n\
         // Build ID {id:08x}. Don't edit: each debug build writes it again.\n"
    );
    let names: Vec<String> = design
        .ports
        .iter()
        .map(|p| vid(&p.name))
        .chain([TX_PORT.to_string(), RX_PORT.to_string()])
        .collect();
    let _ = writeln!(v, "module {TOP} ({});", names.join(", "));
    for p in &design.ports {
        let _ = writeln!(v, "  {} {}{};", p.direction, p.range, vid(&p.name));
    }
    let _ = writeln!(v, "  output {TX_PORT};\n  input {RX_PORT};\n");
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_rd;", read_w.max(1) - 1);
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_wr;", write_w.max(1) - 1);
    let _ = writeln!(v, "  wire [{}:0] logix_dbg_en;", enables.max(1) - 1);
    for (net, width) in &wires {
        let _ = writeln!(v, "  wire [{}:0] {}; // {net}", width - 1, wire_of[net]);
    }

    // The design, with its exposed nets.
    let mut conns: Vec<String> = design
        .ports
        .iter()
        .map(|p| format!(".{}({})", vid(&p.name), vid(&p.name)))
        .collect();
    for (net, _) in &wires {
        conns.push(format!(".{}({})", vid(net), wire_of[net]));
    }
    for o in &map.outputs {
        let value = format!("logix_dbg_wr[{}:{}]", o.offset + o.width - 1, o.offset);
        let driven = match o.enable_bit {
            Some(b) => format!("logix_dbg_en[{b}] ? {value} : {}", signal(&o.net)),
            None => value,
        };
        conns.push(format!(".{}({driven})", vid(&format!("{}.i", o.net))));
    }
    let _ = writeln!(
        v,
        "\n  {} u_design (\n      {}\n  );\n",
        vid(user_top),
        conns.join(",\n      ")
    );

    // The read space, first net in the low bits.
    let parts: Vec<String> = map.inputs.iter().rev().map(|i| signal(&i.net)).collect();
    let rd = if parts.is_empty() {
        "1'b0".to_string()
    } else {
        format!("{{{}}}", parts.join(", "))
    };
    let _ = writeln!(v, "  assign logix_dbg_rd = {rd};\n");
    let _ = writeln!(
        v,
        "  logix_debugger #(\n      .CLKS_PER_BIT({}),\n      .READ_W({}),\n      .WRITE_W({}),\n      .OVR_W({}),\n      .BUILD_ID(32'h{id:08x})\n  ) u_logix_debugger (\n      .clk({}),\n      .rx({RX_PORT}),\n      .tx({TX_PORT}),\n      .rd_in(logix_dbg_rd),\n      .wr_out(logix_dbg_wr),\n      .ovr_en(logix_dbg_en)\n  );\n\nendmodule",
        link.clks_per_bit,
        read_w.max(1),
        write_w.max(1),
        enables.max(1),
        vid(&map.clock)
    );
    v
}

// --- The debugger's own share ----------------------------------------------

async fn debugger_use(
    map: &DebugMap,
    link: &Link,
    gen_dir: &Path,
    bin: &Path,
    call: &Call,
) -> DebuggerUse {
    let read_w = map.inputs.iter().map(|i| i.width).sum::<usize>().max(1);
    let write_w = map.outputs.iter().map(|o| o.width).sum::<usize>().max(1);
    let enables = map
        .outputs
        .iter()
        .filter(|o| o.enable_bit.is_some())
        .count()
        .max(1);
    let script = format!(
        "read_verilog logix_debugger.v; chparam -set CLKS_PER_BIT {} -set READ_W {read_w} \
         -set WRITE_W {write_w} -set OVR_W {enables} logix_debugger; \
         synth_ice40 -top logix_debugger; tee -o debugger_stat.txt stat",
        link.clks_per_bit
    );
    let args: Vec<OsString> = vec!["-q".into(), "-p".into(), script.into()];
    let program = logix_engine::commands::program_in(bin, "yosys");
    let ok = logix_engine::runner::run_tool(&program, &args, gen_dir, bin, &call.cancel)
        .await
        .is_ok_and(|r| r.status.success());
    let stat = if ok {
        std::fs::read_to_string(gen_dir.join("debugger_stat.txt")).unwrap_or_default()
    } else {
        String::new()
    };
    let (mut luts, mut ffs) = (0u32, 0u32);
    for line in stat.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        if let [n, cell] = w.as_slice()
            && let Ok(n) = n.parse::<u32>()
        {
            if *cell == "SB_LUT4" {
                luts += n;
            } else if cell.starts_with("SB_DFF") {
                ffs += n;
            }
        }
    }
    let report = if luts == 0 {
        "The debugger's own share couldn't be measured.".to_string()
    } else {
        format!("The debugger uses about {luts} LUTs and {ffs} flip-flops of that.")
    };
    DebuggerUse {
        luts,
        flip_flops: ffs,
        report,
    }
}

// --- Files ------------------------------------------------------------------

fn read(path: &Path, what: &str) -> Result<String, ToolError> {
    std::fs::read_to_string(path)
        .map_err(|e| ToolError::request(format!("Can't read {what}, {}: {e}", path.display())))
}

fn read_json(path: &Path) -> Result<Value, ToolError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ToolError::new(
            Category::Tool,
            format!("The pre-pass wrote no {}: {e}", path.display()),
        )
    })?;
    serde_json::from_str(&text).map_err(|e| {
        ToolError::new(
            Category::Tool,
            format!("Can't read {}: {e}", path.display()),
        )
    })
}

fn write(path: &Path, text: &str) -> Result<(), ToolError> {
    std::fs::write(path, text)
        .map_err(|e| ToolError::engine(format!("Can't write {}: {e}", path.display())))
}

/// A problem in the debug file, as a design error on that file.
fn file_error(file: &str, message: &str) -> ToolError {
    let line = message
        .split(" at line ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse::<u32>().ok());
    let mut err = ToolError::new(Category::Design, format!("{file}: {message}"));
    err.diagnostics = vec![diagnostic(file, line.unwrap_or(1), message)];
    err
}

fn diagnostic(file: &str, line: u32, message: &str) -> Diagnostic {
    Diagnostic {
        file: Some(file.to_string()),
        line: Some(line),
        column: None,
        severity: logix_engine::errors::Severity::Error,
        text: message.to_string(),
        tool: Some("debug_build".into()),
        time_ns: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcf_pins_read_set_io() {
        let p = pcf_pins("# x\nset_io sys_clk 35\nset_io -nowarn led[0] 40 # c\n");
        assert_eq!(p["sys_clk"], "35");
        assert_eq!(p["led[0]"], "40");
    }

    #[test]
    fn paths_start_with_the_top() {
        let c = config::parse(
            r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"main.u.count"},{"net":"main.blink","name":"b"}]}"#,
        )
        .unwrap();
        let c = in_top(c, "main").unwrap();
        assert_eq!(c.input[0].net, "u.count");
        assert_eq!(c.input[0].name.as_deref(), Some("main.u.count"));
        assert_eq!(c.input[1].name.as_deref(), Some("b"));
        for bad in ["u.count", "mainx.count", "main"] {
            let c =
                config::parse(&format!(r#"{{"clock":"c","uplink":{{"kind":"uart","resource":"uart"}},"input":[{{"net":"{bad}"}}]}}"#)).unwrap();
            assert!(
                in_top(c, "main").unwrap_err().contains("must start"),
                "{bad}"
            );
        }
    }

    #[test]
    fn pins_read_the_port_placed_there() {
        let c = config::parse(
            r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"pin.18"},{"net":"pin.19"}]}"#,
        )
        .unwrap();
        let c = in_top(c, "main").unwrap();
        assert_eq!(c.input[0].net, "pin.18");
        let pins = pcf_pins("set_io sw[0] 18\nset_io key 19\n");
        let p = pin_ports(&c, &pins, "p.pcf").unwrap();
        assert_eq!(p["pin.18"], "sw[0]");
        assert_eq!(p["pin.19"], "key");
        let none = pcf_pins("set_io key 20\n");
        assert!(pin_ports(&c, &none, "p.pcf").is_err());
        assert!(in_top(c.clone(), "pin").is_err());
    }

    #[test]
    fn identifiers_escape_paths() {
        assert_eq!(vid("count"), "count");
        assert_eq!(vid("u_counter.count"), "\\u_counter.count ");
        assert_eq!(pattern("gen[2].u.err"), "w:gen?2?.u.err");
    }
}
