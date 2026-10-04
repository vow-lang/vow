//! The structured record of which verifier-model capacities a proof depends on.
//!
//! Every emitted `__ESBMC_assume` that prunes executions to a model capacity is
//! tagged with a marker comment. The marker is the single source of truth: the
//! "this proof is bounded" note is read off the model that was actually checked,
//! so it can never disagree with it. The wire format is shared with
//! `compiler/c_emitter.vow` (writer) and `compiler/verifier.vow` (reader).

/// Marker prefix of every emitted capacity assumption; the exact syntax is
/// `/* vow:model-bound <Kind> <digits> */`.
const MODEL_BOUND_MARKER: &str = "/* vow:model-bound ";
const MODEL_BOUND_END: &str = " */";

/// The longest capacity (in digits) a marker may carry. Both parsers reject
/// anything longer, so a bound is always representable in an `i64`.
const MAX_CAPACITY_DIGITS: usize = 18;

/// A kind of model capacity. The declaration order is the fixed order
/// [`model_capacity_bounds`] reports them in: the four collections, then the
/// user-struct heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelBoundKind {
    Vec,
    String,
    HashMap,
    BTreeMap,
    Heap,
}

impl ModelBoundKind {
    const ALL: [Self; 5] = [
        Self::Vec,
        Self::String,
        Self::HashMap,
        Self::BTreeMap,
        Self::Heap,
    ];

    /// The name in the marker wire format.
    pub fn marker_name(self) -> &'static str {
        match self {
            Self::Vec => "Vec",
            Self::String => "String",
            Self::HashMap => "HashMap",
            Self::BTreeMap => "BTreeMap",
            Self::Heap => "Heap",
        }
    }

    /// The label of the assertion that fires when a value outgrows the capacity.
    pub(crate) fn capacity_label(self) -> &'static str {
        match self {
            Self::Vec => "vec capacity",
            Self::String => "string capacity",
            Self::HashMap => "hashmap capacity",
            Self::BTreeMap => "btreemap capacity",
            Self::Heap => "heap capacity",
        }
    }

    /// The name in the user-facing bounded-proof note.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Vec => "Vec<T>",
            Self::HashMap => "HashMap<K, V>",
            Self::BTreeMap => "BTreeMap<K, V>",
            Self::String | Self::Heap => self.marker_name(),
        }
    }
}

/// The marker text appended to an emitted capacity assumption.
pub(crate) fn model_bound_marker(kind: ModelBoundKind, cap: usize) -> String {
    format!(
        " {MODEL_BOUND_MARKER}{} {cap}{MODEL_BOUND_END}",
        kind.marker_name()
    )
}

/// A capacity the emitted model restricts executions to. Every proof of a
/// function whose model carries one is a proof *within* that bound: executions
/// beyond it were pruned, not checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapacityBound {
    pub kind: ModelBoundKind,
    pub capacity: usize,
}

/// `(kind, capacity)` of one marker body (`<Kind> <digits>`), or `None` when it
/// is malformed: an unknown kind, no digits, a non-digit character, or more
/// than [`MAX_CAPACITY_DIGITS`] digits.
fn parse_marker_body(body: &str) -> Option<(ModelBoundKind, usize)> {
    let (kind, digits) = body.split_once(' ')?;
    let kind = ModelBoundKind::ALL
        .into_iter()
        .find(|k| k.marker_name() == kind)?;
    if digits.is_empty()
        || digits.len() > MAX_CAPACITY_DIGITS
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((kind, digits.parse().ok()?))
}

