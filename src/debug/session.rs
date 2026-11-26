//! The debug session tools: debug_open, debug_read, debug_write and
//! debug_close (docs/debugger.md, Host side). A session holds a board's
//! serial port and the debug map of the build on the board.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logix_engine::errors::{Category, ToolError};
use logix_engine::tools::EngineTool;

use super::protocol::{self, DebugMap};

/// How long a reply may take. A READ of 128 bytes takes about 12 ms at
/// 115200 baud; USB serial bridges add a few ms.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1000);

struct Session {
    /// The open port; None after the link broke, such as with the board
    /// unplugged, until the next read or write opens it again.
    port: Option<Box<dyn serialport::SerialPort>>,
    port_name: String,
    /// Set when the port was opened again: the board may have restarted,
    /// so once its build is checked, everything is released to match.
    reconnected: bool,
    map: DebugMap,
    /// What the host last wrote: the write registers and the override
    /// enables, which the debugger can't be asked for.
    wr: Vec<u8>,
    en: Vec<u8>,
}

fn sessions() -> &'static Mutex<BTreeMap<String, Session>> {
    static S: OnceLock<Mutex<BTreeMap<String, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn with_session<T>(
    id: &str,
    f: impl FnOnce(&mut Session) -> Result<T, ToolError>,
) -> Result<T, ToolError> {
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let s = all.get_mut(id).ok_or_else(|| {
        ToolError::request(format!(
            "session: {id:?} isn't an open debug session. Open one with debug_open."
        ))
    })?;
    f(s)
}

/// A frame to send: its command and payload.
type Frame = (u8, Vec<u8>);

/// The most bytes sent in one write. The debugger's receive buffer holds 64
/// while it replies to earlier frames.
const MAX_BATCH: usize = 48;

/// Sends frames and returns each reply's payload, in order, checking their
/// status. The frames go out together, up to MAX_BATCH bytes a write, so a
/// batch pays the link's round trip once: USB serial chips hold received
/// bytes for a while (an FTDI chip, such as the Alhambra II's, for 16 ms).
/// Opens the port again first when the link broke; a link error drops the
/// port, so the next call tries again (the board unplugged and back).
fn transact_many(s: &mut Session, frames: &[Frame]) -> Result<Vec<Vec<u8>>, ToolError> {
    if s.port.is_none() {
        let port = serialport::new(&s.port_name, s.map.baud)
            .timeout(Duration::from_millis(20))
            .open()
            .map_err(|e| {
                ToolError::setup(format!(
                    "The board on {} isn't there ({e}). Plug it back in: the session reconnects at the next read or write.",
                    s.port_name
                ))
            })?;
        s.port = Some(port);
        // A board plugged back in has restarted, with nothing overridden.
        s.wr = vec![0; s.map.write_bytes];
        s.en = vec![0; s.map.enable_bytes];
        s.reconnected = true;
        log::info!("Debug session reopened {}", s.port_name);
    }
    let result = exchange(s, frames);
    if let Err(e) = &result
        && e.category == Category::Setup
    {
        s.port = None;
    }
    result
}

/// One frame and its reply's payload.
fn transact(s: &mut Session, command: u8, payload: &[u8]) -> Result<Vec<u8>, ToolError> {
    Ok(transact_many(s, &[(command, payload.to_vec())])?.remove(0))
}

