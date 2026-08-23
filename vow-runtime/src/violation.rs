//! Pure rendering of vow-violation diagnostics.
//!
//! `__vow_violation` in `lib.rs` is the FFI/`process::exit` adapter: it decodes
//! the C ABI (`VowBinding`, C strings), calls into this module to build the two
//! output lines, writes them to stderr, and aborts. All the formatting logic —
//! the `VowViolation` JSON envelope (see `skills/vow/schemas/vow-violation.schema.json`
//! and `docs/spec/cli.md`) and its human-readable twin — lives here, free of
//! `unsafe`, global state, and exit, so it can be unit-tested directly.

// Runtime value tags. These encode how a captured predicate value was laid out
// by codegen so it can be rendered back to source-level text.
const TAG_I32: u8 = 0;
const TAG_I64: u8 = 1;
const TAG_F32: u8 = 2;
const TAG_F64: u8 = 3;
const TAG_BOOL: u8 = 4;
const TAG_U64: u8 = 5;
const TAG_U8: u8 = 6;
const TAG_I8: u8 = 7;
const TAG_I16: u8 = 8;
const TAG_U16: u8 = 9;
const TAG_U32: u8 = 10;

/// A decoded predicate binding: a free variable's name and its captured runtime
/// value. `lib.rs` builds these from the `VowBinding` C ABI records.
pub(crate) struct ValueBinding<'a> {
    pub name: &'a str,
    pub tag: u8,
    pub payload: u64,
}

/// The two rendered lines for a single violation: the structured JSON envelope
/// and the human-readable summary. Named fields so callers never have to
/// remember a tuple order.
pub(crate) struct RenderedViolation {
    pub json: String,
    pub human: String,
}

