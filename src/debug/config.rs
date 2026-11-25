//! The debug configuration file (docs/debugger.md, The debug configuration
//! file): JSONC, checked against the types below, whose schema is
//! schemas/logix-debug-v1.json.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The only `$schema` value a debug file may have.
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/zapta/soft-agent/main/schemas/logix-debug-v1.json";

/// A debug configuration file: which nets the debugger reads and drives, how
/// it reaches the host and which clock it runs on.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Logix debug configuration, version 1")]
pub struct DebugConfig {
    /// This schema's address, for editors. Optional; any other value is
    /// refused.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// The clock the debugger and its UART run on: a top-level input port of
    /// the design, such as sys_clk. Every input and output net must be on
    /// it.
    pub clock: String,
    /// The clock's frequency in MHz. Needed only when the clock port isn't
    /// on the board's clock pin, whose frequency the catalog gives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_mhz: Option<f64>,
    /// How the debugger reaches the host: a UART, named as a resource of the
    /// board, or by its pins.
    pub uplink: Uplink,
    /// Nets the debugger reads.
    #[serde(default)]
    pub input: Vec<InputNet>,
    /// Nets the debugger drives.
    #[serde(default)]
    pub output: Vec<OutputNet>,
    /// Reserved for the capture phase; refused in version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub capture: Option<serde_json::Value>,
}

/// The link to the host: give resource, or tx and rx.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Uplink {
    /// The kind of link: uart, the only one in version 1.
    pub kind: UplinkKind,
    /// The board's UART, by its resource name in the catalog (get_catalog's
    /// resources), such as uart: the debugger uses its tx and rx pins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Instead of resource, the FPGA pin the debugger transmits on, such as
    /// a PMOD pin wired to a USB serial adapter's RX.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx: Option<String>,
    /// Instead of resource, the FPGA pin the debugger receives on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx: Option<String>,
    /// Bits per second. Defaults to 115200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UplinkKind {
    Uart,
}

/// A net the debugger reads.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputNet {
    /// The net: its instance path from the top module, starting with the top
    /// module's name, such as main.u_ctrl.state, or main.count for a net in
    /// the top module itself.
    pub net: String,
    /// The host's name for it. Defaults to the net's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A net the debugger drives.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputNet {
    /// The net, named as for inputs.
    pub net: String,
    /// The host's name for it. Defaults to the net's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// override: the debugger can take over a net the design drives, and
    /// release it back. register: the debugger always drives it. Defaults
    /// to override.
    #[serde(default)]
    pub mode: OutputMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    #[default]
    Override,
    Register,
}

impl OutputMode {
    pub fn as_str(self) -> &'static str {
        match self {
            OutputMode::Override => "override",
            OutputMode::Register => "register",
        }
    }
}

