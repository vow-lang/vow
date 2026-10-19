"""Render the legacy human-readable --help text from the help JSON."""

from helpgen.config import DEFAULT_MAX_K_STEP


def build_help_human(data: dict) -> str:
    lines: list[str] = []
    lines.append("vow \u2014 Vow compiler")
    lines.append("")
    lines.append("USAGE")
    lines.append("  vow build [OPTIONS] <source.vow>    Compile to native executable")
    lines.append(
        "  vow verify [OPTIONS] <source.vow>    Verify contracts only (no executable)"
    )
    lines.append("  vow test [OPTIONS] [<path>]          Run tests with JSON results")
    lines.append("  vow contracts [OPTIONS] <source.vow> List all contracts")
    lines.append(
        "  vow complexity [OPTIONS] <source.vow> Report per-function complexity metrics (JSON)"
    )
    lines.append("  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)")
    lines.append("  vow skill [print [--bundle]|install [--local|--global]]")
    lines.append(
        "                                        Generate or install Claude Code skill"
    )
    lines.append(
        "  vow [OPTIONS] <source.vow>          Legacy mode (same as vow build)"
    )
    lines.append("")

    lines.append("BUILD OPTIONS")
    for flag, desc in data["build_options"].items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    lines.append("VERIFY OPTIONS")
    for flag, desc in data["verify_options"].items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    if data.get("test_options"):
        lines.append("TEST OPTIONS")
        for flag, desc in data["test_options"].items():
            pad = max(24, len(flag) + 2)
            lines.append(f"  {flag:<{pad}s}{desc}")
        lines.append("")

    lines.append("CONTRACTS OPTIONS")
    for flag, desc in data.get("contracts_options", {}).items():
        pad = max(24, len(flag) + 2)
        lines.append(f"  {flag:<{pad}s}{desc}")
    lines.append("")

    if data.get("complexity_options"):
        lines.append("COMPLEXITY OPTIONS")
        for flag, desc in data["complexity_options"].items():
            pad = max(24, len(flag) + 2)
            lines.append(f"  {flag:<{pad}s}{desc}")
        lines.append("")

    if data.get("decl_options"):
        lines.append("DECL OPTIONS")
        for flag, desc in data["decl_options"].items():
            pad = max(24, len(flag) + 2)
            lines.append(f"  {flag:<{pad}s}{desc}")
        lines.append("")

    lines.append("GLOBAL OPTIONS")
    lines.append("  --help                Emit versioned JSON tool-help data")
    lines.append("  --help --human        Emit legacy text help")
    lines.append("")

    lines.append("OUTPUT (JSON on stdout)")
    lines.append(
        "  status      : Verified | Unverified | Skipped | CompileFailed | VerifyFailed"
    )
    lines.append("  executable  : path to compiled binary, or null")
    lines.append(
        "  diagnostics : array of {error_code, message, severity, span: {file, offset, length}}"
    )
    lines.append("  message     : error detail (CompileFailed)")
    lines.append("  function    : function name (VerifyFailed)")
    lines.append("  counterexample: verifier counterexample (VerifyFailed)")
    lines.append("")

    lines.append("EXIT CODES")
    lines.append("  0  success (Verified or Unverified)")
    lines.append("  1  failure (CompileFailed, VerifyFailed, or Skipped)")
    lines.append("")

    lines.append("LANGUAGE SUMMARY")
    lines.append("  module Hello")
    lines.append("  use math.utils")
    lines.append("")
    lines.append("  struct Point { x: i64, y: i64 }")
    lines.append("")
    lines.append("  fn add(x: i64, y: i64) -> i64 {")
    lines.append("    x + y")
    lines.append("  }")
    lines.append("")
    lines.append("  fn divide(x: i64, y: i64) -> i64 vow {")
    lines.append("    requires: y != 0")
    lines.append("    ensures:  result * y == x")
    lines.append("  } {")
    lines.append("    x / y")
    lines.append("  }")
    lines.append("")
    lines.append("  fn main() -> i32 [io] {")
    lines.append("    let v: Vec<i64> = Vec::new();")
    lines.append("    v.push(divide(10, 2));")
    lines.append("    print_i64(v[0]);")
    lines.append("    0")
    lines.append("  }")
    lines.append("")

    lang = data["language"]
    types_str = "  ".join(lang["types"])
    lines.append(f"TYPES     : {types_str}")
    lines.append(f"EFFECTS   : {'  '.join(lang['effects'])}")

    builtins = lang["builtins"]
    bl = []
    for name, sig in builtins.items():
        bl.append(f"{name}: {sig}")
    bl_line = "   ".join(bl[:3])
    lines.append(f"BUILTINS  : {bl_line}")
    if len(bl) > 3:
        bl_line2 = "   ".join(bl[3:])
        lines.append(f"            {bl_line2}")

    methods = lang["methods"]

    def short_methods(names: list[str]) -> str:
        return "/".join(
            n.split("(")[0].lstrip(".") for n in names if not n.startswith("?")
        )

    vec_short = short_methods(methods["Vec<T>"])
    str_short = short_methods(methods["String"])
    hm_short = short_methods(methods["HashMap<K,V>"])
    bm_short = short_methods(methods["BTreeMap<K,V>"])
    opt_short = short_methods(methods["Option<T>"])
    lines.append(f"METHODS   : Vec: {vec_short}   String: {str_short}")
    lines.append(
        f"            HashMap: {hm_short}   BTreeMap: {bm_short}   Option: {opt_short}"
    )

    ops = lang["operators"]
    arith = " ".join(ops["arithmetic"])
    checked = " ".join(ops["checked_arithmetic"])
    comp = " ".join(ops["comparison"])
    logical = " ".join(ops["logical"])
    bitwise_str = " ".join(ops["bitwise"])
    unary = " ".join(ops["unary"])
    lines.append(
        f"OPERATORS : {arith}   {checked} (checked)   {comp}   {logical}   {bitwise_str} (bitwise, integer-only)   unary {unary}"
    )
    lines.append("")

    vdefaults = data.get("verification_defaults", {})
    if vdefaults:
        native = vdefaults.get("native", {})
        lines.append("VERIFICATION DEFAULTS")
        lines.append(
            f"  ESBMC backend (default) : {vdefaults.get('strategy', 'incremental-bmc')} (incremental BMC up to --max-k-step; forward-condition completeness, no k-induction step)"
        )
        lines.append(
            f"  Incremental BMC         : {vdefaults.get('max_k_step', DEFAULT_MAX_K_STEP)} max iterations (--max-k-step)"
        )
        if native:
            lines.append(
                f"  Native backend          : {native.get('strategy', '')} (--backend native; no bound flag, {', '.join(native.get('tuning_flags', []))} is the only knob)"
            )

    return "\n".join(lines)