/// Frames and their replies, on the open port. A refused frame doesn't stop
/// the rest of its batch from being read, so no late reply is left on the
/// line for the next request.
fn exchange(s: &mut Session, frames: &[Frame]) -> Result<Vec<Vec<u8>>, ToolError> {
    let name = s.port_name.clone();
    let Some(port) = s.port.as_mut() else {
        return Err(ToolError::engine("The debug session has no open port."));
    };
    let link = |e: String| {
        ToolError::new(
            Category::Setup,
            format!("The debugger on {name} didn't answer: {e}"),
        )
        .with_details(
            "Check that the board runs this debug build (program_board), and that no other \
             program, such as a serial session, uses the port.",
        )
    };
    let _ = port.clear(serialport::ClearBuffer::Input);
    let mut payloads = Vec::with_capacity(frames.len());
    let mut refused: Option<ToolError> = None;
    let mut next = 0;
    while next < frames.len() {
        // As many frames as fit in one write.
        let mut bytes = Vec::new();
        let mut commands = Vec::new();
        while next < frames.len() {
            let f = protocol::frame(frames[next].0, &frames[next].1);
            if !commands.is_empty() && bytes.len() + f.len() > MAX_BATCH {
                break;
            }
            bytes.extend(f);
            commands.push(frames[next].0);
            next += 1;
        }
        port.write_all(&bytes)
            .and_then(|_| port.flush())
            .map_err(|e| link(e.to_string()))?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        for command in commands {
            loop {
                match protocol::parse_reply(&buf).map_err(link)? {
                    Some((reply, used)) => {
                        buf.drain(..used);
                        if reply.command != command {
                            return Err(link(format!(
                                "it answered command {:02x} to command {command:02x}",
                                reply.command
                            )));
                        }
                        if reply.status != 0 && refused.is_none() {
                            refused = Some(ToolError::engine(format!(
                                "The debugger refused command {command:02x}: {}.",
                                protocol::status_text(reply.status)
                            )));
                        }
                        payloads.push(reply.payload);
                        break;
                    }
                    None if Instant::now() >= deadline => {
                        return Err(link(format!(
                            "no reply in {} ms",
                            REPLY_TIMEOUT.as_millis()
                        )));
                    }
                    None => match port.read(&mut chunk) {
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(e)
                            if e.kind() == ErrorKind::TimedOut
                                || e.kind() == ErrorKind::Interrupted => {}
                        Err(e) => return Err(link(e.to_string())),
                    },
                }
            }
        }
    }
    match refused {
        Some(e) => Err(e),
        None => Ok(payloads),
    }
}

/// The frames that write bytes `first..first + count` of the write
/// registers (WRITE) or the enables (OVERRIDE), a few bytes a frame.
fn write_frames(s: &Session, command: u8, first: usize, count: usize) -> Vec<Frame> {
    let space = if command == protocol::WRITE {
        &s.wr
    } else {
        &s.en
    };
    let mut frames = Vec::new();
    let mut at = first;
    while at < first + count {
        let n = (first + count - at).min(protocol::MAX_WRITE);
        let mut payload = (at as u16).to_le_bytes().to_vec();
        payload.extend_from_slice(&space[at..at + n]);
        frames.push((command, payload));
        at += n;
    }
    frames
}

/// The frames that read `total` bytes of a space (READ, READ_WRITES or
/// READ_ENABLES), a reply's worth each.
fn read_frames(command: u8, total: usize) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut at = 0;
    while at < total {
        let n = (total - at).min(protocol::MAX_READ);
        let mut payload = (at as u16).to_le_bytes().to_vec();
        payload.push(n as u8);
        frames.push((command, payload));
        at += n;
    }
    frames
}

/// The bytes the replies to read_frames(_, total) carry, joined.
fn joined(replies: &[Vec<u8>], total: usize) -> Result<Vec<u8>, ToolError> {
    let bytes: Vec<u8> = replies.concat();
    if bytes.len() != total {
        return Err(ToolError::engine("A debugger reply has the wrong length."));
    }
    Ok(bytes)
}

/// The frames that read the write registers and the enables back.
fn read_back_frames(s: &Session) -> Vec<Frame> {
    let mut frames = read_frames(protocol::READ_WRITES, s.map.write_bytes);
    frames.extend(read_frames(protocol::READ_ENABLES, s.map.enable_bytes));
    frames
}

/// Takes the replies to read_back_frames into the session's copy, so what it
/// reports is the board's.
fn take_read_back(s: &mut Session, replies: &[Vec<u8>]) -> Result<(), ToolError> {
    let n = read_frames(protocol::READ_WRITES, s.map.write_bytes).len();
    if replies.len() < n {
        return Err(ToolError::engine("Missing debugger replies."));
    }
    s.wr = joined(&replies[..n], s.map.write_bytes)?;
    s.en = joined(&replies[n..], s.map.enable_bytes)?;
    Ok(())
}