/// Render a captured predicate value to its source-level text, keyed by tag.
pub(crate) fn format_value(tag: u8, payload: u64) -> String {
    match tag {
        TAG_I32 => format!("{}", payload as i32),
        TAG_I64 => format!("{}", payload as i64),
        TAG_F32 => format!("{}", f32::from_bits(payload as u32)),
        TAG_F64 => format!("{}", f64::from_bits(payload)),
        TAG_BOOL => if payload != 0 { "true" } else { "false" }.to_string(),
        TAG_U64 => format!("{payload}"),
        TAG_U8 => format!("{}", payload as u8),
        TAG_I8 => format!("{}", payload as i8),
        TAG_I16 => format!("{}", payload as i16),
        TAG_U16 => format!("{}", payload as u16),
        TAG_U32 => format!("{}", payload as u32),
        _ => format!("0x{payload:x}"),
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string to JSON cannot fail")
}

/// Build the JSON and human-readable lines for a vow violation.
///
/// `blame == 0` is the Caller (a `requires` violation); any other value is the
/// Callee (an `ensures`/`invariant` violation). When `bindings` is empty the
/// `values` object is omitted from the JSON entirely.
pub(crate) fn render_violation(
    vow_id: u32,
    blame: u8,
    description: &str,
    file: &str,
    offset: u32,
    bindings: &[ValueBinding<'_>],
) -> RenderedViolation {
    let blame_str = if blame == 0 { "Caller" } else { "Callee" };
    let description_json = json_string(description);
    let file_json = json_string(file);

    let (values_json, values_human) = if bindings.is_empty() {
        (String::new(), String::new())
    } else {
        let mut json_pairs = String::new();
        let mut human_pairs = String::new();
        for (i, b) in bindings.iter().enumerate() {
            let name = b.name;
            let name_json = json_string(name);
            let val = format_value(b.tag, b.payload);
            if i > 0 {
                json_pairs.push(',');
                human_pairs.push_str(", ");
            }
            json_pairs.push_str(&format!("{name_json}:{val}"));
            human_pairs.push_str(&format!("{name}={val}"));
        }
        (
            format!(r#","values":{{{json_pairs}}}"#),
            format!(", {human_pairs}"),
        )
    };

    let json = format!(
        r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":"{blame_str}","description":{description_json},"file":{file_json},"offset":{offset}{values_json}}}"#
    );
    let human = format!(
        "vow violation: {description}, blame={blame_str}, file={file}, offset={offset}{values_human}"
    );
    RenderedViolation { json, human }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_value_renders_each_tag() {
        assert_eq!(format_value(TAG_I32, (-5i32) as u32 as u64), "-5");
        assert_eq!(format_value(TAG_I64, (-5i64) as u64), "-5");
        assert_eq!(format_value(TAG_F64, 1.5f64.to_bits()), "1.5");
        assert_eq!(format_value(TAG_F32, 1.5f32.to_bits() as u64), "1.5");
        assert_eq!(format_value(TAG_BOOL, 0), "false");
        assert_eq!(format_value(TAG_BOOL, 1), "true");
        assert_eq!(format_value(TAG_U64, 42), "42");
        assert_eq!(format_value(TAG_U8, 255), "255");
        assert_eq!(format_value(TAG_I8, (-128_i8) as u8 as u64), "-128");
        assert_eq!(format_value(TAG_I16, (-32768_i16) as u16 as u64), "-32768");
        assert_eq!(format_value(TAG_U16, u64::from(u16::MAX)), "65535");
        assert_eq!(format_value(TAG_U32, u64::from(u32::MAX)), "4294967295");
        // Unknown tag falls back to a hex dump of the raw payload.
        assert_eq!(format_value(99, 0xdead), "0xdead");
    }

    // Anchor: docs/spec/cli.md:511 (the divide-by-zero example).
    #[test]
    fn render_violation_matches_documented_json_with_binding() {
        let bindings = [ValueBinding {
            name: "y",
            tag: TAG_I64,
            payload: 0,
        }];
        let r = render_violation(0, 0, "y != 0", "divide.vow", 42, &bindings);
        assert_eq!(
            r.json,
            r#"{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}"#
        );
    }

    #[test]
    fn render_violation_json_round_trips_description_and_file() {
        let description = "string_matches_literal_at(s, pos, \"ab\") == 1 && path == \"C:\\tmp\"\n\t\u{0008}\u{000c}\r";
        let file = "quoted \"source\"/unicode-люм\npath\\file.vow";

        let r = render_violation(1, 0, description, file, 7, &[]);
        let json: serde_json::Value =
            serde_json::from_str(&r.json).expect("violation envelope must be valid JSON");

        assert_eq!(json["description"], description);
        assert_eq!(json["file"], file);
    }

    #[test]
    fn render_violation_json_round_trips_binding_names() {
        let name = "quoted\"name\\unicode-люм\n\t\u{0000}";
        let bindings = [ValueBinding {
            name,
            tag: TAG_I64,
            payload: 42,
        }];

        let r = render_violation(2, 0, "predicate", "source.vow", 11, &bindings);
        let json: serde_json::Value =
            serde_json::from_str(&r.json).expect("violation envelope must be valid JSON");

        assert_eq!(json["values"][name], 42);
    }

    // Anchor: docs/level5-test-trace.md:373-374 (matched JSON + human pair).
    #[test]
    fn render_violation_matches_trace_doc_json_and_human() {
        let bindings = [ValueBinding {
            name: "b",
            tag: TAG_I64,
            payload: 0,
        }];
        let r = render_violation(0, 0, "requires b != 0", "", 0, &bindings);
        assert_eq!(
            r.json,
            r#"{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"requires b != 0","file":"","offset":0,"values":{"b":0}}"#
        );
        assert_eq!(
            r.human,
            "vow violation: requires b != 0, blame=Caller, file=, offset=0, b=0"
        );
    }

    // No bindings → the `values` object is omitted; blame != 0 → Callee.
    #[test]
    fn render_violation_omits_values_when_no_bindings_and_maps_callee() {
        let r = render_violation(7, 1, "result > 0", "f.vow", 3, &[]);
        assert_eq!(
            r.json,
            r#"{"error":"VowViolation","vow_id":7,"blame":"Callee","description":"result > 0","file":"f.vow","offset":3}"#
        );
        assert_eq!(
            r.human,
            "vow violation: result > 0, blame=Callee, file=f.vow, offset=3"
        );
    }

    // Multiple bindings are comma-joined in both renderings.
    #[test]
    fn render_violation_joins_multiple_bindings() {
        let bindings = [
            ValueBinding {
                name: "a",
                tag: TAG_I64,
                payload: 3,
            },
            ValueBinding {
                name: "flag",
                tag: TAG_BOOL,
                payload: 1,
            },
        ];
        let r = render_violation(2, 0, "a >= 0", "m.vow", 9, &bindings);
        assert_eq!(
            r.json,
            r#"{"error":"VowViolation","vow_id":2,"blame":"Caller","description":"a >= 0","file":"m.vow","offset":9,"values":{"a":3,"flag":true}}"#
        );
        assert_eq!(
            r.human,
            "vow violation: a >= 0, blame=Caller, file=m.vow, offset=9, a=3, flag=true"
        );
    }
}