/// Parses a debug file's text, and checks what its types can't.
pub fn parse(text: &str) -> Result<DebugConfig, String> {
    let json = strip_comments(text);
    let config: DebugConfig = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if let Some(s) = &config.schema
        && s != SCHEMA_URL
    {
        return Err(format!("$schema must be {SCHEMA_URL:?}, or left out."));
    }
    if config.capture.is_some() {
        return Err("capture isn't supported yet: it comes in a later version.".into());
    }
    let u = &config.uplink;
    match (&u.resource, &u.tx, &u.rx) {
        (Some(_), None, None) | (None, Some(_), Some(_)) => {}
        (Some(_), _, _) => {
            return Err("uplink: give resource, or tx and rx, not both.".into());
        }
        (None, _, _) => {
            return Err(
                "uplink: name the board's UART as resource, such as \"resource\": \"uart\", or give both tx and rx pins."
                    .into(),
            );
        }
    }
    if config.input.is_empty() && config.output.is_empty() {
        return Err("The file names no nets: add some to input or output.".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for (net, name) in config
        .input
        .iter()
        .map(|i| (&i.net, &i.name))
        .chain(config.output.iter().map(|o| (&o.net, &o.name)))
    {
        check_net(net)?;
        let host = name.as_deref().unwrap_or(net);
        if host.is_empty() {
            return Err(format!("The name of net {net:?} is empty."));
        }
        // An input and an output may share a net, but not a host name.
        if !names.insert(host.to_string()) {
            return Err(format!(
                "The name {host:?} is used twice: give one of them another name."
            ));
        }
    }
    if let Some(mhz) = config.clock_mhz
        && !(mhz > 0.0 && mhz <= 1000.0)
    {
        return Err(format!("clock_mhz {mhz} isn't a clock frequency."));
    }
    Ok(config)
}

/// Version 1's naming rules: an instance path then a net, whole nets only.
fn check_net(net: &str) -> Result<(), String> {
    let bad = |why: &str| {
        Err(format!(
            "The net {net:?} {why}. Name a whole net by its instance path, such as main.u_ctrl.state."
        ))
    };
    if net.is_empty() {
        return bad("is empty");
    }
    if net
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
    {
        return bad("has a space, quote or backslash");
    }
    if net.contains(['*', '?']) {
        return bad("has a wildcard, which version 1 doesn't support");
    }
    // Brackets are for generate blocks in the path, not on the net itself.
    if net
        .rsplit('.')
        .next()
        .is_some_and(|last| last.contains('['))
    {
        return bad("selects bits, which version 1 doesn't support");
    }
    if net.starts_with('.') || net.ends_with('.') || net.contains("..") {
        return bad("isn't a path");
    }
    Ok(())
}

/// JSONC to JSON: removes // and /* */ comments outside strings, keeping
/// line breaks so serde's line numbers stay right.
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                    }
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The schema of the file, as written to schemas/logix-debug-v1.json (by the
/// schema_file_is_current test).
#[cfg(test)]
pub fn schema_json() -> String {
    let mut schema = serde_json::to_value(schemars::schema_for!(DebugConfig)).unwrap();
    if let Some(o) = schema.as_object_mut() {
        o.insert("$id".into(), SCHEMA_URL.into());
    }
    serde_json::to_string_pretty(&schema).unwrap() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_spec_example() {
        let c = parse(
            r#"{
              "$schema": "https://raw.githubusercontent.com/zapta/soft-agent/main/schemas/logix-debug-v1.json",
              "clock": "sys_clk",
              "uplink": { "kind": "uart", "resource": "uart", "baud": 115200 },
              "input": [ { "net": "count", "name": "counter" }, { "net": "blink" },
                         { "net": "u_ctrl.state" }, { "net": "gen_lane[2].u_lane.err" } ],
              /* a block comment */
              "output": [ { "net": "blink", "name": "blink_force", "mode": "override" } ]
            }"#,
        )
        .unwrap();
        assert_eq!(c.input.len(), 4);
        assert_eq!(c.output[0].mode, OutputMode::Override);
    }

    #[test]
    fn refuses_what_version_1_lacks() {
        for (text, why) in [
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"bus[7:4]"}]}"#,
                "selects bits",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"u_*.x"}]}"#,
                "wildcard",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"a"}],"capture":{}}"#,
                "capture",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"}}"#,
                "no nets",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"a"},{"net":"b","name":"a"}]}"#,
                "twice",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"a"}],"$schema":"x"}"#,
                "$schema",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"input":[{"net":"a"}],"extra":1}"#,
                "unknown field",
            ),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains(why), "{text}: {e}");
        }
    }

    #[test]
    fn uplinks_name_a_resource_or_pins() {
        let up = |u: &str| {
            parse(&format!(
                r#"{{"clock":"c","uplink":{u},"input":[{{"net":"main.a"}}]}}"#
            ))
        };
        let u = up(r#"{"kind":"uart","resource":"uart"}"#).unwrap().uplink;
        assert_eq!(
            (u.kind, u.resource.as_deref()),
            (UplinkKind::Uart, Some("uart"))
        );
        assert!(up(r#"{"kind":"uart","tx":"1","rx":"2"}"#).is_ok());
        for (bad, why) in [
            (r#"{}"#, "kind"),
            (r#"{"kind":"uart"}"#, "resource"),
            (r#"{"kind":"uart","tx":"1"}"#, "resource"),
            (r#"{"kind":"uart","resource":"uart","tx":"1"}"#, "not both"),
        ] {
            let e = up(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn comments_in_strings_stay() {
        assert_eq!(strip_comments(r#"{"a":"//x"} // c"#), r#"{"a":"//x"} "#);
    }

    /// schemas/logix-debug-v1.json is the types' schema. To update it:
    /// UPDATE_SCHEMA=1 cargo test schema_file
    #[test]
    fn schema_file_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/logix-debug-v1.json");
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::write(path, schema_json()).unwrap();
        }
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == schema_json(),
            "schemas/logix-debug-v1.json is out of date: run UPDATE_SCHEMA=1 cargo test schema_file"
        );
    }
}
