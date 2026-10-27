# ADR-2026-10-27-0900: replay rebuilds a held `Vec<i128>` / `Vec<u128>`

Status: accepted (issue #1890, epic #1398)

Addendum to [ADR-2026-10-08-1422](2026-10-08-1422-native-verification-semantics.md) (a 128-bit element is one
listed value, #1821) and [ADR-2026-10-27-0617](2026-10-27-0617-fields-read-through-a-handle.md). Replay
harness only (no Rust twin, [ADR-2026-10-08-1421](2026-10-08-1421-verifier-lives-only-in-compiler.md)).

## Context

A `Vec<i128>` or `Vec<u128>` held in a struct field lists its length (`b.0.len`) and every element
(`b.0[i]`), each element as the one decimal value the program reads, exactly as a `Vec` parameter does.
`--replay-cex` nevertheless declined it (`field of type Vec<i128> holds elements replay cannot rebuild`),
on the belief that a held 128-bit element was the raw 64-bit slot. That stopped being true when the listing
was made per element.

## Decision

A held `Vec` whose element is `bool` or any integer, 128-bit ones included, is rebuilt by the same
element rule as a `Vec` parameter (`replay_vec_arg`, `replay_wide_element`): a listed element is a literal
of its type, an unlisted one (length above the listing bound) is a skip. A held `Vec<f64>` stays skipped
(its elements are raw slots a float cannot rebuild).

## Consequences

No counterexample key changes. The four `replay-skipped` fixtures of a held wide `Vec`
(`heap_vec_wide_field_*`, `wide_vec_in_struct_field_*`) now replay `confirmed`; their directive lines are
removed with this change.
