# Architecture Decision Records

## Legacy numbering (0001-0003)

`0001-*.md` through `0003-*.md` use sequential numbers assigned by hand.
This scheme is collision-prone under concurrent PRs — sibling projects have
already hit it for real: `s11` produced two different `0011-*.md` files, and
`symphonika` has a dozen-plus duplicate numbers. These three files are
frozen: don't renumber or reuse a number from this range — they're
referenced by number (e.g. `ADR 0001`, `ADR 0003`) throughout
`compiler/main.vow`, `vow/src/skill.rs`, `docs/spec/grammar.md`, and more.

## Current naming

New ADRs use `docs/adr/YYYY-MM-DD-slug.md`, dated the day the ADR is
authored. Reference one in prose or comments as `ADR-YYYY-MM-DD` (add the
slug too if more than one ADR shares a date). Two authors can't
independently pick the same real-world date-and-slug pair the way they
could pick the same next integer, so there's no more numbering-collision
class to guard against.