/// The capacity assumptions present in an emitted verification model, one entry
/// per kind in the fixed order `Vec`, `String`, `HashMap`, `BTreeMap`, `Heap`.
///
/// A malformed marker is ignored. When a kind carries several capacities the
/// *smallest* is reported: the proof is only claimed for executions within every
/// bound the model applied, so the strictest one is the honest summary, and it
/// never overstates what was checked.
pub fn model_capacity_bounds(c_src: &str) -> Vec<ModelCapacityBound> {
    let mut smallest = [None; ModelBoundKind::ALL.len()];
    let mut rest = c_src;
    while let Some(pos) = rest.find(MODEL_BOUND_MARKER) {
        rest = &rest[pos + MODEL_BOUND_MARKER.len()..];
        let Some(end) = rest.find(MODEL_BOUND_END) else {
            break;
        };
        if let Some((kind, cap)) = parse_marker_body(&rest[..end]) {
            let seen: &mut Option<usize> = &mut smallest[kind as usize];
            *seen = Some(seen.map_or(cap, |seen| seen.min(cap)));
        }
    }
    ModelBoundKind::ALL
        .into_iter()
        .zip(smallest)
        .filter_map(|(kind, capacity)| {
            capacity.map(|capacity| ModelCapacityBound { kind, capacity })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(src: &str) -> Vec<(&'static str, usize)> {
        model_capacity_bounds(src)
            .iter()
            .map(|b| (b.kind.marker_name(), b.capacity))
            .collect()
    }

    #[test]
    fn marker_text_round_trips_for_every_kind() {
        for kind in ModelBoundKind::ALL {
            let src = format!("x;{}\n", model_bound_marker(kind, 42));
            assert_eq!(
                model_capacity_bounds(&src),
                [ModelCapacityBound { kind, capacity: 42 }],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn labels_follow_the_kind() {
        let names: Vec<_> = ModelBoundKind::ALL
            .into_iter()
            .map(|k| (k.marker_name(), k.capacity_label(), k.display_name()))
            .collect();
        assert_eq!(
            names,
            [
                ("Vec", "vec capacity", "Vec<T>"),
                ("String", "string capacity", "String"),
                ("HashMap", "hashmap capacity", "HashMap<K, V>"),
                ("BTreeMap", "btreemap capacity", "BTreeMap<K, V>"),
                ("Heap", "heap capacity", "Heap"),
            ]
        );
    }

    #[test]
    fn reads_markers_in_fixed_order_one_per_kind() {
        let src = "x; /* vow:model-bound String 256 */\n y; /* vow:model-bound Vec 128 */\n \
                   z; /* vow:model-bound Vec 128 */\n w; /* vow:model-bound BTreeMap 64 */\n \
                   h; /* vow:model-bound Heap 1024 */\n";
        assert_eq!(
            pairs(src),
            [
                ("Vec", 128),
                ("String", 256),
                ("BTreeMap", 64),
                ("Heap", 1024)
            ]
        );
        assert!(model_capacity_bounds("no markers here").is_empty());
    }

    #[test]
    fn a_malformed_marker_is_ignored_and_scanning_continues() {
        for bad in [
            "/* vow:model-bound Bogus 1 */",
            "/* vow:model-bound Vec nope */",
            "/* vow:model-bound Vec 128x */",
            "/* vow:model-bound Vec +128 */",
            "/* vow:model-bound Vec -1 */",
            "/* vow:model-bound Vec  */",
            "/* vow:model-bound Vec */",
            "/* vow:model-bound Vec 128 junk */",
            "/* vow:model-bound vec 128 */",
            "/* vow:model-bound Vec 1234567890123456789 */",
        ] {
            let src = format!("a; {bad}\n b; /* vow:model-bound String 256 */\n");
            assert_eq!(pairs(&src), [("String", 256)], "{bad}");
        }
        let src = "a; /* vow:model-bound Vec /* vow:model-bound Vec 64 */\n";
        assert_eq!(pairs(src), [("Vec", 64)], "nested prefix");
        assert!(model_capacity_bounds("a; /* vow:model-bound Vec 128").is_empty());
    }

    #[test]
    fn the_largest_representable_capacity_parses() {
        let src = "/* vow:model-bound Heap 999999999999999999 */";
        assert_eq!(pairs(src), [("Heap", 999_999_999_999_999_999)]);
    }

    #[test]
    fn mixed_capacities_of_one_kind_report_the_smallest() {
        let src = "/* vow:model-bound Vec 128 */ /* vow:model-bound Vec 64 */ \
                   /* vow:model-bound Vec 256 */ /* vow:model-bound String 256 */ \
                   /* vow:model-bound String 300 */";
        assert_eq!(pairs(src), [("Vec", 64), ("String", 256)]);
    }
}