/// Releases every override and clears the write registers, in one batch.
fn reset(s: &mut Session) -> Result<(), ToolError> {
    s.en = vec![0; s.map.enable_bytes];
    s.wr = vec![0; s.map.write_bytes];
    let mut frames = write_frames(s, protocol::OVERRIDE, 0, s.map.enable_bytes);
    frames.extend(write_frames(s, protocol::WRITE, 0, s.map.write_bytes));
    transact_many(s, &frames).map(|_| ())
}

/// Asks the debugger for its build: Ok(Ok(id)) when it is the session's
/// map's build, Ok(Err(id)) when it is another.
fn board_build(s: &mut Session) -> Result<Result<String, String>, ToolError> {
    let info = transact(s, protocol::INFO, &[])?;
    info_build(s, &info)
}

/// An INFO reply's build: Ok(id) when it is the session's map's, Err(id)
/// when it is another.
fn info_build(s: &Session, info: &[u8]) -> Result<Result<String, String>, ToolError> {
    if info.len() != 9 {
        return Err(ToolError::engine(
            "The debugger's INFO reply has the wrong length.",
        ));
    }
    let id = format!(
        "{:08x}",
        u32::from_le_bytes([info[1], info[2], info[3], info[4]])
    );
    let read_bytes = u16::from_le_bytes([info[5], info[6]]) as usize;
    let write_bytes = u16::from_le_bytes([info[7], info[8]]) as usize;
    let same = info[0] == protocol::PROTOCOL_VERSION
        && id == s.map.build_id
        && read_bytes == s.map.read_bytes
        && write_bytes == s.map.write_bytes;
    Ok(if same { Ok(id) } else { Err(id) })
}

/// Before each read and write: the board may have been programmed with
/// another build since debug_open, and the map would then be wrong.
fn same_build(s: &mut Session) -> Result<(), ToolError> {
    let build = board_build(s)?;
    build_ok(s, build)
}

/// The board's build checked against the session's map, from an INFO
/// reply: an error when it is another build. After a reconnect, a matching
/// board is reset to match the session's copy.
fn build_ok(s: &mut Session, build: Result<String, String>) -> Result<(), ToolError> {
    match build {
        Ok(_) => {
            if s.reconnected {
                s.reconnected = false;
                reset(s)?;
            }
            Ok(())
        }
        Err(board_id) => Err(ToolError::new(
            Category::Setup,
            format!(
                "The board now runs another build (ID {board_id}), not this session's (ID {}): it was programmed after debug_open. Close this session, and open one with the build folder of the build on the board.",
                s.map.build_id
            ),
        )
        .with_data("board_build_id", &board_id)
        .with_data("map_build_id", &s.map.build_id)),
    }
}

// --- debug_open -------------------------------------------------------------

pub struct DebugOpen;

