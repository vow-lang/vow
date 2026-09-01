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
const TAG_I128: u8 = 11;
const TAG_U128: u8 = 12;

/// A decoded predicate binding: a free variable's name and its captured runtime
/// value. `lib.rs` builds these from the `VowBinding` C ABI records.
pub(crate) struct ValueBinding<'a> {
    pub name: &'a str,
    pub tag: u8,
    pub payload: u128,
}

/// The two rendered lines for a single violation: the structured JSON envelope
/// and the human-readable summary. Named fields so callers never have to
/// remember a tuple order.
pub(crate) struct RenderedViolation {
    pub json: String,
    pub human: String,
}

/// Render a captured predicate value to its source-level text, keyed by tag.
///
/// Values are interpolated into the JSON envelope bare, as numeric/boolean
/// literals. Two arms can still emit tokens that are not valid JSON: non-finite
/// floats (`NaN`, `inf`) and the unknown-tag `0x…` fallback. Fixing that needs a
/// schema decision (`values` is `["integer","number","boolean"]` with
/// `additionalProperties: false`), so it is tracked separately under #436 rather
/// than papered over here.
pub(crate) fn format_value(tag: u8, payload: u128) -> String {
    let lo = payload as u64;
    match tag {
        TAG_I32 => format!("{}", lo as i32),
        TAG_I64 => format!("{}", lo as i64),
        TAG_F32 => format!("{}", f32::from_bits(lo as u32)),
        TAG_F64 => format!("{}", f64::from_bits(lo)),
        TAG_BOOL => if lo != 0 { "true" } else { "false" }.to_string(),
        TAG_U64 => format!("{lo}"),
        TAG_U8 => format!("{}", lo as u8),
        TAG_I8 => format!("{}", lo as i8),
        TAG_I16 => format!("{}", lo as i16),
        TAG_U16 => format!("{}", lo as u16),
        TAG_U32 => format!("{}", lo as u32),
        TAG_I128 => format!("{}", payload as i128),
        TAG_U128 => format!("{payload}"),
        _ => format!("0x{payload:x}"),
    }
}

/// Render `value` as a complete JSON string literal, **including the
/// surrounding double quotes**. Callers interpolate the result bare — writing
/// `"{json_string(x)}"` would double-encode the quotes.
///
/// Infallible: the only `Serializer` error for a `&str` is invalid UTF-8, and
/// every caller's input already came through `CStr::to_string_lossy`.
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
    let blame_json = json_string(blame_str);
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
        r#"{{"error":"VowViolation","vow_id":{vow_id},"blame":{blame_json},"description":{description_json},"file":{file_json},"offset":{offset}{values_json}}}"#
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
        assert_eq!(format_value(TAG_I32, (-5i32) as u32 as u128), "-5");
        assert_eq!(format_value(TAG_I64, (-5i64) as u64 as u128), "-5");
        assert_eq!(format_value(TAG_F64, 1.5f64.to_bits() as u128), "1.5");
        assert_eq!(format_value(TAG_F32, 1.5f32.to_bits() as u128), "1.5");
        assert_eq!(format_value(TAG_BOOL, 0), "false");
        assert_eq!(format_value(TAG_BOOL, 1), "true");
        assert_eq!(format_value(TAG_U64, 42), "42");
        assert_eq!(format_value(TAG_U8, 255), "255");
        assert_eq!(format_value(TAG_I8, (-128_i8) as u8 as u128), "-128");
        assert_eq!(format_value(TAG_I16, (-32768_i16) as u16 as u128), "-32768");
        assert_eq!(format_value(TAG_U16, u128::from(u16::MAX)), "65535");
        assert_eq!(format_value(TAG_U32, u128::from(u32::MAX)), "4294967295");
        assert_eq!(
            format_value(TAG_I128, i128::MIN as u128),
            "-170141183460469231731687303715884105728"
        );
        // 171 * 2^64: low limb zero, so a low-limb-only render would say "0".
        assert_eq!(
            format_value(TAG_I128, 3154393236604333326336),
            "3154393236604333326336"
        );
        assert_eq!(
            format_value(TAG_U128, u128::MAX),
            "340282366920938463463374607431768211455"
        );
        // Unknown tag falls back to a hex dump of the raw payload.
        assert_eq!(format_value(99, 0xdead), "0xdead");
    }

    #[test]
    fn render_violation_reports_full_128_bit_magnitude() {
        let bindings = [ValueBinding {
            name: "x",
            tag: TAG_U128,
            payload: u128::MAX,
        }];
        let r = render_violation(0, 0, "x < 0", "wide.vow", 17, &bindings);

        assert!(
            r.json
                .contains(r#""x":340282366920938463463374607431768211455"#)
        );
        assert!(
            r.human
                .contains("x=340282366920938463463374607431768211455")
        );
        serde_json::from_str::<serde_json::Value>(&r.json)
            .expect("violation envelope must be valid single-line JSON");
        assert_eq!(r.json.lines().count(), 1);
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

    // Issue #1046: a string-literal predicate puts real quotes in the
    // description, and a path can hold quotes, backslashes, or a newline. One
    // unescaped character makes the whole envelope unparseable, and a raw
    // newline additionally splits the line-delimited stderr protocol.
    #[test]
    fn render_violation_escapes_every_json_string_field() {
        let description = r#"s == String::from("ab") && p == "C:\tmp""#;
        let file = "quoted \"source\"\npath\\file.vow";
        let name = "quoted\"name\\tab\t";
        let bindings = [ValueBinding {
            name,
            tag: TAG_I64,
            payload: 42,
        }];

        let r = render_violation(1, 0, description, file, 7, &bindings);
        let json: serde_json::Value =
            serde_json::from_str(&r.json).expect("violation envelope must be valid JSON");

        assert_eq!(json["description"], description);
        assert_eq!(json["file"], file);
        assert_eq!(json["values"][name], 42);
        // The escaped envelope stays on a single stderr line.
        assert_eq!(r.json.lines().count(), 1);
        // Only the JSON line is escaped; the human line keeps the raw text.
        assert!(r.human.contains(description));
        assert!(r.human.contains(file));
        assert!(r.human.contains(&format!("{name}=42")));
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