impl EngineTool for DebugOpen {
    const NAME: &'static str = "debug_open";
    const DESCRIPTION: &'static str = "Opens a debug session with the Logix debugger on a board, \
        for a build made by debug_build and programmed with program_board. Give the build \
        folder (its debug_map.json names the nets) and the board's serial port (a device's \
        serial_port from list_devices). Checks that the board runs that very build, by its \
        build ID, and refuses another. Opening releases every override, so the design runs on \
        its own. The debugger owns the port while the session is open: don't open a serial \
        session on it too, and close the debug session before program_board.";
    const READ_ONLY: bool = false;
    type Input = DebugOpenInput;
    type Output = DebugOpenOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugOpenInput {
    /// The debug build's folder, as an absolute path.
    pub build_folder: String,
    /// The serial port, such as /dev/cu.usbmodem1102 or COM5.
    pub port: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugOpenOutput {
    /// The session's ID, for the other debug tools.
    pub session: String,
    pub port: String,
    /// The build ID the board's debugger reported, which matches the map.
    pub build_id: String,
    /// What can be driven: each driven net's name, width and mode.
    pub drive: Vec<NetInfo>,
    /// What can be watched: each watched net's name and width.
    pub watch: Vec<NetInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NetInfo {
    pub name: String,
    pub net: String,
    pub width: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// For watched nets: the debug file's format for showing it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

static NEXT: AtomicU32 = AtomicU32::new(1);

pub fn open(input: DebugOpenInput) -> Result<DebugOpenOutput, ToolError> {
    let folder = PathBuf::from(&input.build_folder);
    if !folder.is_absolute() {
        return Err(ToolError::request("build_folder must be an absolute path."));
    }
    let map_path = folder.join(protocol::MAP_FILE);
    let text = std::fs::read_to_string(&map_path).map_err(|e| {
        ToolError::request(format!(
            "Can't read {}: {e}. Build with debug_build into this folder first.",
            map_path.display()
        ))
    })?;
    let map: DebugMap = serde_json::from_str(&text).map_err(|e| {
        ToolError::request(format!("{} isn't a debug map: {e}", map_path.display()))
    })?;
    if map.format != protocol::MAP_FORMAT || map.protocol != protocol::PROTOCOL_VERSION {
        return Err(ToolError::request(format!(
            "{} is a {} map for protocol {}; this soft-agent reads {} for protocol {}. Build again with debug_build.",
            map_path.display(),
            map.format,
            map.protocol,
            protocol::MAP_FORMAT,
            protocol::PROTOCOL_VERSION
        )));
    }
    {
        let all = sessions().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((id, _)) = all.iter().find(|(_, s)| s.port_name == input.port) {
            return Err(ToolError::request(format!(
                "Debug session {id} already has {}: use it, or close it first.",
                input.port
            )));
        }
    }
    let port = serialport::new(&input.port, map.baud)
        .timeout(Duration::from_millis(20))
        .open()
        .map_err(|e| {
            ToolError::setup(format!("Can't open {}: {e}", input.port)).with_details(
                "Check the port with list_devices, and that no serial session (close_serial) \
                 or other program has it.",
            )
        })?;
    let mut s = Session {
        port: Some(port),
        reconnected: false,
        port_name: input.port.clone(),
        wr: vec![0; map.write_bytes],
        en: vec![0; map.enable_bytes],
        map,
    };
    let board_id = match board_build(&mut s)? {
        Ok(id) => id,
        Err(board_id) => return Err(ToolError::new(
            Category::Setup,
            format!(
                "The board runs another debug build (ID {board_id}), not the one in {} (ID {}). Program this build with program_board, or open the build folder of the one on the board.",
                input.build_folder, s.map.build_id
            ),
        )
        .with_data("board_build_id", &board_id)
        .with_data("map_build_id", &s.map.build_id)),
    };
    reset(&mut s)?;
    let out = DebugOpenOutput {
        session: String::new(),
        port: input.port.clone(),
        build_id: board_id,
        watch: s
            .map
            .watch
            .iter()
            .map(|i| NetInfo {
                name: i.name.clone(),
                net: i.net.clone(),
                width: i.width,
                mode: None,
                format: i.format.clone(),
            })
            .collect(),
        drive: s
            .map
            .drive
            .iter()
            .map(|o| NetInfo {
                name: o.name.clone(),
                net: o.net.clone(),
                width: o.width,
                mode: Some(o.mode.clone()),
                format: None,
            })
            .collect(),
    };
    let id = format!("debug{}", NEXT.fetch_add(1, Ordering::Relaxed));
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), s);
    Ok(DebugOpenOutput { session: id, ..out })
}

// --- debug_read -------------------------------------------------------------

pub struct DebugRead;

impl EngineTool for DebugRead {
    const NAME: &'static str = "debug_read";
    const DESCRIPTION: &'static str = "Reads nets of the running design through a debug \
        session, all in one snapshot taken in a single clock cycle. Give the watched names to \
        read (from debug_open), or none for all. Each value comes as bits (most significant \
        first), as hex and, up to 53 bits, as a number.";
    const READ_ONLY: bool = true;
    type Input = DebugReadInput;
    type Output = DebugReadOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugReadInput {
    /// The session, from debug_open.
    pub session: String,
    /// The watched nets to read, by name. Omitted means all.
    #[serde(default)]
    pub names: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugReadOutput {
    pub values: Vec<NetValue>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NetValue {
    pub name: String,
    pub net: String,
    pub width: usize,
    /// The value as a number, when it has 53 bits or fewer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// The value in hex, such as 0x5b8d7f.
    pub hex: String,
    /// The bits, most significant first.
    pub bits: String,
}

pub fn read(input: DebugReadInput) -> Result<DebugReadOutput, ToolError> {
    with_session(&input.session, |s| {
        let wanted: Vec<protocol::MapWatch> = match &input.names {
            None => s.map.watch.clone(),
            Some(names) => names
                .iter()
                .map(|n| {
                    s.map
                        .watch
                        .iter()
                        .find(|i| i.name == *n)
                        .cloned()
                        .ok_or_else(|| {
                            let known: Vec<&str> =
                                s.map.watch.iter().map(|i| i.name.as_str()).collect();
                            ToolError::request(format!(
                                "names: {n:?} isn't watched in this build. It watches: {}.",
                                known.join(", ")
                            ))
                        })
                })
                .collect::<Result<_, _>>()?,
        };
        // The build check, the snapshot and the reads, in one batch.
        let mut frames: Vec<Frame> = vec![(protocol::INFO, vec![]), (protocol::SNAPSHOT, vec![])];
        frames.extend(read_frames(protocol::READ, s.map.read_bytes));
        let replies = transact_many(s, &frames)?;
        let build = info_build(s, &replies[0])?;
        build_ok(s, build)?;
        let bytes = joined(&replies[2..], s.map.read_bytes)?;
        Ok(DebugReadOutput {
            values: wanted
                .into_iter()
                .map(|i| {
                    let bits = protocol::get_bits(&bytes, i.offset, i.width);
                    net_value(&i.name, &i.net, &bits)
                })
                .collect(),
        })
    })
}

fn net_value(name: &str, net: &str, bits: &[bool]) -> NetValue {
    let msb_first: String = bits
        .iter()
        .rev()
        .map(|&b| if b { '1' } else { '0' })
        .collect();
    let mut hex = String::new();
    for chunk in bits.chunks(4).rev() {
        let nibble = chunk
            .iter()
            .enumerate()
            .fold(0u8, |a, (k, &b)| a | ((b as u8) << k));
        hex.push(char::from_digit(nibble as u32, 16).unwrap_or('0'));
    }
    let value = (bits.len() <= 53).then(|| {
        bits.iter()
            .enumerate()
            .fold(0u64, |a, (k, &b)| a | ((b as u64) << k))
    });
    NetValue {
        name: name.to_string(),
        net: net.to_string(),
        width: bits.len(),
        value,
        hex: format!("0x{hex}"),
        bits: msb_first,
    }
}

// --- debug_write ------------------------------------------------------------

pub struct DebugWrite;

impl EngineTool for DebugWrite {
    const NAME: &'static str = "debug_write";
    const DESCRIPTION: &'static str = "Drives a net of the running design through a debug \
        session: one of its drive names from debug_open. Writing takes the net over from the \
        design until release; release gives it back. The value is a number, or a string in hex (0x..), binary (0b..) \
        or decimal, and must fit the net's width.";
    const READ_ONLY: bool = false;
    type Input = DebugWriteInput;
    type Output = DebugWriteOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugWriteInput {
    /// The session, from debug_open.
    pub session: String,
    /// The net to drive, by its name in the debug file's drive list.
    pub name: String,
    /// The value to drive. Required unless release is true.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Give an override net back to the design. Defaults to false.
    #[serde(default)]
    pub release: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugWriteOutput {
    pub name: String,
    /// True while the debugger drives the net.
    pub driven: bool,
    /// The value the debugger drives, read back from it, in hex; omitted
    /// when it doesn't drive the net.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

pub fn write(input: DebugWriteInput) -> Result<DebugWriteOutput, ToolError> {
    with_session(&input.session, |s| {
        same_build(s)?;
        let o = s
            .map
            .drive
            .iter()
            .find(|o| o.name == input.name)
            .cloned()
            .ok_or_else(|| {
                let known: Vec<&str> = s.map.drive.iter().map(|o| o.name.as_str()).collect();
                ToolError::request(format!(
                    "name: {:?} isn't driven in this build. It drives: {}.",
                    input.name,
                    if known.is_empty() {
                        "none".into()
                    } else {
                        known.join(", ")
                    }
                ))
            })?;
        let enable_byte = o.enable_bit.map(|b| b / 8);
        if input.release {
            if input.value.is_some() {
                return Err(ToolError::request("Give value or release, not both."));
            }
            let Some(bit) = o.enable_bit else {
                return Err(ToolError::request(format!(
                    "{} is in {} mode, which has no release.",
                    o.name, o.mode
                )));
            };
            protocol::set_bits(&mut s.en, bit, &[false]);
            // The release and the read-back, in one batch.
            let mut frames = write_frames(s, protocol::OVERRIDE, bit / 8, 1);
            let n = frames.len();
            frames.extend(read_back_frames(s));
            let replies = transact_many(s, &frames)?;
            take_read_back(s, &replies[n..])?;
            let driven = protocol::get_bits(&s.en, bit, 1)[0];
            return Ok(DebugWriteOutput {
                name: o.name,
                driven,
                hex: None,
            });
        }
        let Some(value) = &input.value else {
            return Err(ToolError::request(
                "value is required, unless release is true.",
            ));
        };
        let bits = parse_value(value, o.width)?;
        protocol::set_bits(&mut s.wr, o.offset, &bits);
        let first = o.offset / 8;
        let last = (o.offset + o.width - 1) / 8;
        // The value, the take-over and the read-back, in one batch.
        let mut frames = write_frames(s, protocol::WRITE, first, last - first + 1);
        if let (Some(bit), Some(byte)) = (o.enable_bit, enable_byte) {
            protocol::set_bits(&mut s.en, bit, &[true]);
            frames.extend(write_frames(s, protocol::OVERRIDE, byte, 1));
        }
        let n = frames.len();
        frames.extend(read_back_frames(s));
        let replies = transact_many(s, &frames)?;
        // What the debugger now holds.
        take_read_back(s, &replies[n..])?;
        let held = protocol::get_bits(&s.wr, o.offset, o.width);
        let driven = o
            .enable_bit
            .is_none_or(|b| protocol::get_bits(&s.en, b, 1)[0]);
        Ok(DebugWriteOutput {
            name: o.name.clone(),
            driven,
            hex: driven.then(|| net_value(&o.name, &o.net, &held).hex),
        })
    })
}

/// A value for a net of `width` bits, as bits, least significant first.
fn parse_value(value: &serde_json::Value, width: usize) -> Result<Vec<bool>, ToolError> {
    let bad = |why: String| ToolError::request(format!("value: {why}"));
    let text = match value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .map(|v| v.to_string())
            .ok_or_else(|| bad(format!("{n} isn't a whole number of 0 or more")))?,
        serde_json::Value::String(s) => s.trim().replace('_', ""),
        serde_json::Value::Bool(b) => (*b as u8).to_string(),
        other => return Err(bad(format!("{other} isn't a number or a string"))),
    };
    let lower = text.to_ascii_lowercase();
    let (digits, radix) = if let Some(h) = lower.strip_prefix("0x") {
        (h, 16)
    } else if let Some(b) = lower.strip_prefix("0b") {
        (b, 2)
    } else {
        (lower.as_str(), 10)
    };
    if digits.is_empty() {
        return Err(bad(format!("{text:?} has no digits")));
    }
    // Bits, least significant first, for any width.
    let mut bits: Vec<bool> = Vec::new();
    if radix == 10 {
        let v: u128 = digits
            .parse()
            .map_err(|_| bad(format!("{text:?} isn't a number")))?;
        let mut v = v;
        while v > 0 {
            bits.push(v & 1 == 1);
            v >>= 1;
        }
    } else {
        let per = if radix == 16 { 4 } else { 1 };
        for c in digits.chars().rev() {
            let d = c
                .to_digit(radix)
                .ok_or_else(|| bad(format!("{text:?} has the digit {c:?}")))?;
            for k in 0..per {
                bits.push(d >> k & 1 == 1);
            }
        }
    }
    while bits.last() == Some(&false) {
        bits.pop();
    }
    if bits.len() > width {
        return Err(bad(format!(
            "{text} needs {} bits, and the net has {width}",
            bits.len()
        )));
    }
    bits.resize(width, false);
    Ok(bits)
}

// --- debug_close ------------------------------------------------------------

pub struct DebugClose;

impl EngineTool for DebugClose {
    const NAME: &'static str = "debug_close";
    const DESCRIPTION: &'static str = "Ends a debug session: releases every override, so the \
        design runs on its own, and frees the serial port.";
    const READ_ONLY: bool = false;
    type Input = DebugCloseInput;
    type Output = DebugCloseOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugCloseInput {
    /// The session, from debug_open.
    pub session: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugCloseOutput {
    /// False when the overrides couldn't be released, such as with the
    /// board unplugged.
    pub released: bool,
}

pub fn close(input: DebugCloseInput) -> Result<DebugCloseOutput, ToolError> {
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let mut s = all.remove(&input.session).ok_or_else(|| {
        ToolError::request(format!(
            "session: {:?} isn't an open debug session.",
            input.session
        ))
    })?;
    drop(all);
    let released = reset(&mut s).is_ok();
    Ok(DebugCloseOutput { released })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_in_three_bases() {
        let v = |j: serde_json::Value, w| parse_value(&j, w);
        assert_eq!(
            v(serde_json::json!(5), 4).unwrap(),
            vec![true, false, true, false]
        );
        assert_eq!(
            v(serde_json::json!("0x5"), 3).unwrap(),
            vec![true, false, true]
        );
        assert_eq!(
            v(serde_json::json!("0b1_01"), 3).unwrap(),
            vec![true, false, true]
        );
        assert!(v(serde_json::json!(8), 3).is_err());
        assert!(v(serde_json::json!("0xg"), 8).is_err());
    }

    #[test]
    fn values_show_three_ways() {
        let n = net_value("c", "c", &[true, false, true, true, true]);
        assert_eq!(n.bits, "11101");
        assert_eq!(n.hex, "0x1d");
        assert_eq!(n.value, Some(29));
    }
}

// --- For the panel ----------------------------------------------------------

/// A session's driven nets as the host last set them, and its build, for the
/// debug panel.
#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub session: String,
    pub port: String,
    pub build_id: String,
    pub board: String,
    pub top_module: String,
    /// True when the driven state was just read back from the board;
    /// false when it couldn't be, and it is what the host last wrote.
    pub verified: bool,
    pub drive: Vec<OutputState>,
    pub watch: Vec<NetInfo>,
}

#[derive(Debug, Serialize)]
pub struct OutputState {
    pub name: String,
    pub net: String,
    pub width: usize,
    pub mode: String,
    /// True while the debugger drives it.
    pub driven: bool,
    /// The value the debugger holds for it, in hex and as bits (most
    /// significant first); read back from the board when `verified`.
    pub hex: String,
    pub bits: String,
    /// The debug file's format for showing it, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

pub fn info(id: &str) -> Result<SessionInfo, ToolError> {
    with_session(id, |s| {
        // The build check and the read-back, in one batch.
        let verified = (|| {
            let mut frames: Vec<Frame> = vec![(protocol::INFO, vec![])];
            frames.extend(read_back_frames(s));
            let replies = transact_many(s, &frames)?;
            let build = info_build(s, &replies[0])?;
            build_ok(s, build)?;
            take_read_back(s, &replies[1..])
        })()
        .is_ok();
        Ok(SessionInfo {
            verified,
            session: id.to_string(),
            port: s.port_name.clone(),
            build_id: s.map.build_id.clone(),
            board: s.map.board.clone(),
            top_module: s.map.top_module.clone(),
            watch: s
                .map
                .watch
                .iter()
                .map(|i| NetInfo {
                    name: i.name.clone(),
                    net: i.net.clone(),
                    width: i.width,
                    mode: None,
                    format: i.format.clone(),
                })
                .collect(),
            drive: s
                .map
                .drive
                .iter()
                .map(|o| {
                    let bits = protocol::get_bits(&s.wr, o.offset, o.width);
                    OutputState {
                        name: o.name.clone(),
                        net: o.net.clone(),
                        width: o.width,
                        mode: o.mode.clone(),
                        driven: o
                            .enable_bit
                            .is_none_or(|b| protocol::get_bits(&s.en, b, 1)[0]),
                        hex: net_value(&o.name, &o.net, &bits).hex,
                        bits: bits
                            .iter()
                            .rev()
                            .map(|&b| if b { '1' } else { '0' })
                            .collect(),
                        format: o.format.clone(),
                    }
                })
                .collect(),
        })
    })
}

/// The open sessions' IDs.
pub fn ids() -> Vec<String> {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect()
}
