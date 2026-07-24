//! The `vow skill` module: the embedded skill documentation (the `--help`
//! machine JSON, the human `--help` text, and the installable SKILL.md
//! entrypoint + reference/schema support files) together with the logic that
//! installs that tree into a project or the user's home.
//!
//! main.rs sees only the small interface below — `json`, `human`,
//! `entrypoint_markdown`, `bundle_markdown`, `install`, `maybe_auto_install`.
//! The ~10k lines of embedded documentation and the install mechanics stay
//! behind that seam.
//!
//! The three `// GENERATE:SKILL_*` blocks are populated by
//! `scripts/generate_help.py` from `docs/spec/*.md`; do not edit them by hand.

use std::path::{Path, PathBuf};

// --- interface: embedded documentation ---------------------------------------

/// Machine-readable `--help` output (schema_version 2 tool_help JSON).
pub(crate) fn json() -> String {
    skill_json()
}

/// Human-readable `--help` output.
pub(crate) fn human() -> String {
    skill_human()
}

/// The concise installed `SKILL.md` entrypoint.
pub(crate) fn entrypoint_markdown() -> String {
    skill_entrypoint_markdown()
}

/// The full single-file skill bundle (`vow skill print --bundle`).
pub(crate) fn bundle_markdown() -> String {
    skill_bundle_markdown()
}

// --- install mechanics -------------------------------------------------------

fn install_skill_tree_to(root: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join(".claude/skills/vow");
    std::fs::create_dir_all(&dir).map_err(|e| {
        std::io::Error::new(e.kind(), format!("cannot create {}: {}", dir.display(), e))
    })?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, skill_entrypoint_markdown()).map_err(|e| {
        std::io::Error::new(e.kind(), format!("cannot write {}: {}", path.display(), e))
    })?;
    for (relative_path, contents) in skill_support_files() {
        let support_path = dir.join(relative_path);
        if let Some(parent) = support_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("cannot create {}: {}", parent.display(), e),
                )
            })?;
        }
        std::fs::write(&support_path, contents).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("cannot write {}: {}", support_path.display(), e),
            )
        })?;
    }
    Ok(path)
}

fn run_skill_install_scoped<R: std::io::BufRead, W: std::io::Write>(
    cwd: &Path,
    home: Option<&Path>,
    local: bool,
    global: bool,
    stdin: &mut R,
    stderr: &mut W,
) -> Result<PathBuf, String> {
    if local && global {
        return Err("vow skill install: --local and --global are mutually exclusive".to_string());
    }

    let mut use_local = local;
    let mut use_global = global;
    if !use_local && !use_global {
        stderr
            .write_all(b"Install Vow skill locally (./.claude) or globally (~/.claude)? [l/g]: ")
            .map_err(|e| format!("vow skill install: cannot write prompt: {e}"))?;
        let mut answer = String::new();
        stdin
            .read_line(&mut answer)
            .map_err(|e| format!("vow skill install: cannot read answer: {e}"))?;
        let normalized = answer.trim().to_ascii_lowercase();
        if normalized == "l" || normalized == "local" {
            use_local = true;
        } else if normalized == "g" || normalized == "global" {
            use_global = true;
        } else {
            return Err(
                "vow skill install: stdin closed or unrecognised answer; pass --local or --global"
                    .to_string(),
            );
        }
    }

    let root = if use_local {
        if !cwd.join(".claude").is_dir() {
            return Err(
                "vow skill install: --local requires a .claude/ directory in the current project"
                    .to_string(),
            );
        }
        if !cwd.join(".git").exists() {
            return Err(
                "vow skill install: --local requires the current project to be a git checkout"
                    .to_string(),
            );
        }
        cwd.to_path_buf()
    } else if use_global {
        home.filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| "vow skill install: --global requires $HOME".to_string())?
            .to_path_buf()
    } else {
        return Err("vow skill install: no install target selected".to_string());
    };

    install_skill_tree_to(&root).map_err(|e| format!("vow skill install: {e}"))
}

pub(crate) fn install(local: bool, global: bool) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    match run_skill_install_scoped(
        &cwd,
        home.as_deref(),
        local,
        global,
        &mut stdin,
        &mut stderr,
    ) {
        Ok(path) => eprintln!("installed skill to {}", path.display()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// First-run auto-install: when `.claude/` already exists in the working
/// directory but the Vow skill is not yet installed, drop a fresh copy in
/// place. Silent on success, silent on failure — must never fail the build.
pub(crate) fn maybe_auto_install(cwd: &Path) {
    let claude_dir = cwd.join(".claude");
    if !claude_dir.is_dir() {
        return;
    }
    let target = claude_dir.join("skills/vow/SKILL.md");
    if target.exists() {
        return;
    }
    let _ = install_skill_tree_to(cwd);
}

// --- generated documentation (populated by scripts/generate_help.py) ---------

// GENERATE:SKILL_JSON:START
fn skill_json() -> String {
    r##"{
  "schema_version": "2",
  "kind": "tool_help",
  "tool": "vow",
  "audience": "agent",
  "default_format": "json",
  "description": "Vow compiler: compiles Vow source to native executables with contract verification",
  "usage": "vow <command> [OPTIONS] <source.vow>",
  "legacy_usage": "vow [OPTIONS] <source.vow> (equivalent to vow build)",
  "references": {
    "grammar": "reference/grammar.md",
    "cli": "reference/cli.md",
    "contracts": "reference/contracts.md",
    "errors": "reference/errors.md",
    "stdlib": "reference/stdlib.md",
    "examples": "examples/examples.md",
    "schemas": {
      "build_result": "schemas/build-result.schema.json",
      "contracts_result": "schemas/contracts-result.schema.json",
      "diagnostic": "schemas/diagnostic.schema.json",
      "counterexample": "schemas/counterexample.schema.json",
      "mutants_result": "schemas/mutants-result.schema.json",
      "test_result": "schemas/test-result.schema.json",
      "complexity_result": "schemas/complexity-result.schema.json",
      "vow_violation": "schemas/vow-violation.schema.json"
    }
  },
  "invocation": {
    "canonical": "vow <command> [OPTIONS] <source.vow>",
    "default_command": "build",
    "legacy_equivalent": "vow [OPTIONS] <source.vow>",
    "source_argument": {
      "name": "source",
      "kind": "path",
      "required": true,
      "suffix": ".vow"
    }
  },
  "commands": {
    "build": "Compile source to native executable (verifies by default; use --no-verify to skip)",
    "verify": "Verify contracts without producing an executable (use --no-cache to skip cache)",
    "test": "Run tests: discover, compile, execute test_*.vow files with JSON results",
    "decl": "Emit declaration file (.vow.d) with type signatures only",
    "contracts": "List all contracts with optional verification status",
    "skill": "Generate or install the Claude Code skill document for this compiler version",
    "complexity": "Report per-function complexity metrics as deterministic, byte-identical JSON"
  },
  "command_details": {
    "build": {
      "status": "implemented",
      "usage": "vow build [OPTIONS] <source.vow>",
      "default_when_command_omitted": true,
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "-o, --output <path>",
          "description": "Output executable path (default: source without .vow extension)",
          "short": "-o",
          "long": "--output",
          "value_name": "path",
          "value_kind": "path",
          "default": "source without .vow extension"
        },
        {
          "form": "--mode <debug|release|profile|sanitize>",
          "description": "Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking (default: release)",
          "long": "--mode",
          "value_name": "debug|release|profile|sanitize",
          "value_kind": "enum",
          "values": [
            "debug",
            "release",
            "profile",
            "sanitize"
          ],
          "default": "release"
        },
        {
          "form": "--no-verify",
          "description": "Skip ESBMC static verification",
          "long": "--no-verify",
          "value_kind": "flag"
        },
        {
          "form": "--dump-ir",
          "description": "Print IR text to stdout and exit (no JSON output, no codegen)",
          "long": "--dump-ir",
          "value_kind": "flag"
        },
        {
          "form": "--debug-trace <off|calls|full>",
          "description": "Emit JSON trace lines to stderr at runtime (default: off)",
          "long": "--debug-trace",
          "value_name": "trace",
          "value_kind": "enum",
          "values": [
            "off",
            "calls",
            "full"
          ],
          "default": "off"
        },
        {
          "form": "--no-cache",
          "description": "Disable verification result caching, and (for --no-verify builds) the compile-object cache. See \"Compile-object cache behavior\" below",
          "long": "--no-cache",
          "value_kind": "flag"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (default: 50)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--solver <boolector|z3|bitwuzla|auto>",
          "description": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
          "long": "--solver",
          "value_name": "boolector|z3|bitwuzla|auto",
          "value_kind": "enum",
          "values": [
            "boolector",
            "z3",
            "bitwuzla",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--encoding <bv|ir|auto>",
          "description": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
          "long": "--encoding",
          "value_name": "bv|ir|auto",
          "value_kind": "enum",
          "values": [
            "bv",
            "ir",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--timeout <N>",
          "description": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
          "long": "--timeout",
          "value_name": "N",
          "value_kind": "integer",
          "default": "300 (or 30 when --encoding is auto)"
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Max concurrent ESBMC verification jobs (default: num_cpus/2)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        },
        {
          "form": "--replay-cex",
          "description": "Differential test the verifier model against runtime semantics (same as vow verify --replay-cex; see below)",
          "long": "--replay-cex",
          "value_kind": "flag"
        },
        {
          "form": "--perfetto <path>",
          "description": "Write a gzipped Chrome Trace Event Format trace of this compilation to <path> (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link, per-function ESBMC proof spans, the compiler\u2192ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from --mode profile, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache.",
          "long": "--perfetto",
          "value_name": "path",
          "value_kind": "string"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/build-result.schema.json",
        "suppressed_by": [
          "--dump-ir"
        ]
      },
      "stderr": {
        "channels": [
          "diagnostic stream",
          "debug trace"
        ],
        "debug_trace_flag": "--debug-trace <off|calls|full>"
      },
      "notes": [
        "verification is enabled by default",
        "debug mode inserts runtime vow checks"
      ]
    },
    "verify": {
      "status": "implemented",
      "usage": "vow verify [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "--no-cache",
          "description": "Disable verification result caching",
          "long": "--no-cache",
          "value_kind": "flag"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (default: 50)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--solver <boolector|z3|bitwuzla|auto>",
          "description": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
          "long": "--solver",
          "value_name": "boolector|z3|bitwuzla|auto",
          "value_kind": "enum",
          "values": [
            "boolector",
            "z3",
            "bitwuzla",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--encoding <bv|ir|auto>",
          "description": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
          "long": "--encoding",
          "value_name": "bv|ir|auto",
          "value_kind": "enum",
          "values": [
            "bv",
            "ir",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--timeout <N>",
          "description": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
          "long": "--timeout",
          "value_name": "N",
          "value_kind": "integer",
          "default": "300 (or 30 when --encoding is auto)"
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Max concurrent ESBMC verification jobs (default: num_cpus/2)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        },
        {
          "form": "--replay-cex",
          "description": "Differential test of the verifier model against runtime semantics. After ESBMC reports a counterexample, build a --mode debug harness that calls the failing function with the counterexample's concrete inputs and check that the runtime VowViolation agrees (same vow_id and blame). Adds a replay field to each counterexample (see \"Counterexample replay\" below). Opt-in, off by default; also accepted by vow build.",
          "long": "--replay-cex",
          "value_kind": "flag"
        },
        {
          "form": "--perfetto <path>",
          "description": "Write a gzipped Chrome Trace Event Format trace of this verification run to <path> (load directly at ui.perfetto.dev). Captures frontend phase spans, per-function ESBMC proof spans, the compiler\u2192ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Pure side artifact.",
          "long": "--perfetto",
          "value_name": "path",
          "value_kind": "string"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/build-result.schema.json",
        "fixed_fields": {
          "executable": null
        }
      },
      "notes": [
        "runs verification only and never emits a binary"
      ]
    },
    "test": {
      "status": "implemented",
      "usage": "vow test [OPTIONS] [<path>]",
      "arguments": [
        {
          "name": "path",
          "kind": "path",
          "required": false,
          "default": ".",
          "description": "Directory to scan or single .vow file"
        }
      ],
      "options": [
        {
          "form": "--verify",
          "description": "Run ESBMC verification on test files",
          "long": "--verify",
          "value_kind": "flag"
        },
        {
          "form": "--filter <pat>",
          "description": "Only run tests whose file stem contains pat (default: (none))",
          "long": "--filter",
          "value_name": "pat",
          "value_kind": "string",
          "default": "(none)"
        },
        {
          "form": "--module-root <path>",
          "description": "Resolve use declarations against <path>. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. (default: (auto))",
          "long": "--module-root",
          "value_name": "path",
          "value_kind": "string",
          "default": "(auto)"
        },
        {
          "form": "--mode <debug|release>",
          "description": "Build mode; debug inserts runtime vow checks (default: (default))",
          "long": "--mode",
          "value_name": "mode",
          "value_kind": "enum",
          "values": [
            "debug",
            "release"
          ],
          "default": "(default)"
        },
        {
          "form": "--timeout <ms>",
          "description": "Per-test execution timeout in milliseconds (default: 30000)",
          "long": "--timeout",
          "value_name": "ms",
          "value_kind": "string",
          "default": "30000"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (with --verify)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Max concurrent ESBMC verification jobs (with --verify)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        }
      ],
      "stdout": {
        "format": "json"
      },
      "notes": [
        "discovers test_*.vow and *_test.vow files",
        "each test must contain main() -> i32 returning 0 on success",
        "default mode is debug (runtime vow checks enabled)"
      ]
    },
    "decl": {
      "status": "implemented",
      "usage": "vow decl [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "-o, --output <path>",
          "description": "Output declaration file path (default: <source>.vow.d)",
          "short": "-o",
          "long": "--output",
          "value_name": "path",
          "value_kind": "path",
          "default": "<source>.vow.d"
        }
      ],
      "stdout": {
        "format": "none"
      },
      "side_effects": [
        {
          "kind": "write_file",
          "default_path": "<source>.vow.d"
        }
      ]
    },
    "contracts": {
      "status": "implemented",
      "usage": "vow contracts [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "--verify",
          "description": "Run ESBMC verification and report per-contract status",
          "long": "--verify",
          "value_kind": "flag"
        },
        {
          "form": "--no-cache",
          "description": "Disable verification result caching",
          "long": "--no-cache",
          "value_kind": "flag"
        },
        {
          "form": "--max-k-step <N>",
          "description": "ESBMC incremental BMC max iterations (default: 50)",
          "long": "--max-k-step",
          "value_name": "N",
          "value_kind": "integer",
          "default": 50
        },
        {
          "form": "--solver <boolector|z3|bitwuzla|auto>",
          "description": "ESBMC SMT solver (with --verify)",
          "long": "--solver",
          "value_name": "boolector|z3|bitwuzla|auto",
          "value_kind": "enum",
          "values": [
            "boolector",
            "z3",
            "bitwuzla",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--encoding <bv|ir|auto>",
          "description": "ESBMC encoding mode (with --verify); ir requires z3 (default: auto)",
          "long": "--encoding",
          "value_name": "bv|ir|auto",
          "value_kind": "enum",
          "values": [
            "bv",
            "ir",
            "auto"
          ],
          "default": "auto"
        },
        {
          "form": "--verify-jobs <N>",
          "description": "Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial)",
          "long": "--verify-jobs",
          "value_name": "N",
          "value_kind": "integer",
          "default": "num_cpus/2"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/contracts-result.schema.json"
      },
      "notes": [
        "runs frontend only by default",
        "use --verify for per-contract ESBMC status"
      ]
    },
    "complexity": {
      "status": "implemented",
      "usage": "vow complexity [OPTIONS] <source.vow>",
      "arguments": [
        {
          "name": "source",
          "kind": "path",
          "required": true,
          "suffix": ".vow"
        }
      ],
      "options": [
        {
          "form": "--cog-anchor <N>",
          "description": "Cognitive-complexity value mapped to sub-score 0.800 (SonarQube's default flag line). (default: 15)",
          "long": "--cog-anchor",
          "value_name": "N",
          "value_kind": "integer",
          "default": "15"
        },
        {
          "form": "--nloc-anchor <N>",
          "description": "NLOC value mapped to sub-score 0.800 (~50\u201360 line guidance). (default: 60)",
          "long": "--nloc-anchor",
          "value_name": "N",
          "value_kind": "integer",
          "default": "60"
        },
        {
          "form": "--max-score <N>",
          "description": "CI gate: exit nonzero if any function's complexity_score exceeds N. The recommended line is 80, but gating is opt-in only. (default: (unset))",
          "long": "--max-score",
          "value_name": "N",
          "value_kind": "integer",
          "default": "(unset)"
        },
        {
          "form": "--max-cognitive <N>",
          "description": "CI gate: exit nonzero if any function's cognitive exceeds N. (default: (unset))",
          "long": "--max-cognitive",
          "value_name": "N",
          "value_kind": "integer",
          "default": "(unset)"
        },
        {
          "form": "--max-cyclomatic <N>",
          "description": "CI gate: exit nonzero if any function's cyclomatic exceeds N. (default: (unset))",
          "long": "--max-cyclomatic",
          "value_name": "N",
          "value_kind": "integer",
          "default": "(unset)"
        }
      ],
      "stdout": {
        "format": "json",
        "schema_ref": "schemas/complexity-result.schema.json"
      },
      "notes": [
        "reports only functions defined in the queried entry file",
        "complexity_score is a readability / refactor-priority gate, not a defect predictor",
        "--max-score / --max-cognitive / --max-cyclomatic gates are opt-in; exit nonzero only when set"
      ]
    }
  },
  "build_options": {
    "-o, --output <path>": "Output executable path (default: source without .vow extension)",
    "--mode <debug|release|profile|sanitize>": "Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking (default: release)",
    "--no-verify": "Skip ESBMC static verification",
    "--dump-ir": "Print IR text to stdout and exit (no JSON output, no codegen)",
    "--debug-trace <off|calls|full>": "Emit JSON trace lines to stderr at runtime (default: off)",
    "--no-cache": "Disable verification result caching, and (for --no-verify builds) the compile-object cache. See \"Compile-object cache behavior\" below",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (default: 50)",
    "--solver <boolector|z3|bitwuzla|auto>": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
    "--encoding <bv|ir|auto>": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
    "--timeout <N>": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
    "--verify-jobs <N>": "Max concurrent ESBMC verification jobs (default: num_cpus/2)",
    "--replay-cex": "Differential test the verifier model against runtime semantics (same as vow verify --replay-cex; see below)",
    "--perfetto <path>": "Write a gzipped Chrome Trace Event Format trace of this compilation to <path> (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link, per-function ESBMC proof spans, the compiler\u2192ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from --mode profile, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache."
  },
  "verify_options": {
    "--no-cache": "Disable verification result caching",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (default: 50)",
    "--solver <boolector|z3|bitwuzla|auto>": "ESBMC SMT solver; auto selects per-function via heuristic (default: auto)",
    "--encoding <bv|ir|auto>": "ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)",
    "--timeout <N>": "ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))",
    "--verify-jobs <N>": "Max concurrent ESBMC verification jobs (default: num_cpus/2)",
    "--replay-cex": "Differential test of the verifier model against runtime semantics. After ESBMC reports a counterexample, build a --mode debug harness that calls the failing function with the counterexample's concrete inputs and check that the runtime VowViolation agrees (same vow_id and blame). Adds a replay field to each counterexample (see \"Counterexample replay\" below). Opt-in, off by default; also accepted by vow build.",
    "--perfetto <path>": "Write a gzipped Chrome Trace Event Format trace of this verification run to <path> (load directly at ui.perfetto.dev). Captures frontend phase spans, per-function ESBMC proof spans, the compiler\u2192ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Pure side artifact."
  },
  "test_options": {
    "--verify": "Run ESBMC verification on test files",
    "--filter <pat>": "Only run tests whose file stem contains pat (default: (none))",
    "--module-root <path>": "Resolve use declarations against <path>. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. (default: (auto))",
    "--mode <debug|release>": "Build mode; debug inserts runtime vow checks (default: (default))",
    "--timeout <ms>": "Per-test execution timeout in milliseconds (default: 30000)",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (with --verify)",
    "--verify-jobs <N>": "Max concurrent ESBMC verification jobs (with --verify)"
  },
  "decl_options": {
    "-o, --output <path>": "Output declaration file path (default: <source>.vow.d)"
  },
  "contracts_options": {
    "--verify": "Run ESBMC verification and report per-contract status",
    "--no-cache": "Disable verification result caching",
    "--max-k-step <N>": "ESBMC incremental BMC max iterations (default: 50)",
    "--solver <boolector|z3|bitwuzla|auto>": "ESBMC SMT solver (with --verify)",
    "--encoding <bv|ir|auto>": "ESBMC encoding mode (with --verify); ir requires z3 (default: auto)",
    "--verify-jobs <N>": "Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial)"
  },
  "complexity_options": {
    "--cog-anchor <N>": "Cognitive-complexity value mapped to sub-score 0.800 (SonarQube's default flag line). (default: 15)",
    "--nloc-anchor <N>": "NLOC value mapped to sub-score 0.800 (~50\u201360 line guidance). (default: 60)",
    "--max-score <N>": "CI gate: exit nonzero if any function's complexity_score exceeds N. The recommended line is 80, but gating is opt-in only. (default: (unset))",
    "--max-cognitive <N>": "CI gate: exit nonzero if any function's cognitive exceeds N. (default: (unset))",
    "--max-cyclomatic <N>": "CI gate: exit nonzero if any function's cyclomatic exceeds N. (default: (unset))"
  },
  "global_options": {
    "--help": "Emit versioned JSON tool-help data",
    "--help --human": "Emit legacy human-readable help (compatibility mode)"
  },
  "outputs": {
    "build_result": {
      "schema_ref": "schemas/build-result.schema.json",
      "emitted_by": [
        "build",
        "verify"
      ],
      "status_values": [
        "Verified",
        "Unverified",
        "Skipped",
        "CompileFailed",
        "VerifyFailed"
      ],
      "legacy_fields": [
        "counterexample"
      ]
    },
    "contracts_result": {
      "schema_ref": "schemas/contracts-result.schema.json",
      "emitted_by": [
        "contracts"
      ]
    },
    "diagnostic": {
      "schema_ref": "schemas/diagnostic.schema.json",
      "embedded_in": "build_result.diagnostics"
    },
    "runtime_vow_violation": {
      "schema_ref": "schemas/vow-violation.schema.json",
      "emitted_on": "stderr",
      "requires_mode": "debug"
    },
    "runtime_trace": {
      "emitted_on": "stderr",
      "enabled_by": "--debug-trace <off|calls|full>",
      "format": "jsonl"
    }
  },
  "output_json": {
    "status": "Verified | Unverified | Skipped | CompileFailed | VerifyFailed",
    "executable": "path to compiled binary, or null",
    "diagnostics": "[array of {error_code, message, severity, span: {file, offset, length}}]",
    "message": "error detail (CompileFailed)",
    "function": "function name (VerifyFailed)",
    "counterexample": "ESBMC counterexample description (VerifyFailed)"
  },
  "diagnostics": {
    "schema_ref": "schemas/diagnostic.schema.json",
    "fields": [
      "error_code",
      "message",
      "severity",
      "span.file",
      "span.offset",
      "span.length"
    ]
  },
  "exit_codes": {
    "0": "success (Verified or Unverified)",
    "1": "failure (CompileFailed, VerifyFailed, or Skipped)"
  },
  "language": {
    "module": "module <Name>",
    "use_declaration": "use foo.bar",
    "const_declaration": "const NAME: i64 = 1024",
    "comments": "// line comments only; block comments unsupported",
    "let_binding": "let name: Type = expr; or let mut name: Type = expr;",
    "function": "fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "public_function": "pub fn <name>(<params>) -> <RetTy> [<effects>] { <body> }",
    "vow_function": "fn <name>(<params>) -> <RetTy> vow { requires: <expr>; ensures: <expr> } { <body> }",
    "while_with_invariant": "while <cond> vow { invariant: <expr> } { <body> }",
    "literals": {
      "integer": "42 | -1 | 42u64 (unsuffixed integers default to i64)",
      "float": "3.14 | -0.5",
      "bool": "true | false",
      "string": "\"text\" with escapes \\n \\t \\r \\\\ \\\" \\0"
    },
    "casts": "x as u64 or y as i64",
    "types": [
      "i8",
      "i16",
      "i32",
      "i64",
      "i128",
      "u8",
      "u16",
      "u32",
      "u64",
      "u128",
      "f32",
      "f64",
      "bool",
      "()",
      "!",
      "Vec<T>",
      "Option<T>",
      "Result<T, E>",
      "String",
      "HashMap<K, V>",
      "BTreeMap<K, V>"
    ],
    "effects": [
      "io",
      "read",
      "write",
      "panic",
      "unsafe"
    ],
    "builtins": {
      "pin_to_root": "fn(value: String) -> String and fn<T>(value: Vec<T>) -> Vec<T> for flat scalar T []",
      "print_str": "fn(s: String) -> () [io]",
      "print_i64": "fn(v: i64) -> () [io]",
      "print_u64": "fn(v: u64) -> () [io]",
      "eprintln_str": "fn(s: String) -> () [io]",
      "debug_str": "fn(s: String) -> () []",
      "debug_i64": "fn(v: i64) -> () []",
      "debug_u64": "fn(v: u64) -> () []",
      "fs_read": "fn(path: String) -> String [read]",
      "fs_open": "fn(path: String) -> i64 [read]",
      "fs_read_line": "fn(handle: i64) -> String [read]",
      "fs_status": "fn(handle: i64) -> i64 [read]",
      "fs_close": "fn(handle: i64) -> i64 [read]",
      "fs_write": "fn(path: String, data: String) -> i64 [write]",
      "fs_exists": "fn(path: String) -> i64 [read]",
      "fs_mkdir": "fn(path: String) -> i64 [io]",
      "fs_listdir": "fn(path: String) -> Vec<String> [read]",
      "fs_remove": "fn(path: String) -> i64 [io]",
      "fs_remove_dir": "fn(path: String) -> i64 [io]",
      "fs_is_dir": "fn(path: String) -> i64 [read]",
      "fs_is_symlink": "fn(path: String) -> i64 [read]",
      "fs_rename": "fn(old: String, new: String) -> i64 [io]",
      "string_substr": "fn(s: String, start: i64, len: i64) -> String []",
      "string_split": "fn(s: String, delim: String) -> Vec<String> []",
      "string_starts_with": "fn(s: String, prefix: String) -> i64 []",
      "string_ends_with": "fn(s: String, suffix: String) -> i64 []",
      "string_matches_literal_at": "fn(s: String, pos: i64, literal: String literal) -> i64 []",
      "string_trim": "fn(s: String) -> String []",
      "string_to_upper": "fn(s: String) -> String []",
      "string_to_lower": "fn(s: String) -> String []",
      "string_replace": "fn(s: String, from: String, to: String) -> String []",
      "string_join": "fn(parts: Vec<String>, sep: String) -> String []",
      "int_to_string": "fn(v: i64) -> String []",
      "uint_to_string": "fn(v: u64) -> String []",
      "i64_to_string": "fn(v: i64) -> String (alias of int_to_string) []",
      "vec_sort": "fn(v: Vec<i64>) -> Vec<i64> []",
      "time_unix": "fn() -> i64 [io]",
      "time_unix_ms": "fn() -> i64 [io]",
      "num_cpus": "fn() -> i64 [io]",
      "memory_root_arena_bytes": "fn() -> u64 [io]",
      "memory_peak_bytes": "fn() -> u64 [io]",
      "memory_alloc_count_since_start": "fn() -> u64 [io]",
      "hex_encode": "fn(data: Vec<u8>) -> String []",
      "hex_decode": "fn(s: String) -> Vec<u8> []",
      "args": "fn() -> Vec<String> [read]",
      "stdin_read": "fn() -> String [read]",
      "stdin_read_line": "fn() -> String [read]",
      "stdin_ready": "fn() -> bool [read]",
      "process_exit": "fn(code: i64) -> ! [io]",
      "process_run": "fn(cmd: String, args: Vec<String>) -> i64 [io]",
      "process_get_stdout": "fn() -> String [io]",
      "process_get_stderr": "fn() -> String [io]",
      "process_start": "fn(cmd: String, args: Vec<String>) -> i64 [io]",
      "process_wait": "fn(pid: i64) -> i64 [io]",
      "process_wait_timeout": "fn(pid: i64, timeout_ms: i64) -> i64 [io]",
      "process_kill": "fn(pid: i64) -> i64 [io]",
      "process_stdout_for": "fn(pid: i64) -> String [io]",
      "process_stderr_for": "fn(pid: i64) -> String [io]"
    },
    "operators": {
      "arithmetic": [
        "+",
        "-",
        "*",
        "/",
        "%"
      ],
      "checked_arithmetic": [
        "+!",
        "-!",
        "*!",
        "/!",
        "%!"
      ],
      "comparison": [
        "==",
        "!=",
        "<",
        "<=",
        ">",
        ">="
      ],
      "logical": [
        "&&",
        "||",
        "!"
      ],
      "bitwise": [
        "&",
        "|",
        "^",
        "<<",
        ">>"
      ],
      "unary": [
        "-",
        "!",
        "&",
        "?"
      ]
    },
    "vow_clauses": {
      "requires": "precondition \u2014 blame=Caller on violation",
      "ensures": "postcondition \u2014 blame=Callee on violation; use `result` for return value",
      "invariant": "loop invariant \u2014 checked at top of each iteration"
    },
    "where_clauses": "fn f(x: i64 where x >= 0) -> i64 \u2014 refinement types on parameters",
    "structs": {
      "definition": "struct Name { field: Type, ... }",
      "linear": "linear struct Name { field: Type, ... } \u2014 linear obligation must be consumed or returned before region close",
      "literal": "Name { field: value, ... }",
      "field_access": "value.field"
    },
    "enums": {
      "definition": "enum Name { Variant1(T), Variant2, Variant3 { field: T } }",
      "construction": "Name::Variant(value)",
      "builtin_option": "Option<T> \u2014 variants: Some(T), None",
      "builtin_result": "Result<T, E> \u2014 variants: Ok(T), Err(E)"
    },
    "match_expression": {
      "syntax": "match value { Pattern => expr, ... }",
      "patterns": [
        "Wildcard (_)",
        "Immutable identifier binding (value)",
        "Qualified enum variant (unit) (Option::None)",
        "Qualified enum variant (tuple payload) (Option::Some(value))"
      ]
    },
    "control_flow": {
      "if_else": "if cond { expr } else { expr } \u2014 expression, both branches same type",
      "while": "while cond { body }",
      "for_each": "for item in vec { body } \u2014 iterate Vec elements",
      "for_enumerate": "for i, item in vec { body } \u2014 iterate with index",
      "loop": "loop { ... break value; } \u2014 infinite loop, break to exit",
      "break": "break; or break value;",
      "return": "return; or return value;"
    },
    "modules": {
      "declaration": "module Name",
      "import": "use foo.bar \u2014 resolves to <rootdir>/foo/bar.vow",
      "visibility": "pub fn \u2014 public functions visible to importers"
    },
    "type_aliases": "type Name = Type",
    "extern_blocks": "extern \"C\" vow { requires: ... } { fn name(x: i64) -> i64 [unsafe] }",
    "methods": {
      "Vec<T>": [
        "Vec::new()",
        "Vec::from_raw_parts_copy(ptr, len)",
        ".push(val)",
        ".pop()",
        ".len()",
        ".clear()",
        ".truncate(n)",
        "v[i]",
        "v[i] = val"
      ],
      "String": [
        "String::from(s)",
        "String::new()",
        "String::from_raw_parts_copy(ptr, len)",
        ".len()",
        ".byte_at(i)",
        ".push_byte(b)",
        ".push_str(s)",
        ".clear()",
        ".contains(s)",
        ".eq(s)",
        ".substring(start, end)",
        ".parse_i64()",
        ".parse_u64()"
      ],
      "HashMap<K,V>": [
        "HashMap::new()",
        ".insert(k, v)",
        ".get(k)",
        ".contains_key(k)",
        ".remove(k)",
        ".len()"
      ],
      "BTreeMap<K,V>": [
        "BTreeMap::new()",
        ".insert(k, v)",
        ".get(k)",
        ".contains(k)",
        ".len()"
      ],
      "Option<T>": [
        ".unwrap()",
        "? operator"
      ]
    },
    "error_propagation": "? on Option<T> or Result<T, E> propagates None/Err to the caller",
    "indexing": {
      "read": "v[i] \u2014 Vec index access",
      "write": "v[i] = val \u2014 Vec index assignment"
    },
    "feature_status": {
      "implemented": {
        "function_vow_blocks": "requires / ensures / invariant",
        "where_clauses": "parameter-level refinement sugar",
        "loop_invariants": "simple invariant predicates"
      },
      "partial": {
        "refinement_type_predicates": "rejected with a type error (fail-closed, never silently unverified); use a where clause on the parameter or a requires/ensures contract",
        "effect_tracking": "user-defined effect propagation is enforced; some builtin panic/unsafe effects are not yet modeled"
      },
      "target": {
        "module_level_vow_blocks": "specified in docs but not parsed or represented in the AST",
        "quantifiers": "forall / exists are not yet in the lexer or parser"
      },
      "unsupported": [
        "user-defined generics",
        "traits",
        "closures",
        "operator overloading",
        "macros",
        "assert / assume statements"
      ]
    }
  },
  "verification_defaults": {
    "strategy": "incremental-bmc",
    "max_k_step": 50
  }
}"##
    .to_string()
}
// GENERATE:SKILL_JSON:END

// GENERATE:SKILL_HUMAN:START
fn skill_human() -> String {
    r##"vow — Vow compiler

USAGE
  vow build [OPTIONS] <source.vow>    Compile to native executable
  vow verify [OPTIONS] <source.vow>    Verify contracts only (no executable)
  vow test [OPTIONS] [<path>]          Run tests with JSON results
  vow contracts [OPTIONS] <source.vow> List all contracts
  vow complexity [OPTIONS] <source.vow> Report per-function complexity metrics (JSON)
  vow decl [OPTIONS] <source.vow>    Emit declaration file (.vow.d)
  vow skill [print [--bundle]|install [--local|--global]]
                                        Generate or install Claude Code skill
  vow [OPTIONS] <source.vow>          Legacy mode (same as vow build)

BUILD OPTIONS
  -o, --output <path>     Output executable path (default: source without .vow extension)
  --mode <debug|release|profile|sanitize>  Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking (default: release)
  --no-verify             Skip ESBMC static verification
  --dump-ir               Print IR text to stdout and exit (no JSON output, no codegen)
  --debug-trace <off|calls|full>  Emit JSON trace lines to stderr at runtime (default: off)
  --no-cache              Disable verification result caching, and (for --no-verify builds) the compile-object cache. See "Compile-object cache behavior" below
  --max-k-step <N>        ESBMC incremental BMC max iterations (default: 50)
  --solver <boolector|z3|bitwuzla|auto>  ESBMC SMT solver; auto selects per-function via heuristic (default: auto)
  --encoding <bv|ir|auto>  ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)
  --timeout <N>           ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))
  --verify-jobs <N>       Max concurrent ESBMC verification jobs (default: num_cpus/2)
  --replay-cex            Differential test the verifier model against runtime semantics (same as vow verify --replay-cex; see below)
  --perfetto <path>       Write a gzipped Chrome Trace Event Format trace of this compilation to <path> (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from --mode profile, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache.

VERIFY OPTIONS
  --no-cache              Disable verification result caching
  --max-k-step <N>        ESBMC incremental BMC max iterations (default: 50)
  --solver <boolector|z3|bitwuzla|auto>  ESBMC SMT solver; auto selects per-function via heuristic (default: auto)
  --encoding <bv|ir|auto>  ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 (default: auto)
  --timeout <N>           ESBMC per-function timeout in seconds. Under --encoding auto, a 30s default is applied so the BV-timeout fallback to --encoding ir --solver z3 can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit --timeout overrides both. --timeout 0 is honoured as an immediate watchdog kill (default: 300 (or 30 when --encoding is auto))
  --verify-jobs <N>       Max concurrent ESBMC verification jobs (default: num_cpus/2)
  --replay-cex            Differential test of the verifier model against runtime semantics. After ESBMC reports a counterexample, build a --mode debug harness that calls the failing function with the counterexample's concrete inputs and check that the runtime VowViolation agrees (same vow_id and blame). Adds a replay field to each counterexample (see "Counterexample replay" below). Opt-in, off by default; also accepted by vow build.
  --perfetto <path>       Write a gzipped Chrome Trace Event Format trace of this verification run to <path> (load directly at ui.perfetto.dev). Captures frontend phase spans, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Pure side artifact.

TEST OPTIONS
  --verify                Run ESBMC verification on test files
  --filter <pat>          Only run tests whose file stem contains pat (default: (none))
  --module-root <path>    Resolve use declarations against <path>. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. (default: (auto))
  --mode <debug|release>  Build mode; debug inserts runtime vow checks (default: (default))
  --timeout <ms>          Per-test execution timeout in milliseconds (default: 30000)
  --max-k-step <N>        ESBMC incremental BMC max iterations (with --verify)
  --verify-jobs <N>       Max concurrent ESBMC verification jobs (with --verify)

CONTRACTS OPTIONS
  --verify                Run ESBMC verification and report per-contract status
  --no-cache              Disable verification result caching
  --max-k-step <N>        ESBMC incremental BMC max iterations (default: 50)
  --solver <boolector|z3|bitwuzla|auto>  ESBMC SMT solver (with --verify)
  --encoding <bv|ir|auto>  ESBMC encoding mode (with --verify); ir requires z3 (default: auto)
  --verify-jobs <N>       Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial)

COMPLEXITY OPTIONS
  --cog-anchor <N>        Cognitive-complexity value mapped to sub-score 0.800 (SonarQube's default flag line). (default: 15)
  --nloc-anchor <N>       NLOC value mapped to sub-score 0.800 (~50–60 line guidance). (default: 60)
  --max-score <N>         CI gate: exit nonzero if any function's complexity_score exceeds N. The recommended line is 80, but gating is opt-in only. (default: (unset))
  --max-cognitive <N>     CI gate: exit nonzero if any function's cognitive exceeds N. (default: (unset))
  --max-cyclomatic <N>    CI gate: exit nonzero if any function's cyclomatic exceeds N. (default: (unset))

DECL OPTIONS
  -o, --output <path>     Output declaration file path (default: <source>.vow.d)

GLOBAL OPTIONS
  --help                Emit versioned JSON tool-help data
  --help --human        Emit legacy text help

OUTPUT (JSON on stdout)
  status      : Verified | Unverified | Skipped | CompileFailed | VerifyFailed
  executable  : path to compiled binary, or null
  diagnostics : array of {error_code, message, severity, span: {file, offset, length}}
  message     : error detail (CompileFailed)
  function    : function name (VerifyFailed)
  counterexample: ESBMC counterexample (VerifyFailed)

EXIT CODES
  0  success (Verified or Unverified)
  1  failure (CompileFailed, VerifyFailed, or Skipped)

LANGUAGE SUMMARY
  module Hello
  use math.utils

  struct Point { x: i64, y: i64 }

  fn add(x: i64, y: i64) -> i64 {
    x + y
  }

  fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
    ensures:  result * y == x
  } {
    x / y
  }

  fn main() -> i32 [io] {
    let v: Vec<i64> = Vec::new();
    v.push(divide(10, 2));
    print_i64(v[0]);
    0
  }

TYPES     : i8  i16  i32  i64  i128  u8  u16  u32  u64  u128  f32  f64  bool  ()  !  Vec<T>  Option<T>  Result<T, E>  String  HashMap<K, V>  BTreeMap<K, V>
EFFECTS   : io  read  write  panic  unsafe
BUILTINS  : pin_to_root: fn(value: String) -> String and fn<T>(value: Vec<T>) -> Vec<T> for flat scalar T []   print_str: fn(s: String) -> () [io]   print_i64: fn(v: i64) -> () [io]
            print_u64: fn(v: u64) -> () [io]   eprintln_str: fn(s: String) -> () [io]   debug_str: fn(s: String) -> () []   debug_i64: fn(v: i64) -> () []   debug_u64: fn(v: u64) -> () []   fs_read: fn(path: String) -> String [read]   fs_open: fn(path: String) -> i64 [read]   fs_read_line: fn(handle: i64) -> String [read]   fs_status: fn(handle: i64) -> i64 [read]   fs_close: fn(handle: i64) -> i64 [read]   fs_write: fn(path: String, data: String) -> i64 [write]   fs_exists: fn(path: String) -> i64 [read]   fs_mkdir: fn(path: String) -> i64 [io]   fs_listdir: fn(path: String) -> Vec<String> [read]   fs_remove: fn(path: String) -> i64 [io]   fs_remove_dir: fn(path: String) -> i64 [io]   fs_is_dir: fn(path: String) -> i64 [read]   fs_is_symlink: fn(path: String) -> i64 [read]   fs_rename: fn(old: String, new: String) -> i64 [io]   string_substr: fn(s: String, start: i64, len: i64) -> String []   string_split: fn(s: String, delim: String) -> Vec<String> []   string_starts_with: fn(s: String, prefix: String) -> i64 []   string_ends_with: fn(s: String, suffix: String) -> i64 []   string_matches_literal_at: fn(s: String, pos: i64, literal: String literal) -> i64 []   string_trim: fn(s: String) -> String []   string_to_upper: fn(s: String) -> String []   string_to_lower: fn(s: String) -> String []   string_replace: fn(s: String, from: String, to: String) -> String []   string_join: fn(parts: Vec<String>, sep: String) -> String []   int_to_string: fn(v: i64) -> String []   uint_to_string: fn(v: u64) -> String []   i64_to_string: fn(v: i64) -> String (alias of int_to_string) []   vec_sort: fn(v: Vec<i64>) -> Vec<i64> []   time_unix: fn() -> i64 [io]   time_unix_ms: fn() -> i64 [io]   num_cpus: fn() -> i64 [io]   memory_root_arena_bytes: fn() -> u64 [io]   memory_peak_bytes: fn() -> u64 [io]   memory_alloc_count_since_start: fn() -> u64 [io]   hex_encode: fn(data: Vec<u8>) -> String []   hex_decode: fn(s: String) -> Vec<u8> []   args: fn() -> Vec<String> [read]   stdin_read: fn() -> String [read]   stdin_read_line: fn() -> String [read]   stdin_ready: fn() -> bool [read]   process_exit: fn(code: i64) -> ! [io]   process_run: fn(cmd: String, args: Vec<String>) -> i64 [io]   process_get_stdout: fn() -> String [io]   process_get_stderr: fn() -> String [io]   process_start: fn(cmd: String, args: Vec<String>) -> i64 [io]   process_wait: fn(pid: i64) -> i64 [io]   process_wait_timeout: fn(pid: i64, timeout_ms: i64) -> i64 [io]   process_kill: fn(pid: i64) -> i64 [io]   process_stdout_for: fn(pid: i64) -> String [io]   process_stderr_for: fn(pid: i64) -> String [io]
METHODS   : Vec: Vec::new/Vec::from_raw_parts_copy/push/pop/len/clear/truncate/v[i]/v[i] = val   String: String::from/String::new/String::from_raw_parts_copy/len/byte_at/push_byte/push_str/clear/contains/eq/substring/parse_i64/parse_u64
            HashMap: HashMap::new/insert/get/contains_key/remove/len   BTreeMap: BTreeMap::new/insert/get/contains/len   Option: unwrap
OPERATORS : + - * / %   +! -! *! /! %! (checked)   == != < <= > >=   && || !   & | ^ << >> (bitwise, integer-only)   unary - ! & ?

VERIFICATION DEFAULTS (--max-k-step)
  Strategy        : incremental-bmc (incremental BMC up to --max-k-step; forward-condition completeness, no k-induction step)
  Incremental BMC : 50 max iterations (--max-k-step)"##
        .to_string()
}
// GENERATE:SKILL_HUMAN:END

// GENERATE:SKILL_FULL:START
fn skill_entrypoint_markdown() -> String {
    r#"---
name: vow
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, ESBMC counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
---

# Vow

Use this skill when writing, compiling, debugging, or verifying Vow programs.
Keep the workflow tight: run the compiler, read the structured JSON, fix the
program or contract, and repeat until the result is `Verified`.

## Installed toolchain (live)

!`(command -v vow >/dev/null 2>&1 && vow --help 2>/dev/null | head -200) || (command -v build/vowc >/dev/null 2>&1 && build/vowc --help 2>/dev/null | head -200) || echo '(vow toolchain not found on PATH; run scripts/bootstrap.sh to build build/vowc)'`

## Core workflow

1. Write a `.vow` file with explicit contracts.
2. Run `ulimit -v 2000000; build/vowc build <file.vow>`.
3. Parse stdout JSON and inspect `status`, `diagnostics`, and `counterexamples`.
4. Fix compile errors, verification failures, or weak contracts, then rerun.

## Minimal program

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

## Reference files

- Grammar, types, effects, builtins: [reference/grammar.md](reference/grammar.md)
- CLI commands, flags, JSON output: [reference/cli.md](reference/cli.md)
- Contracts and CEGIS guidance: [reference/contracts.md](reference/contracts.md)
- Which contracts to write (taxonomy & strength): [reference/contracts-methodology.md](reference/contracts-methodology.md)
- Diagnostics and fixes: [reference/errors.md](reference/errors.md)
- Standard library (math, heap, stack, geometry, bignum, gc): [reference/stdlib.md](reference/stdlib.md)
- Worked examples: [examples/examples.md](examples/examples.md)
- JSON schemas: [schemas/](schemas/)
"#
    .to_string()
}

fn skill_bundle_markdown() -> String {
    r##"---
name: vow
description: >-
  Write, compile, debug, and verify Vow programs (.vow files) with contracts,
  CEGIS, ESBMC counterexamples, diagnostics, and vow build / vow verify.
when_to_use: >-
  Use when the user edits or creates .vow files, says "write a Vow program",
  "fix this counterexample", "add contracts", "why did verification fail",
  "ESBMC", "vow build", or "vow verify".
argument-hint: "[file.vow]"
---

# Vow Language Reference

Vow is a systems programming language with built-in contracts (preconditions, postconditions, loop invariants) that are statically verified by ESBMC bounded model checking. Programs compile to native executables via Cranelift. The compiler emits structured JSON for machine consumption.

In all documentation below, `vow` refers to the `build/vowc` binary. Always use `ulimit -v 2000000` before invoking the compiler or any binary it produces — without this, the process can consume all system memory.

## What Vow Excludes

No block comments, no generics, no traits, no closures, no macros, no garbage collection. Line comments (`//`) are supported.

## CEGIS Workflow

The standard workflow for writing verified Vow programs:

1. **Write** — Create a `.vow` file with function contracts (`requires`, `ensures`, `invariant`)
2. **Build** — Run `ulimit -v 2000000; build/vowc build <file.vow>`
3. **Parse JSON** — Read the JSON object from stdout
4. **Handle status:**
   - `Verified` → Done. Binary is at `executable`.
   - `Unverified` → Compiled but ESBMC not available. Binary is at `executable`.
   - `CompileFailed` → Read `diagnostics[]` for errors. Fix and retry.
   - `VerifyFailed` → Read `counterexamples[]`. Fix contracts or implementation. Retry.
5. **Iterate** — Repeat steps 2–4 until `Verified`.

## Minimal Program

```vow
module Hello

fn main() -> i32 [io] {
    print_str("Hello, world!");
    0
}
```

Build and run (`build/vowc` is the primary compiler binary, produced by `scripts/bootstrap.sh`):
```
$ ulimit -v 2000000; build/vowc build hello.vow
$ ulimit -v 2000000; ./hello
Hello, world!
```

## Development Discipline

Vow is written by agents under a finite context window. These principles apply to every task, not just language design.

**Deep modules, not shallow ones.** A deep module packs a lot of functionality behind a simple interface and hides complexity. A shallow module exposes a complex interface for not much functionality and surfaces complexity to every caller. Prefer deep. When you design or extend a module, ask: does the interface hide more complexity than it exposes? If not, collapse the module or move its logic behind a narrower boundary. Many exported symbols, thin wrappers, and "pass-through" functions are signs of a shallow module.

**Surgical changes.** Prefer many small changes over one large change. Small changes are easier to review, debug, bisect with `git bisect`, and revert. A bug fix fixes the bug — it does not also refactor the surrounding code, rename variables, or reformat the file. Unrelated improvements belong in their own commits or PRs. If a task grows, split it; do not bundle.

**Keep files small, functions smaller.** Context is precious. Every file an agent reads consumes budget that could go toward solving the problem. A 2000-line file forces whole-file reads and pushes other context out; a 200-line file does not. A 100-line function is harder to reason about than four 25-line functions with clear names. Split by responsibility as soon as a unit stops fitting a single coherent idea. This applies to both Vow source code and any tooling around it.

---

# Vow Grammar Reference

Complete grammar for the Vow programming language. Vow source files use the `.vow` extension.

**Line comments.** `//` starts a line comment extending to end of line. Comments are stripped during lexing and never enter the token stream. Block comments (`/* */`) are not supported. Machine-relevant intent belongs in contracts; comments are for non-semantic rationale.

## Module Declaration

Every file begins with a module declaration:

```
module <Name>
```

`<Name>` is a PascalCase identifier. There is no semicolon.

## Use Declarations

Import other modules with dot-separated paths:

```
use foo.bar
```

This resolves to `<rootdir>/foo/bar.vow` relative to the main source file.

## Const Declarations

Named constants with compile-time values:

```vow
const MAX_SIZE: i64 = 1024;
const NEG_ONE: i64 = -1;
const DEBUG: bool = true;
```

Supported value forms: integer literals, boolean literals, negated integer literals. Constants are inlined at every use site (zero runtime cost). The type must be any of the 10 integer types (`i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`) or `bool`. Integer constants are subject to the same compile-time range check as integer literals. Constants are referenced by name in expressions like any other identifier.

## Functions

### Pure Function

```vow
fn add(x: i64, y: i64) -> i64 {
    x + y
}
```

### Function with Effects

```vow
fn main() -> i32 [io] {
    print_str("hello");
    0
}
```

Effects appear in brackets after the return type: `[io]`, `[read, write]`, `[io, panic]`.

### Function with Vow Block

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

The `vow` block sits between the signature and the body. Clauses:
- `requires: <expr>` — precondition (blame: Caller)
- `ensures: <expr>` — postcondition (blame: Callee); use `result` for the return value
- `invariant: <expr>` — loop invariant (blame: Callee)

Multiple clauses are separated by commas:

```vow
fn clamp(x: i64, lo: i64, hi: i64) -> i64 vow {
    requires: lo <= hi,
    ensures: result >= lo,
    ensures: result <= hi
} {
    if x < lo { lo } else { if x > hi { hi } else { x } }
}
```

### Where Clauses (Refinement Types on Parameters)

```vow
fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}
```

`where` constraints on parameters become additional `requires` in verification. Each `where` clause can only reference its own parameter — it cannot reference other parameters.

### Public Functions

```vow
pub fn api_function(x: i64) -> i64 {
    x
}
```

## Types

### Primitive Types

| Type   | Description              |
|--------|--------------------------|
| `i8`   | 8-bit signed integer     |
| `i16`  | 16-bit signed integer    |
| `i32`  | 32-bit signed integer    |
| `i64`  | 64-bit signed integer    |
| `i128` | 128-bit signed integer (verifier may time out; see below) |
| `u8`   | 8-bit unsigned integer   |
| `u16`  | 16-bit unsigned integer  |
| `u32`  | 32-bit unsigned integer  |
| `u64`  | 64-bit unsigned integer  |
| `u128` | 128-bit unsigned integer (verifier may time out; see below) |
| `f32`  | 32-bit float (limited support — avoid in contracts) |
| `f64`  | 64-bit float (limited support — avoid in contracts) |
| `bool` | Boolean                  |
| `()`   | Unit type                |
| `!`    | Never type (diverges)    |

There is no `isize`/`usize`. Vow targets 64-bit only; `Vec::len()` returns `i64`,
indices are `i64`. This is deliberate — it preserves binary fixed point
reproducibility across compilations. See [ADR 0001](../adr/0001-numeric-tower-narrow-ints.md).

**128-bit verification:** `i128`/`u128` arithmetic codegens via Cranelift's
`I128` and verifies via ESBMC's `__int128`. Predicates over 128-bit values may
exceed reasonable SMT solver timeouts; the `--no-128-verify` flag skips
verification for functions whose contracts mention 128-bit values while still
generating native code for them.

**Struct field layout:** every struct field up to 64 bits wide occupies one
8-byte slot regardless of declared type (narrow ints are padded); `i128`/`u128`
fields occupy two consecutive 8-byte slots (16 bytes). There is no packing or
natural-alignment layout today; FFI structs that need a specific C layout must
shim through `Vec<u8>` or extern wrappers.

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array                  |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan)     |
| `BTreeMap<K, V>`   | Sorted key-value map (binary search; ascending iteration). `K` must be `i64`; `V` may be any non-linear type |

### User-Defined Types

Structs and enums (see below).

## Literals

### Integer Literals

```vow
42
-1
0
```

Unsuffixed integer literals default to `i64` in expression position, and
**context-coerce** to any of the 10 integer types when the
surrounding context fixes one — `let` bindings, function arguments, struct
fields, and the typed operand of an arithmetic, bitwise, or comparison
operator. The same coercion applies to constant expressions composed entirely
of unsuffixed integer literals (e.g. `1 + 2`, `1 << 3`, `-5`).

Out-of-range literals in a typed context are a compile-time error:

```vow
let x: u8 = 300;   // error: LiteralOutOfRange — 300 does not fit in u8
let y: i8 = 200;   // error: LiteralOutOfRange — i8 range is -128..=127
```

**Suffixed integer literals** force the type at the literal:

```vow
42u8     42u16     42u32     42u64     42u128
42i8     42i16     42i32     42i64     42i128
```

Suffixed forms are supported for all 10 integer widths. They override context
coercion and are still subject to the same compile-time range check.

### Float Literals

```vow
3.14
-0.5
```

### Boolean Literals

```vow
true
false
```

### String Literals

```vow
"hello, world"
"line one\nline two"
"tab\there"
"null\0byte"
"escaped\\backslash"
"escaped\"quote"
```

Supported escape sequences: `\n`, `\t`, `\r`, `\\`, `\"`, `\0`.

String literals have type `String` and are backed by a read-only static
descriptor. Passing or returning a literal does not allocate. To obtain a
mutable, arena-owned copy, use `String::from("...")`.

## Operators

### Wrapping Arithmetic (default)

| Operator | Meaning        |
|----------|----------------|
| `+`      | Add (wrapping) |
| `-`      | Sub (wrapping) |
| `*`      | Mul (wrapping) |
| `/`      | Div (wrapping) |
| `%`      | Rem (wrapping) |

Wrapping operators silently wrap on overflow. For unsigned operands, including
`u8`, division and remainder use unsigned semantics.

### Checked Arithmetic

| Operator | Meaning           |
|----------|-------------------|
| `+!`     | Add (checked)     |
| `-!`     | Sub (checked)     |
| `*!`     | Mul (checked)     |
| `/!`     | Div (checked)     |
| `%!`     | Rem (checked)     |

Checked operators abort with `ArithmeticOverflow` on overflow.

### Saturating Arithmetic

Saturating arithmetic uses named compiler intrinsics rather than a third
operator family. The `u8` intrinsics are:

| Function | Signature | Behavior |
|----------|-----------|----------|
| `add_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps sums above 255 to 255 |
| `sub_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps differences below 0 to 0 |
| `mul_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps products above 255 to 255 |

These functions are pure and have direct verifier semantics; they do not
lower to wrapping arithmetic.

### Comparison Operators

| Operator | Meaning                |
|----------|------------------------|
| `==`     | Equal                  |
| `!=`     | Not equal              |
| `<`      | Less than              |
| `<=`     | Less than or equal     |
| `>`      | Greater than           |
| `>=`     | Greater than or equal  |

### Bitwise Operators

| Operator | Meaning      |
|----------|--------------|
| `&`      | Bitwise AND  |
| `\|`     | Bitwise OR   |
| `^`      | Bitwise XOR  |
| `<<`     | Left shift   |
| `>>`     | Right shift  |

Bitwise `& | ^` require integer operands of the same type and work on all 10
integer widths. `>>` is **arithmetic** (sign-extending) for signed types
(`i8`..`i128`) and **logical** (zero-extending) for unsigned types
(`u8`..`u128`).

**Shift count type.** The right operand of `<<` and `>>` is `u32`. Unsuffixed
integer literals on the right side context-coerce to `u32`: given
`let x: u8 = ...`, `x << 3` is well-typed (`3` coerces to `u32`). The left
operand keeps its own integer type; the shift result has the left operand's
type.

**Shift count range.** A const-expression shift count `>= bit-width(LHS)` is a
compile-time error (`ShiftCountOutOfRange`). For example, `(x: u8) << 8` does
not compile. Dynamic shift counts (`x << n` where `n` is not a const
expression) get a contract on the operation that ESBMC checks: the count must
be less than the LHS width at the point of the shift.

Unsuffixed literal coercion still applies for `&`, `|`, `^` operands: with
`let x: u64 = ...`, `3 & x` and `x | 0xff` type-check because the literal
side coerces to `u64`. Use a suffix to force a different type explicitly.

### Logical Operators

| Operator | Meaning    |
|----------|------------|
| `&&`     | Logical AND (short-circuit) |
| `\|\|`   | Logical OR (short-circuit) |
| `!`      | Logical NOT|

`&&` and `||` use short-circuit evaluation: for `a && b`, `b` is only evaluated if `a` is true; for `a || b`, `b` is only evaluated if `a` is false.

### Operator Precedence

From loosest to tightest, Vow follows the usual C/Rust precedence for logical and bitwise operators:

`||`, `&&`, comparisons (`== != < <= > >=`), `|`, `^`, `&`, `<< >>`, `+ -`, `* / %`

Unary `-`, `!`, `&`, and `?` bind tighter than every binary operator.

Single `&` is overloaded by position: prefix `&expr` is borrow, while infix `lhs & rhs` is bitwise AND.

### Unary Operators

| Operator | Meaning    |
|----------|------------|
| `-`      | Negation (not allowed on `u64`) |
| `!`      | Logical NOT|
| `&`      | Borrow     |
| `?`      | Unwrap (propagate error) |

### Type Cast

`as` is **widening-only** across integer types. Any narrower integer can be
cast to any wider integer; signed sources sign-extend, unsigned sources
zero-extend:

```vow
let a: i32 = -1;
let b: i64 = a as i64;     // sign-extend: -1_i64
let c: u8  = 200;
let d: u64 = c as u64;     // zero-extend: 200_u64
let e: u32 = 1;
let f: i64 = e as i64;     // unsigned-to-signed widening, value preserved
```

`as` between signed and unsigned of **the same width** is also allowed
(machine-level bit reinterpretation): `i64 as u64`, `u64 as i64`, `i32 as u32`,
etc.

**Narrowing via `as` is a compile-time error** (`NarrowingCastNotAllowed`):

```vow
let big: i64 = 300;
let small: u8 = big as u8;     // error — narrowing not allowed via `as`
```

To narrow, use a named intrinsic that makes the intent explicit. For every
narrowing pair `(src, tgt)` the compiler exposes three free functions:

| Intrinsic                         | Behavior on out-of-range input          |
|-----------------------------------|-----------------------------------------|
| `<src>_to_<tgt>_try(x) -> Option<tgt>` | returns `Option::None`             |
| `<src>_to_<tgt>_wrap(x) -> tgt`   | truncates (low bits, two's-complement)  |
| `<src>_to_<tgt>_sat(x) -> tgt`    | clamps to the target type's range       |

Example:

```vow
let big: i64 = 300;
match i64_to_u8_try(big) {
    Option::Some(b) => use_byte(b),
    Option::None    => fallback(),
}
```

These intrinsics are emitted by the compiler so ESBMC sees their semantics
directly in the verification C model.

For the `u8` target, the available narrowing source types are `i16`, `i32`,
`i64`, `i128`, `u16`, `u32`, `u64`, and `u128`. Each source provides all three
forms, for example `u16_to_u8_try`, `u16_to_u8_wrap`, and `u16_to_u8_sat`.

No implicit conversions: `i64 + u64` and `u8 + i32` are type errors. The
operands must already have the same type. The compiler does not coerce
across integer types at operator sites — only literals coerce, per the
[Integer Literals](#integer-literals) rules.

## Let Bindings

### Immutable

```vow
let x: i64 = 42;
x = 43;   // error[ImmutableAssignment]: declare it with `let mut x`
```

Bindings are immutable by default. Reassigning a binding that was not declared
`mut` is a compile error (`ImmutableAssignment`). `mut` is required **only** for
whole-binding reassignment `x = e`; field writes (`s.f = e`) and index writes
(`v[i] = e`) are permitted through any binding and do not require the base to be
`mut`.

### Mutable

```vow
let mut i: i64 = 0;
i = i + 1;
```

A `let mut` binding that is never reassigned is a compile error (`UnusedMut`) —
drop the `mut`. Because only whole-binding reassignment counts as a use of `mut`,
a binding mutated solely via `s.f = e`, `v[i] = e`, or a method call should be
declared `let`, not `let mut`.

### Pattern Destructuring

```vow
let (a, b): (i64, i64) = (1, 2);
```

## Control Flow

### If / Else

```vow
if x > 0 {
    x
} else {
    0 - x
}
```

`if`/`else` is an expression — both branches must have the same type. There is no `else if` keyword; nest `if` inside `else`:

```vow
if x < lo {
    lo
} else {
    if x > hi {
        hi
    } else {
        x
    }
}
```

### While Loop

```vow
while i > 0 {
    i = i - 1;
}
```

### While Loop with Invariant

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    v.push(i);
    i = i + 1;
}
```

### For-Each Loop

```vow
for x in vec {
    print_i64(x);
}
```

Iterates over each element of a `Vec<T>`. The loop variable `x` is bound to each element in turn. Desugars to a `while` loop with index arithmetic — zero verification overhead.

### For-Each Loop with Invariant

```vow
for x in vec vow {
    invariant: total >= 0
} {
    total = total + x;
}
```

### Loop (Infinite)

`loop` creates an infinite loop. The expression returns the type of the `break` value:

```vow
let idx: i64 = loop {
    if data[i] == target {
        break i;
    }
    i = i + 1;
    if i >= n { break -1; }
};
```

ESBMC cannot verify unbounded `loop` constructs — use `while` with invariants for verifiable loops.

### Break

`break` exits the innermost loop. Inside `loop`, `break value` sets the loop's result:

```vow
break;           // exit while or loop (loop returns Unit)
break value;     // exit loop with a value (only inside loop, not while)
```

### Continue

`continue` skips the remaining statements in the current loop iteration and jumps back to the loop header:

```vow
continue;        // skip to next iteration of while, loop, or for
```

Inside `while` and `loop`, `continue` emits back-edge values for any mutated variables. Inside `for`, it also advances the loop index.

### Return

```vow
return;
return value;
```

## Struct Definitions

```vow
struct Point {
    x: i64,
    y: i64,
}
```

### Linear Structs

```vow
linear struct FileHandle {
    fd: i64,
}
```

Linear struct values carry a linear obligation. The obligation must either be consumed before the value's owning region closes or transferred to the caller by returning the value.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

### Field Access

```vow
p.x
```

### Field Assignment

```vow
p.x = 10;
```

### Passing Semantics

Structs are heap-allocated. A struct value is a pointer to a heap region, so passing a struct to a function passes the pointer — the function operates on the same heap data, not a copy. Field assignments inside the called function are visible to the caller:

```vow
fn shift_right(p: Point, dx: i64) {
    p.x = p.x + dx;
}

fn main() -> i32 [io] {
    let p: Point = Point { x: 0, y: 0 };
    shift_right(p, 5);
    print_i64(p.x);  // 5 — mutation visible to caller
    0
}
```

This enables in-place mutation patterns (e.g., make/unmake in search trees) without cloning. The same aliasing semantics apply when structs are stored in containers — see [Indexing](#indexing). To avoid aliasing, construct a fresh struct literal with the desired field values.

**Note:** For `linear struct` types, passing the value to a function consumes it; the caller cannot access it afterward. Returning a linear value transfers the obligation to the caller, so this is the normal way to hand an updated linear value back out of a function.

## Enum Definitions

```vow
enum Shape {
    Circle(i64),
    Rect(i64, i64),
    Empty,
}
```

Variant kinds: unit (`Empty`), tuple (`Circle(i64)`), struct (`Named { x: i64 }`).

### Enum Construction

```vow
let s: Shape = Shape::Circle(5);
let none: Option<i64> = Option::None;
let some: Option<i64> = Option::Some(42);
```

### Built-in Enums

`Option<T>` has variants `Some(T)` and `None`.
`Result<T, E>` has variants `Ok(T)` and `Err(E)`.

## Pattern Matching

```vow
match value {
    Pattern1 => expr1,
    Pattern2 => expr2,
    _ => default_expr,
}
```

Match is an expression. The scrutinee must have an enum type, including an
applied built-in enum such as `Option<T>` or `Result<T, E>`. All arms must
return the same type. Patterns must be exhaustive.

### Pattern Kinds

| Implemented pattern                         | Example              |
|---------------------------------------------|----------------------|
| Wildcard                                    | `_`                  |
| Immutable identifier binding                | `value`              |
| Qualified enum variant (unit)               | `Option::None`       |
| Qualified enum variant (tuple payload)      | `Option::Some(value)` |

Tuple-variant payloads may contain only `_` or immutable identifier bindings.
Nested payload destructuring is not implemented. A catchall `_` or immutable
identifier arm must be the final arm because it matches every enum value.

Mutable identifier, literal (integer, boolean, or string), tuple, struct,
enum-struct, or-pattern, unqualified enum-variant, and nested payload patterns
are not implemented. Parsed unsupported forms produce
`error[UnsupportedPattern]`; forms that the parser cannot represent produce
`error[UnexpectedToken]`. Both are compile-time failures and no executable is
produced.

## Method Calls

```vow
v.push(42);
v.len()
s.byte_at(0)
m.contains_key(k)
```

### Vec<T> Methods

| Method         | Signature                        |
|----------------|----------------------------------|
| `Vec::new()`   | `() -> Vec<T>`                   |
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> Vec<T>` for flat scalar `T` |
| `.push(val)`   | `(T) -> ()`                      |
| `.pop()`       | `() -> ()`                       |
| `.len()`       | `() -> i64`                      |
| `.clear()`     | `() -> ()` — frees buffer, resets to empty |
| `.truncate(n)` | `(i64) -> ()` — shrinks to n elements, frees excess memory |
| `v[i]`         | Index read — copies slot value; aliases heap types (panics if out of bounds) |
| `v[i] = val`   | Index write — copies value into slot |

### String Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `String::from(s)`   | `(String) -> String` — mutable copy |
| `String::new()`     | `() -> String`              |
| `String::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> String` |
| `.len()`            | `() -> i64`                 |
| `.byte_at(i)`       | `(i64) -> i64`              |
| `.push_byte(b)`     | `(i64) -> ()`               |
| `.push_str(s)`      | `(String) -> ()`            |
| `.clear()`          | `() -> ()` — frees buffer, resets to empty |
| `.contains(s)`      | `(String) -> bool`          |
| `.eq(s)`            | `(String) -> bool`          |
| `.substring(start, end)` | `(i64, i64) -> String` |
| `.parse_i64()`      | `() -> Option<i64>`         |
| `.parse_u64()`      | `() -> Option<u64>`         |

### HashMap<K, V> Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `HashMap::new()`    | `() -> HashMap<K, V>`       |
| `.insert(k, v)`     | `(K, V) -> ()`              |
| `.get(k)`           | `(K) -> V`                  |
| `.contains_key(k)`  | `(K) -> bool`               |
| `.remove(k)`        | `(K) -> ()`                 |
| `.len()`            | `() -> i64`                 |

### BTreeMap<K, V> Methods

Keys must be `i64` (K violations raise `BTreeMapKeyTypeMustBeI64`). Values may be any
non-linear type — primitives, structs, `Vec<T>`, `Option<T>`, or nested combinations.
A `V` that is or transitively contains a `linear struct` is rejected with
`BTreeMapValueMustBeNonLinear`, because the runtime/verifier shift values bitwise and
would silently duplicate a linear obligation.
Storage is two parallel sorted arrays (binary-search lookup, sorted-insert writes).
Iteration order is ascending by key and is **deterministic across runs and compilers** —
prefer `BTreeMap` over `HashMap` for any map whose iteration affects compiler output.

| Method              | Signature                   |
|---------------------|-----------------------------|
| `BTreeMap::new()`   | `() -> BTreeMap<K, V>`      |
| `.insert(k, v)`     | `(K, V) -> Option<V>` (returns the previous value bound to `k`, if any) |
| `.get(k)`           | `(K) -> Option<V>` (returns the value bound to `k`, or `None`)          |
| `.contains(k)`      | `(K) -> bool`               |
| `.len()`            | `() -> i64`                 |

### Option<T> Methods

| Method      | Signature                              |
|-------------|----------------------------------------|
| `.unwrap()` | `() -> T` (panics on None; requires `[panic]` effect) |

The `?` operator on `Option<T>` or `Result<T, E>` propagates `None`/`Err` to the caller (the calling function must return `Option` or `Result`).

## Indexing

```vow
let val: i64 = v[0];
v[i] = new_val;
```

Indexing uses **copy semantics**: `v[i]` copies the 8-byte slot value and `v[i] = val` copies a value into the slot. The base container is not consumed.

For primitive types (`i64`, `bool`), this is a genuine value copy — the result is independent of the container. For heap types (`Vec<T>`, `String`, structs, enums), the 8-byte slot holds a pointer, so indexing copies the pointer, creating an **alias**. Both the container slot and the local variable point to the same heap data:

```vow
let buckets: Vec<Vec<i64>> = Vec::new();
buckets.push(Vec::new());
let b: Vec<i64> = buckets[0];  // b aliases buckets[0]
b.push(42);                     // visible through buckets[0]
```

This aliasing is the intended behavior for arena and hash-table patterns where bucket contents are read and mutated repeatedly through index access.

## Extern Blocks

Declare external C functions (a `vow` contract block is required):

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

Omitting the `vow` block produces a `MissingContract` error (see [errors.md](errors.md)).

## Type Aliases

```vow
type Score = i64
```

## Effect System

Effects are explicit. Every function declares which side effects it may perform. Pure functions (no effects) need no annotation.

### Effect Types

| Effect   | Meaning                              |
|----------|--------------------------------------|
| `io`     | Standard I/O (print, stdin, network) |
| `read`   | File system reads                    |
| `write`  | File system writes                   |
| `panic`  | May panic (unwrap, etc.)             |
| `unsafe` | Unsafe operations (FFI, raw memory)  |

Each effect is independent — `io` is not a superset of `read` or `write`.

### Propagation

A function must declare every effect that any function it calls may produce:

```vow
fn do_io() -> () [io] {
    print_str("hi");
}

fn caller() -> () [io] {
    do_io();
}
```

If `caller` omitted `[io]`, the type checker would emit `EffectViolation`.

### Contract Purity

Contract expressions (`requires`, `ensures`, `invariant`) must be pure — they cannot call effectful functions.

### Builtin Function Signatures

#### FFI Wrapper Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for flat scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a flat scalar slot type (`i*`, `u*`, `f32`, `f64`, `bool`). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.

`String::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` flat scalar slots into a fresh `Vec<T>`. The surface length type is `i64`; the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

For pointer-containing C payloads, a wrapper must be written per type: call the extern, recursively copy every Vow-owned heap subobject into the target region, free every C-owned pointer according to the extern's ownership contract, then return the Vow-placed value. A bytewise copy of a pointer-containing payload is unsound because it preserves stale pointers into C-owned storage.

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

#### Debug

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `debug_str`      | `fn(s: String) -> ()`                      | `[]`       |
| `debug_i64`      | `fn(v: i64) -> ()`                         | `[]`       |
| `debug_u64`      | `fn(v: u64) -> ()`                         | `[]`       |

**Debug print semantics:** Debug prints are effect-free and callable from pure functions. In debug and sanitize modes (`--mode debug`, `--mode sanitize`), they write to stderr. In release and profile modes, the debug call itself is not emitted — no function call occurs. However, argument expressions are still evaluated (a direct literal such as `"label"` is static, while `String::from("label")` still allocates a mutable copy). They are also no-ops during verification. Use them to trace values inside pure kernel code without restructuring the effect hierarchy.

#### Filesystem

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_open`        | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_read_line`   | `fn(handle: i64) -> String`                | `[read]`   |
| `fs_status`      | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_close`       | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> i64`    | `[write]`  |
| `fs_exists`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_mkdir`       | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_listdir`     | `fn(path: String) -> Vec<String>`          | `[read]`   |
| `fs_remove`      | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir`  | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_is_dir`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_is_symlink`  | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_rename`      | `fn(old: String, new: String) -> i64`      | `[io]`     |

#### String Operations

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `string_substr`       | `fn(s: String, start: i64, len: i64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: i64, literal: String literal) -> i64` | `[]` |
| `string_trim`         | `fn(s: String) -> String`                        | `[]`    |
| `string_to_upper`     | `fn(s: String) -> String`                        | `[]`    |
| `string_to_lower`     | `fn(s: String) -> String`                        | `[]`    |
| `string_replace`      | `fn(s: String, from: String, to: String) -> String` | `[]` |
| `string_join`         | `fn(parts: Vec<String>, sep: String) -> String`  | `[]`    |

#### Conversion

**Formatting** uses two baselines; widen via `as` for narrower types:

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `int_to_string`  | `fn(v: i64) -> String`                     | `[]`       |
| `uint_to_string` | `fn(v: u64) -> String`                     | `[]`       |
| `i64_to_string`  | `fn(v: i64) -> String` (alias of `int_to_string`) | `[]` |

```vow
let small: u8 = 42;
print_str(uint_to_string(small as u64));  // widen then format
```

**Parsing** exposes a try-form for every integer width:

| Function       | Signature                                |
|----------------|------------------------------------------|
| `parse_i8`     | `fn(s: String) -> Option<i8>`            |
| `parse_i16`    | `fn(s: String) -> Option<i16>`           |
| `parse_i32`    | `fn(s: String) -> Option<i32>`           |
| `parse_i64`    | `fn(s: String) -> Option<i64>` (also see `String.parse_i64()`) |
| `parse_i128`   | `fn(s: String) -> Option<i128>`          |
| `parse_u8`     | `fn(s: String) -> Option<u8>`            |
| `parse_u16`    | `fn(s: String) -> Option<u16>`           |
| `parse_u32`    | `fn(s: String) -> Option<u32>`           |
| `parse_u64`    | `fn(s: String) -> Option<u64>` (also see `String.parse_u64()`) |
| `parse_u128`   | `fn(s: String) -> Option<u128>`          |

Each `parse_X` returns `Option::None` for malformed input, empty strings, or
values outside the target type's range.

In particular, `parse_u8` accepts decimal values from `0` through `255` and
returns `Option::None` for negative or larger values.

**Narrowing intrinsics** (per [Type Cast](#type-cast)): for every narrowing
pair the compiler emits `<src>_to_<tgt>_try`, `<src>_to_<tgt>_wrap`, and
`<src>_to_<tgt>_sat` free functions with the semantics described in that
section.

#### Collections

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `vec_sort`       | `fn(v: Vec<i64>) -> Vec<i64>`              | `[]`       |

#### Time

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `time_unix`      | `fn() -> i64`                              | `[io]`     |
| `time_unix_ms`   | `fn() -> i64`                              | `[io]`     |

#### System

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `num_cpus`       | `fn() -> i64`                              | `[io]`     |
| `memory_root_arena_bytes` | `fn() -> u64`                    | `[io]`     |
| `memory_peak_bytes` | `fn() -> u64`                           | `[io]`     |
| `memory_alloc_count_since_start` | `fn() -> u64`              | `[io]`     |

`num_cpus()` returns the number of available logical CPUs (from `std::thread::available_parallelism`), or `1` if the query fails. Used to size worker pools (e.g. the default `--verify-jobs` value).

`memory_root_arena_bytes()` returns the current bytes retained by root-region arena chunks. `memory_peak_bytes()` returns the peak live bytes retained by all open arena chunks since process start. `memory_alloc_count_since_start()` returns the number of successful Vow arena allocation requests since process start. These queries do not allocate; they are effectful because they observe runtime process state.

#### Encoding

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hex_encode`     | `fn(data: Vec<u8>) -> String`              | `[]`       |
| `hex_decode`     | `fn(s: String) -> Vec<u8>`                 | `[]`       |

#### Input

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `stdin_ready`    | `fn() -> bool`                             | `[read]`   |

#### Process Management

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `process_exit`        | `fn(code: i64) -> !`                             | `[io]`  |
| `process_run`         | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_get_stdout`  | `fn() -> String`                                 | `[io]`  |
| `process_get_stderr`  | `fn() -> String`                                 | `[io]`  |
| `process_start`       | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_wait`        | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_wait_timeout`| `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_kill`        | `fn(pid: i64) -> i64`                             | `[io]`  |
| `process_stdout_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_stderr_for`  | `fn(pid: i64) -> String`                         | `[io]`  |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. Callers should check `result.len() == 0` to detect failure.

**Streaming file input:** `fs_open(path)` opens a file for incremental reading and returns a positive handle, or `-1` on path/open error. `fs_read_line(handle)` reads one line from the current cursor and returns it as a String, including the trailing newline when present. It returns `""` at EOF, for an invalid handle, or after a read error. A blank line is returned as `"\n"`, so newline-delimited callers can distinguish a real blank line from EOF by content. After `fs_read_line(handle)` returns `""`, call `fs_status(handle)` to distinguish EOF from error: `0` means the handle is open with no EOF/error state, `1` means EOF, and `-1` means invalid handle or read error. `fs_status(handle)` reports the result of the most recent `fs_read_line(handle)` call on that open handle; read it immediately after a `""` return because later reads may update it. `fs_close(handle)` releases the handle and returns `0` on success or `-1` for an invalid/already-closed handle. Long-running programs must close handles they no longer need. All streaming handle operations use the `[read]` effect, including `fs_close`, because closing a read handle releases read-stream state and does not mutate filesystem contents. The current runtime stores streaming handles in one process-global table, and `fs_read_line` holds that table lock while it reads the next line. This keeps the API simple for single-stream file processing, but it is not intended for latency-sensitive concurrent reads from multiple slow handles.

**Filesystem return values:** `fs_write`, `fs_mkdir`, `fs_remove`, `fs_remove_dir`, and `fs_rename` return `i64`: 0 on success, non-zero on failure. `fs_open`, `fs_status`, and `fs_close` use the streaming status codes above. `fs_exists`, `fs_is_dir`, and `fs_is_symlink` are predicates: they return 1 for true, 0 for false. Errors (null pointer, invalid UTF-8) also return 0, so callers cannot distinguish "false" from "error". `fs_is_symlink` uses `lstat`-equivalent semantics: a symlink reports 1 even when its target is a regular file or directory.

**`string_starts_with` / `string_ends_with` / `string_matches_literal_at` return values:** Return `i64`: 1 if true, 0 if false.

**`string_matches_literal_at` literal operand:** The third argument must be written as a string literal at the call site. The compiler lowers that literal to static bytes plus an explicit byte length, so no temporary `String` allocation is created and embedded NUL bytes are preserved. Passing a variable or computed `String` as the third argument is a type-check error (`StaticLiteralRequired`). Use `string_starts_with`, `string_ends_with`, or `String` methods when the needle must be dynamic.

**`process_run` vs `process_start`:** `process_run(cmd, args)` runs a subprocess synchronously and returns its exit code. After it returns, `process_get_stdout()` and `process_get_stderr()` retrieve the captured output of the most recent `process_run` call. `process_start(cmd, args)` launches a subprocess asynchronously and returns a process ID. Use `process_wait(pid)` to wait for completion and get the exit code, and `process_stdout_for(pid)` / `process_stderr_for(pid)` to retrieve output.

**`process_wait_timeout`:** `process_wait_timeout(pid, timeout_ms)` polls a process started with `process_start` until it exits or the timeout (in milliseconds) elapses. Returns the exit code on completion, `-1` on error, or `-2` on timeout. After a timeout, the process is still running; use `process_kill(pid)` to terminate it.

**`process_kill`:** `process_kill(pid)` sends a kill signal to a running process and waits for it to exit. Returns 0 on success, -1 on error. No-op (returns 0) if the process has already completed.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. The returned String is runtime scratch storage valid until the next `stdin_read_line()` call. Process each line before reading the next one for bounded memory; use `pin_to_root(line)` before the next read when a line must be stored, returned, passed to a function that may store it, mutated, or otherwise retained. The direct scratch line is read-only. The scratch buffer keeps the largest line capacity seen so far, so memory is bounded by maximum line length rather than total input, but one very large line can retain that capacity for the process lifetime.

```vow
let lines: Vec<String> = Vec::new();
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // Without pin_to_root, lines.push(line) would store the scratch alias, not a copy.
    lines.push(pin_to_root(line));
    line = stdin_read_line();
}
```

```vow
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

**`stdin_ready`:** `stdin_ready()` returns `true` if `stdin_read_line()` would return immediately without blocking, `false` otherwise. Uses a non-blocking poll with zero timeout. Use this in computation loops that must remain responsive to external input:

```vow
while !stdin_ready() && depth < max_depth {
    // continue searching
    depth = depth + 1;
}
if stdin_ready() {
    let cmd: String = stdin_read_line();
    // handle command
}
```

## Canonical Form

The canonical printer normalizes source: `parse → print → parse` is idempotent. Effects are sorted alphabetically, indentation uses 4 spaces, trailing expressions omit semicolons.

---

# Vow CLI Reference

## Commands

### `vow build` (default)

Compile source to native executable. Verifies contracts by default.

```
vow build [OPTIONS] <source.vow>
vow [OPTIONS] <source.vow>          # legacy (equivalent)
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `build/<stem>` | Output executable path                  |
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC incremental BMC max iterations          |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |
| `--replay-cex`    | (off)       | Differential test the verifier model against runtime semantics (same as `vow verify --replay-cex`; see below) |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this compilation to `<path>` (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from `--mode profile`, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache. |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever ESBMC verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |
| `--replay-cex`    | (off)       | Differential test of the verifier model against runtime semantics. After ESBMC reports a counterexample, build a `--mode debug` harness that calls the failing function with the counterexample's concrete inputs and check that the runtime `VowViolation` agrees (same `vow_id` and blame). Adds a `replay` field to each counterexample (see "Counterexample replay" below). Opt-in, off by default; also accepted by `vow build`. |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this verification run to `<path>` (load directly at ui.perfetto.dev). Captures frontend phase spans, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Pure side artifact. |

### `vow contracts`

List all contracts (requires, ensures, invariant) in a program. Runs frontend only by default (no codegen, no verification).

```
vow contracts [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--verify`        | (off)       | Run ESBMC verification and report per-contract status |
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver (with --verify)           |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode (with --verify); ir requires z3 |
| `--verify-jobs <N>` | `num_cpus/2` | Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial) |

**Exit code.** With `--verify`, `vow contracts` fails closed exactly like `vow build --verify` and `vow verify`: it exits **1** if any contract's `status` is not proven — i.e. any `failed`, `timeout`, `unknown`, `error`, `skipped`, or `vacuous` — and **0** only when every contract is `proven`/`proven-ir`. Without `--verify` every contract is `not_verified` and the command exits 0. (This is independent of the static `quality` classification, which never affects the exit code.)

When `--verify` is requested but ESBMC is not installed, the command still emits the full contracts-result JSON schema with every entry's `status` set to `error` and exits with code 1 (fail-closed). Install ESBMC, or omit `--verify`, to obtain proven/failed/unknown statuses.

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # print concise Claude Code SKILL.md entrypoint
vow skill print --bundle  # print self-contained bundle for raw API harnesses
vow skill install      # prompt for local or global install target
vow skill install --local   # install to ./.claude/skills/vow/
vow skill install --global  # install to $HOME/.claude/skills/vow/ on Linux
```

`print` writes the concise installed `SKILL.md` entrypoint (with YAML frontmatter) to stdout. `print --bundle` writes a complete self-contained skill document to stdout for non–Claude Code harnesses that cannot load supporting files.

`install` writes `SKILL.md` plus supporting files under `reference/`, `examples/`, and `schemas/`. Claude Code discovers the skill from the `.claude/skills/` directory and uses the frontmatter description/`when_to_use` metadata to load it for `.vow` file work as well as creation and verification-debugging prompts before a `.vow` file exists.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the legacy `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run ESBMC verification on test files       |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). Single-file invocations (`vow test path/to/test_foo.vow`) keep the default behaviour of resolving `use` against the file's parent directory.

**Test Output JSON:**

```json
{
  "status": "TestsPassed",
  "total": 3,
  "passed": 3,
  "failed": 0,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed",
      "exit_code": 0,
      "stdout": "7",
      "stderr": "",
      "duration_ms": 72,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "contract_density": {
    "functions_total": 1,
    "functions_with_vows": 0,
    "density_pct": 0.0
  }
}
```

| Status Field   | Meaning                                           |
|----------------|---------------------------------------------------|
| `TestsPassed`  | All tests passed                                  |
| `TestsFailed`  | One or more tests failed                          |

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `contract_skipped`, `skipped`.

`contract_skipped` means ESBMC was never invoked because a vowed function is non-modelable (distinct from `verify_failed`, where ESBMC proved a violation). Both are fail-closed — a `contract_skipped` test counts toward `failed` and yields a `TestsFailed` overall status.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier2-cmd` | `scripts/full_test.sh` | Full oracle. Only run on Tier-1 survivors. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |

Output schemas: see `docs/spec/schemas/mutants-result.schema.json`.

### `vow complexity`

Report per-function complexity metrics as deterministic, **byte-identical** JSON (the Rust and self-hosted compilers produce identical output). Every structural metric sits next to its size; the single 0–100 `complexity_score` is a readability / refactor-priority gate — explicitly **not** a defect predictor. The component vector, not the scalar, is the source of truth; gating on the scalar alone is opt-in and discouraged as the sole signal.

```
vow complexity <source.vow>
               [--cog-anchor N] [--nloc-anchor N]
               [--max-score N] [--max-cognitive N] [--max-cyclomatic N]
```

| Flag | Default | Notes |
|---|---|---|
| `--cog-anchor <N>` | `15` | Cognitive-complexity value mapped to sub-score `0.800` (SonarQube's default flag line). |
| `--nloc-anchor <N>` | `60` | NLOC value mapped to sub-score `0.800` (~50–60 line guidance). |
| `--max-score <N>` | (unset) | CI gate: exit nonzero if any function's `complexity_score` exceeds N. The recommended line is 80, but gating is opt-in only. |
| `--max-cognitive <N>` | (unset) | CI gate: exit nonzero if any function's `cognitive` exceeds N. |
| `--max-cyclomatic <N>` | (unset) | CI gate: exit nonzero if any function's `cyclomatic` exceeds N. |

**Exit code.** Nonzero on frontend/read failures, malformed numeric flags, or when a `--max-*` threshold is passed and exceeded. With no `--max-*` flag the command is pure reporting once the input is readable and valid — no threshold gates by default (per the decouple-language-from-prover principle).

**Numeric convention.** The non-integer metrics (`halstead.volume`/`difficulty`/`effort` and `score_factors.*`) are emitted as fixed-3-decimal JSON numbers computed in **integer fixed-point** (scale 1000) — never native floats — so both compilers stay byte-identical. `complexity_score` is an integer in `[0, 100]`. The score's saturating anchor map uses a rational curve (`0.800` at the anchor, asymptoting to `1.000`), not an exponential, because the self-hosted compiler has no floating point.

**Contract identifier convention.** `vow.contract.free_vars` counts distinct value identifiers referenced by clause predicates. It excludes the `result` binding, function callee identifiers, and method names; receiver and argument expressions still count when they are values.

Output schema: see `docs/spec/schemas/complexity-result.schema.json`.

### `vow --help`

`vow --help` is agent-first. It emits versioned JSON capability data for the tool, command set,
language surface, result schemas, and implementation status. `--help --human` exists only as a
legacy compatibility mode and is not the canonical interface.

```
vow --help               # versioned JSON tool-help protocol
vow --help --human       # legacy compatibility text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same legacy text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                                                                            |
|------|------------------------------------------------------------------------------------|
| `0`  | Success (`Verified` or `Unverified`)                                               |
| `1`  | Failure (`CompileFailed`, `VerifyFailed`, or `Skipped`)                            |

`vow build` and `vow verify` both fail closed on `Skipped`: if ESBMC was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

The table above is the exit status of the `vowc` **compiler**. A **compiled Vow program** exits
with whatever its `main` returns, with one reserved exception: any runtime abort — out-of-memory,
contract violation, arithmetic overflow, unwrap-on-`None`, index-out-of-bounds, region-literal
mutation, stack overflow, or a sanitizer trap — terminates with the reserved status **`134`**. By
convention `134` is reserved for aborts: it is never produced *spontaneously* by a normal `main`
return, so a program that does not itself return or `process_exit(134)` can treat any `134` as a
runtime abort rather than an application result. The reservation is a convention, not enforced — a
program that deliberately exits `134` opts out. See the *Exit status* note under Runtime Errors in
[`errors.md`](errors.md) for the full list and rationale.

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by ESBMC. |
| `Unverified`    | Compiled but ESBMC was not invoked (e.g. `--no-verify`, `--dump-ir`). Exit 0. |
| `Skipped`       | ESBMC was invoked but at least one vowed function could not be modelled (e.g. body uses `Linear*`, `Load`/`Store`, `RemF*`, or has effects). Struct construction (`RegionAlloc`) and field reads/writes (`FieldGet`/`FieldSet`) **are** modelled via the user-struct heap model. Each skipped function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`. Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, link failure, or a diagnostic-emission I/O failure (e.g. a broken stderr/stdout pipe other than the tolerated case, or a full disk) |
| `VerifyFailed`  | ESBMC produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`), tool error, the tool was not found, or the verifier worker thread crashed (`verify_status: "panicked"`). Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

### Verified Example

```json
{
  "status": "Verified",
  "executable": "examples/divide",
  "diagnostics": [],
  "counterexamples": []
}
```

### CompileFailed Example

```json
{
  "status": "CompileFailed",
  "executable": null,
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "function body has type `bool` but declared return type is `i32`",
      "severity": "error",
      "span": {
        "file": "bad.vow",
        "offset": 25,
        "length": 8
      }
    }
  ],
  "message": "type error",
  "counterexamples": []
}
```

### VerifyFailed Example

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "values": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      },
      "blame": "callee"
    }
  ]
}
```

For caller-blame failures where a verified function violates a callee's
`requires` clause, the counterexample reports the callee clause in `violation`
and `vow_id`, and includes the caller expression in `call_sites`. When the
callee precondition binds a parameter, `violating_args` names the callee
parameter, the counterexample value when available, and the caller argument span.
If `violating_args[].value` is `""`, Vow could not statically recover the
caller argument value; `arg_offset` and `arg_length` still identify the
argument expression.

### Fields Reference

| Field              | Type                | When Present      | Description                               |
|--------------------|---------------------|-------------------|-------------------------------------------|
| `status`           | string              | Always            | One of the four status values             |
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure or library module (no main) |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Error category ("parse error", "type error", "module load error", link error detail, or "failed to emit frontend diagnostics: {io_error}") |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string                 |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema)   |
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, `"tool_not_found"`, or `"panicked"` (verifier worker thread crashed — no counterexample available) |
| `verify_message`   | string              | On backend failure | ESBMC/backend error detail                |

### Counterexample replay

With `--replay-cex`, each object in `counterexamples[]` gains a `replay` field — a **differential test** of the verifier's IR-to-C model against the executable's debug-mode runtime semantics. It is **not part of the soundness proof**: it neither strengthens nor weakens the static verdict, and the exit code is unchanged. It exists to catch *drift* between the two independent lowerings — `vow-verify`'s C emitter (`requires` → `__ESBMC_assume`, `ensures`/`invariant` → `__ESBMC_assert`) and `vow-codegen`'s debug runtime checks.

For each counterexample, Vow maps the ESBMC assignment back to concrete Vow inputs, synthesizes a `--mode debug` harness that calls the failing function with those inputs, runs it, and compares the observed `VowViolation`.

| `replay` value | Meaning |
|----------------|---------|
| `"confirmed"`  | The harness fired `VowViolation` with the **same `vow_id` and the same blame** the counterexample predicted. High-confidence: the model agrees with runtime. |
| `"diverged"`   | The harness exited cleanly, or fired a *different* `vow_id`/blame. Either the verifier C model is wrong (a model false-positive) or the counterexample values do not reach the violation in real execution. `replay_reason` explains which. |
| `"skipped"`    | Replay was not attempted (e.g. an input type outside v1 scope, a Unit/aggregate parameter, the function is not defined in the entry file, or harness compilation failed). `replay_reason` gives the cause. |

**v1 input scope.** Reconstruction supports scalar parameters (`i64`, `u64`, `bool`) and bounded `Vec` of those scalars. `String`, `HashMap`, `BTreeMap`, struct, reference, and nested-aggregate parameters are reported as `"skipped"` with a reason. The self-hosted compiler's v1 reconstructs scalars only and reports `Vec` parameters as `"skipped"` (the Rust compiler additionally reconstructs bounded `Vec`s); both report identical outcomes for scalar and aggregate-skip cases. Replaying a counterexample for a function whose entry file already defines `main` is `"skipped"` by the self-hosted compiler.

`replay`/`replay_reason` are present on a counterexample only when `--replay-cex` was passed.

## Contracts Output JSON

`vow contracts` emits a single JSON object to stdout. Schema: [`schemas/contracts-result.schema.json`](schemas/contracts-result.schema.json).

### Example (without --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "not_verified",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Example (with --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "proven",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Contract Fields

| Field         | Type    | Description                                              |
|---------------|---------|----------------------------------------------------------|
| `vow_id`      | integer | Unique contract identifier within the program            |
| `function`    | string  | Function containing this contract                        |
| `kind`        | string  | `"requires"`, `"ensures"`, or `"invariant"`              |
| `description` | string  | Full contract text                                       |
| `blame`       | string  | `"Caller"` (requires) or `"Callee"` (ensures/invariant)  |
| `source`      | object  | `{ "file": string, "offset": integer }`                  |
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, `"skipped"`, or `"vacuous"` |
| `quality`     | string  | Static clause-shape classification (no ESBMC): `"weak"`, `"tautological"`, or `"substantive"` |
| `trivially_satisfiable` | bool | `--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational — never affects the exit code. See `docs/spec/contracts-methodology.md`. |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs (bit-vector encoding, overflow modeled) |
| `proven-ir`     | ESBMC proved this contract under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | ESBMC could not conclude for this contract — either `VERIFICATION UNKNOWN` was reported for the containing function (the incremental-BMC forward condition was unable to prove or falsify), or the function's verification failed overall and ESBMC's per-clause `--multi-property` run returned no individual verdict for this clause |
| `timeout`       | ESBMC timed out on the containing function (BV and — when applicable — IR fallback both timed out) |
| `error`         | ESBMC error or tool not found                        |
| `skipped`       | The containing function's body uses opcodes the verifier cannot model (e.g. `Load`/`Store`, `Linear*` consume/borrow, `RemF*`) or the function has effects. (Struct construction and field ops are modelled — see the `Skipped` build-status row.) Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke ESBMC at all. |
| `vacuous`       | The containing function's `requires` clauses are contradictory, so every `ensures` is satisfied vacuously — ESBMC proved nothing of substance (antecedent failure). Detected by a second ESBMC run with `--error-label`: a `vow_reach` label planted after the `requires` assumes is unreachable. All of the function's clauses are reported `vacuous` (fail-closed, exit 1). See `docs/spec/contracts-methodology.md`. |

The `proven` / `proven-ir` split and the rule that a resource-limited retry (e.g. the BV→IR fallback) may never report a weakened check as `proven` are the verifier's soundness discipline — the safe-vs-unsafe retry rules are specified in `docs/verifier-discipline.md`.

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without ESBMC and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

## Trace Output (stderr, --debug-trace)

When `--debug-trace=calls` or `--debug-trace=full` is used, the compiled binary emits JSON lines to stderr:

### calls mode
```json
{"event":"enter","fn":"main"}
{"event":"enter","fn":"divide"}
{"event":"exit","fn":"divide"}
{"event":"exit","fn":"main"}
```

### full mode (adds vow check results)
```json
{"event":"enter","fn":"divide"}
{"event":"vow","fn":"divide","vow_id":0,"passed":true}
{"event":"exit","fn":"divide"}
```

## Profile Output (stderr, profile mode)

When `--mode profile` is used, the compiled binary prints a call-count report to stderr on normal exit (via `atexit`). The report is not printed if the program is killed by a signal or calls `abort()`.

```
--- vow profile report ---
function                                        calls       %
-------------------------------------------------------------
infer                                         4812399   48.2%
is_def_eq_core                                3201882   32.1%
whnf                                           984201    9.9%
main                                                1    0.0%
-------------------------------------------------------------
total calls: 9998483, unique functions: 12
```

The report lists the top 20 most-called functions sorted by call count. No vow checks are emitted in profile mode.

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

### VowViolation

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

Schema: [`schemas/vow-violation.schema.json`](schemas/vow-violation.schema.json).

### ArithmeticOverflow

```json
{"error":"ArithmeticOverflow"}
```

Emitted when a checked arithmetic operator (`+!`, `-!`, etc.) overflows at runtime.

### UnwrapOnNone

```json
{"error":"UnwrapOnNone"}
```

Emitted when `.unwrap()` is called on `Option::None`.

### IndexOutOfBounds

```json
{"error":"IndexOutOfBounds"}
```

Emitted when a `Vec` index is out of bounds.

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec operation is attempted on a Vec that has already been freed.

### DoubleFree (sanitize mode only)

```json
{"error":"DoubleFree","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec is freed twice.

### StaleIndex (sanitize mode only)

```json
{"error":"StaleIndex","index":5,"expected_gen":3,"actual_gen":7,"vec":"0x55a1b2c3d4e0"}
```

Emitted when `__vow_sanitize_check_generation` detects that a Vec slot's generation counter does not match the expected value, indicating the slot was overwritten since the index was recorded.

## Agent Decision Tree

```
Parse JSON from stdout
├── status == "Verified"       → Success. Binary at `executable`.
├── status == "Unverified"     → Compiled but unverified. ESBMC missing or --no-verify.
├── status == "CompileFailed"  → Read `diagnostics[]` for error details.
│   ├── error_code is parse error  → Fix syntax (see grammar.md)
│   └── error_code is type error   → Fix types (see errors.md)
└── status == "VerifyFailed"   → Read `counterexamples[]`.
    ├── Check `inputs` for the violating values
    ├── Check `violation` for which contract failed
    ├── Check `source` for the location
    └── Fix the contract or the implementation, then rebuild
```

Always check stderr for human-readable diagnostics alongside the JSON on stdout.

---

# Contract Authoring and Verification

Vow uses ESBMC (bounded model checker) for static contract verification. This document covers contract patterns, verification behavior, and common pitfalls.

## Verification Pipeline

Codegen (Cranelift) and verification run in parallel:

```
Vow Source → Parse → Type Check → IR Lower ─┬─→ Cranelift → executable
                                              └─→ C Emit → ESBMC → proof / counterexample
```

Contract clauses become IR opcodes. The C emitter translates `requires` to `__ESBMC_assume()` (the verifier assumes preconditions hold) and `ensures`/`invariant` to `__ESBMC_assert()` (the verifier checks postconditions).

### ESBMC Configuration

- Verification strategy: **incremental BMC** (`--incremental-bmc`) — base case plus forward condition, **not** k-induction (there is no inductive step). A contract is `proven` only when ESBMC's forward condition establishes completeness within the bound; otherwise the result is `unknown`, never a false `proven`
- Incremental BMC with `--max-k-step` (default: **50**) — loops are verified incrementally up to N iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC is a *bounded* model checker, so it models collection types as
fixed-size arrays and reasons about them up to a finite capacity. These
capacities are an internal property of the verifier, not of the language:

| Type              | Model Capacity | Supported Operations |
|-------------------|----------------|----------------------------------------------|
| `Vec<T>`          | 128            | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256            | `from`, `len`, `push_byte`, `push_str`, `byte_at`, `matches_literal_at` |
| `HashMap<K, V>`   | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| `BTreeMap<K, V>`  | 64             | `new`, `insert`, `get`, `contains_key`, `len`|

**These bounds are not a language feature and are not user-tunable.** A `Vec`
in a Vow program grows dynamically on the heap with no fixed maximum; the
capacity above only describes how far the *bounded* model checker reasons. The
language and its contracts are deliberately decoupled from what any particular
prover can prove: replace ESBMC with a stronger (or unbounded) checker and the
same source, the same contracts, and the same CLI keep working — the only
difference is that proof covers more (or all) of the state space. For this
reason a `requires`/`ensures` clause must never encode a verifier bound (e.g.
`requires: v.len() <= 128`); see "Verification-Driven Bounds (Anti-Pattern)"
below and `docs/design/verifier-model-bounds.md`.

These models support the same operations as the runtime but with bounded
storage. String literals carry their concrete length and bytes in verification,
and `String::from` copies that model from its source value. The effective string
model capacity is automatically at least the longest static string literal, so
literal byte initializers always fit the model array. Operations whose bytes are
not statically known, such as `String::from_cstr`, produce a nondeterministic
length (0 to max-1). `string_matches_literal_at` is modeled against the
literal's concrete bytes and byte length; the third argument must be a string
literal so the verifier never has to infer static text from a dynamic `String`.

## Blame Model

| Clause      | Blame  | Who is at fault                                    |
|-------------|--------|----------------------------------------------------|
| `requires`  | Caller | The caller passed invalid arguments                |
| `ensures`   | Callee | The function body doesn't satisfy the postcondition|
| `invariant` | Callee | The loop body breaks the invariant                 |

## Counterexample Replay (Differential Test)

`vow verify --replay-cex` (also `vow build --replay-cex`) cross-checks a counterexample against the executable's runtime semantics. After ESBMC reports a violation, Vow maps the symbolic assignment to concrete Vow inputs, builds a `--mode debug` harness that calls the failing function with them, and checks whether the runtime `VowViolation` matches — **same `vow_id` and same blame**.

This is a *differential test*, **not part of the proof**. The static verdict and exit code are unchanged whether or not replay is requested. Its purpose is to detect drift between the two independent lowerings of a contract: the verifier's C model (`requires` → `__ESBMC_assume`, `ensures`/`invariant` → `__ESBMC_assert`) and `vow-codegen`'s debug-mode runtime checks. A `confirmed` replay grounds the counterexample in real execution; a `diverged` replay flags either a model false-positive or values that do not reach the violation at runtime. See `docs/spec/cli.md` → "Counterexample replay" for the JSON shape and v1 input scope.

## Integer Contracts

### Non-zero Guard

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

### Range Bounds

Use range bounds only when they reflect genuine semantic constraints (e.g., overflow prevention), not to appease the verifier:

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

The bounds here prevent `a + b` from overflowing `i64` — a legitimate semantic concern, not a verifier limitation.

### Equality Postcondition

```vow
fn twice(x: i64) -> i64 vow {
    ensures: result == x + x
} {
    x + x
}
```

### Negation

```vow
fn negate(x: i64) -> i64 vow {
    ensures: result + x == 0
} {
    0 - x
}
```

**Warning:** Fails for `x = -9223372036854775808` (i64 min) due to wrapping overflow. Add `requires: x > -9223372036854775808` if needed.

## Vec Contracts

### Bounds Check

```vow
fn get_element(v: Vec<i64>, i: i64) -> i64 vow {
    requires: i >= 0,
    requires: i < v.len()
} {
    v[i]
}
```

### Fill Pattern with Loop Invariant

See the worked CEGIS example in [examples.md](examples.md#3-vec-fill--loop-invariant).

## String Contracts

### Non-empty String

```vow
fn make_greeting() -> String vow {
    ensures: result.len() > 0
} {
    let s: String = String::from("");
    s.push_byte(72);
    s
}
```

## HashMap Contracts

### Contains Key After Insert

```vow
fn insert_and_check() -> HashMap<i64, i64> vow {
    ensures: result.contains_key(42)
} {
    let m: HashMap<i64, i64> = HashMap::new();
    m.insert(42, 100);
    m
}
```

## Loop Invariants

### Counter Bounds

The most common loop invariant pattern bounds the loop counter:

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    i = i + 1;
}
```

### Search Range

```vow
fn bisect(lo: i64, hi: i64) -> i64 vow {
    requires: hi >= lo
} {
    let mut lo: i64 = lo;
    let hi: i64 = hi;
    while lo + 1 < hi vow {
        invariant: hi - lo >= 0
    } {
        let mid: i64 = lo + (hi - lo) / 2;
        lo = mid;
    }
    lo
}
```

## Where Clause Patterns

Where clauses on parameters become refinement types (additional `requires` for verification):

```vow
fn bounded_add(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

Each `where` clause can only reference its own parameter.

## Anti-Patterns

### Tautological Contracts

A contract must constrain behavior the implementation could get wrong. A clause provable from the return type alone, or from a constant/literal body, verifies nothing.

```vow
fn IOP_CONST() -> i64 vow { ensures: result >= 0 } { 0 }
fn sentinel() -> i64 vow { ensures: result == -1 } { -1 }
```

The first is trivially true of the literal `0`; the second restates the body verbatim. Both prove nothing and only enlarge the proof surface.

**Fix:** delete the `vow` block. A postcondition earns its place only when it pins a property of a **computed** result — one that depends on the inputs or control flow and that a wrong implementation would violate (`ensures: result > 0` on a loop-computed `gcd`; `ensures: result == 0 || result == 1` on a branch-computed flag). Named-constant accessors and enum-tag functions returning a literal must carry no contract.

**Crisp rule:** if the clause is true without reading past the signature and a constant body, it is a non-contract — remove it. This is distinct from weakening a real contract (forbidden, see CLAUDE.md "Contract Authoring"): a tautology was never a contract, so deleting it loses no verification value.

### Over-Specifying

```vow
fn add(x: i64, y: i64) -> i64 vow {
    ensures: result == x + y
} {
    x + y
}
```

Fails when `x + y` overflows. The contract mirrors the implementation exactly — it verifies nothing useful and breaks on edge cases.

**Fix:** Add bounds (`requires: x >= 0, ...`) or verify a weaker property.

### Wrapping Arithmetic Overflow

Default arithmetic (`+`, `-`, `*`) wraps on overflow. Contracts that assume no overflow will be violated:

```vow
fn double(x: i64) -> i64 vow {
    ensures: result > x
} {
    x + x
}
```

ESBMC finds: `x = 4611686018427387904` → `result = -9223372036854775808` (wraps negative).

**Fix:** Bound the input or use checked arithmetic (`+!`).

### Non-Inductive Loop Invariant

An invariant must hold at the **start** of every iteration, not just at the end:

```vow
while i < n vow {
    invariant: v.len() == n
} { ... }
```

This is not inductive — `v.len() == n` is only true after the loop.

**Fix:** Use `invariant: i >= 0, invariant: i <= n`.

### Unbound Loop Iterations

Without a bound on loop iterations, ESBMC may timeout (default max-k-step is 50):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

ESBMC will only verify this for small `n` values. **Do not** add `requires: n <= 8` to the contract — that would distort the semantic specification. The contract is correct as-is; ESBMC's bounded verification provides partial assurance.

### Verification-Driven Bounds (Anti-Pattern)

**Never** add artificial bounds to contracts solely to help ESBMC verify them:

```vow
// WRONG: bounds exist only to appease the verifier
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    requires: a <= 15,   // <-- verifier artifact, not semantic
    requires: b <= 15,   // <-- verifier artifact, not semantic
    ensures: result > 0
} { ... }
```

```vow
// CORRECT: only genuine semantic constraints
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    ensures: result > 0
} { ... }
```

Contracts express what is mathematically required for correctness. ESBMC verifies within its capabilities (bounded loops, bounded arithmetic, bounded collection models) — if it cannot fully prove a correct contract, that is acceptable. Partial verification is better than a distorted specification. The same rule is why the verifier's collection model capacities (see "Collection Models for Verification") are internal defaults rather than CLI flags or contract clauses: a bound that belongs to the prover must never leak into the language.

## Interpreting Counterexamples

A counterexample in the JSON output:

```json
{
  "function": "safe_sub",
  "values": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1,
  "source": { "file": "cegis_broken.vow", "offset": 76, "length": 20 },
  "blame": "callee"
}
```

| Field       | Meaning                                                        |
|-------------|----------------------------------------------------------------|
| `function`  | Which function's verification query failed                     |
| `values`    | Source or ESBMC variable values in the counterexample           |
| `violation` | Which contract clause was violated                             |
| `vow_id`    | Function-local ID linking to the specific vow clause            |
| `source`    | Byte offset in the source file of the violated clause           |
| `blame`     | Whether the caller, callee, or neither party is responsible     |

When caller code violates a callee's `requires` clause, `violation` and
`vow_id` identify the callee clause. `call_sites` points back to the caller
expression, and `violating_args` identifies the callee parameter and caller
argument span when Vow can recover it. If `violating_args[].value` is `""`,
Vow could not statically recover the caller argument value; `arg_offset` and
`arg_length` still identify the argument expression.

Variable names prefixed with `_esbmc_` are ESBMC internal variables; named inputs map directly to function parameters.

## Unsigned Integer Contracts

The `u64` type works naturally in contracts. Use `as u64` to cast literal values in contract expressions:

```vow
fn safe_add(a: u64, b: u64) -> u64
vow {
    requires: a <= 1000 as u64
    requires: b <= 1000 as u64
    ensures: result >= a
    ensures: result >= b
}
{
    a + b
}
```

ESBMC verifies `u64` contracts using `uint64_t` and unsigned nondet values.

## Extern Block Contracts

Every `extern "C"` block **must** include a `vow { ... }` contract specifying the expected behavior of foreign functions. Omitting the contract is a `MissingContract` error.

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

The contract applies to all functions declared in the block. ESBMC uses `requires` as assumptions and `ensures` as assertions when verifying callers of extern functions.

---

# Contract Methodology: What to Verify

This document answers a question that `contracts.md` does not: given a function,
**which** properties are worth proving, how do you tell a strong contract from a
hollow one, and how do you express the strong shapes within ESBMC's reach.

`contracts.md` is the *reference* (syntax, blame, the verification pipeline,
anti-patterns). This is the *methodology* (judgement). Read that first.

## The core principle: strength, not volume

A proven contract is worth nothing if it would also hold for an incorrect
implementation. The number of contracts a codebase proves is not a quality
signal — the *discriminating power* of each contract is.

This is measurable. Polikarpova, Furia, Pei, Wei, and Meyer (the originator of
Design by Contract), *"What Good Are Strong Specifications?"* (ICSE 2013), found
that testing implementations against **strong** specifications — comprehensive
functional pre/post/invariants — detected roughly **twice as many bugs** as
testing against standard/weak contracts. Their conclusion: *"the quality of
specifications limits the value of verification."*

A concrete Vow example of the trap. Many tag constants in the self-hosted
compiler carry this contract:

```vow
fn EFF_IO() -> i64 vow { ensures: result > 0 } { 1 }
```

ESBMC proves `1 > 0` in milliseconds. But `ensures: result > 0` also holds for
`{ 2 }`, `{ 99 }`, and every other positive constant — it does not pin the value
this function is *supposed* to return. It is a real postcondition, but a **weak**
one: it constrains the output to a half-line, not to a point. Proving 354 of
these does not make the compiler more correct; it makes the verification report
longer.

The fix is not "delete contracts" — it is "make each contract say something only
the correct implementation satisfies."

## A taxonomy of contract shapes

Each shape below lists its *intent*, *when it applies*, a *real Vow example*, and
*strength notes*. The expressibility/verifiability status of every shape is
collected in the matrix at the end.

### 1. Domain precondition (range / validity bound)

**Intent:** restrict the inputs the function promises to handle correctly. Blame
falls on the caller (`requires`).

**When:** the function is only correct on a subset of its parameter types — a byte
in `0..=255`, a non-zero divisor, an in-bounds index.

```vow
fn write_u8(out: Vec<i64>, v: i64) vow {
    requires: v >= 0,
    requires: v <= 255
} { out.push(v); }
```

**Strength:** a precondition is strong when it is the *true* domain of the
function — no wider (which would admit miscompilation) and no narrower (a
verifier-driven bound like `requires: n <= 8`, forbidden by `contracts.md`).
A bounds-check precondition such as `requires: i >= 0, requires: i < v.len()` is
the standard guard for every indexing operation.

### 2. Output-range postcondition (the weak default — use sparingly)

**Intent:** constrain the result to a range.

**When:** the range *is* the full functional spec — e.g. a function whose only
guarantee is non-negativity. This is rare. Most uses are the weak trap above.

```vow
fn item_kind(v: i64) -> i64 vow {
    requires: v >= 0,
    ensures: result >= 0          // weak: any non-negative value satisfies this
} { v / 4294967296 }
```

**Strength:** weak by default. Reach for shape 3, 4, or 5 instead whenever the
function actually computes a *specific* value. If you find yourself writing
`ensures: result >= 0` on a function that returns a computed quantity, ask what
the result *equals* or *inverts*, and assert that.

### 3. Exact functional postcondition (equality)

**Intent:** pin the result to the value the function is defined to produce.

**When:** the output is a closed-form function of the inputs (arithmetic,
bit-packing, encodings).

```vow
fn region_pack(kind: i64, val: i64) -> i64 vow {
    requires: kind >= 0,
    requires: kind <= 3,
    requires: val >= 0,
    requires: val <= 4294967295,
    ensures: result == val * 4 + kind     // exact: only the right answer passes
} { val * 4 + kind }
```

**Strength:** strong — a wrong shift or offset is caught immediately. Note the
preconditions are not verifier appeasement: they bound `val` and `kind` so the
packed result cannot overflow `i64` (a genuine semantic constraint). Contrast
`region_pack` (exact) with `item_pack`/`item_kind` (shape 2, only `>= 0`): the
same bit-packing pattern, one specified strongly and one weakly.

### 4. Round-trip / inverse

**Intent:** prove that an encode/decode (or pack/unpack, serialize/deserialize)
pair compose to the identity on the valid domain.

**When:** two functions are defined as inverses — `pack`/`unpack`,
`encode`/`decode`, `to_bytes`/`from_bytes`.

Specify each direction with an exact closed-form postcondition — shape 3 applied
to the extractor as well as the encoder, so the decoder is pinned to the exact
arithmetic that inverts the pack:

```vow
fn region_kind(r: i64) -> i64 vow {
    requires: r >= 0,
    ensures: result == r - (r / 4) * 4,   // exact extractor
    ensures: result <= 3
} { r - (r / 4) * 4 }
```

Because both directions are pinned to closed forms — `region_pack`'s exact
`ensures: result == val * 4 + kind` (shape 3 above) and the matching
`region_kind`/`region_val` extractors — a `region_pack` then
`region_kind`/`region_val` round-trip recovers `(kind, val)` exactly, and ESBMC
discharges that composition with no separate assertion. The inverse can also be
asserted directly: Vow allows pure-function calls in postconditions, so an
`ensures: region_kind(result) == kind` on `region_pack` is expressible and
modelable when the partner is pure (matrix shape 4). **Strength:** very strong —
round-trip is the property a serialization layer must have, and it catches the
entire class of "encoder and decoder drifted apart" bugs that output-range
contracts miss completely.

### 5. Dispatch totality (fail-closed decoders)

**Intent:** prove that a decoder/dispatcher maps **every** valid input to a
defined output and **never** silently falls through to a default.

**When:** a function switches over a tag/opcode/discriminator. This is the
single highest-value shape for Vow, because silent-fallback normalization
(mapping an unknown tag to a valid-looking default) is the bug class issue #81
was filed over.

The pattern has two halves — a validity precondition and an explicit error
sentinel for the unreachable tail:

```vow
fn is_valid_binop(op: i64) -> bool { op >= 0 && op <= 22 }

fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op)
} {
    if op == BINOP_ADD() { return ...; }
    // ... one arm per valid op ...
    -1                                    // unreachable under the precondition
}
```

**Strength — and a live hardening gap.** The precondition pins the domain, but
this contract does **not yet prove totality**: nothing asserts the function never
returns the `-1` sentinel. The strong form adds a postcondition that rules out
the fallthrough:

```vow
fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op),
    ensures: result != -1                 // proves every valid op is handled
} { ... }
```

With `ensures: result != -1`, ESBMC must show that on every `op` in `0..=22` some
arm returns before the sentinel — i.e. the dispatch is exhaustive. If an agent
later adds opcode 23 to `is_valid_binop` but forgets the matching arm,
verification fails instead of miscompiling. This is the contract that converts a
silent fallback into a caught error.

The static classifier rates this clause `substantive`, and `vow contracts --verify`'s
body-replace probe reports it `trivially_satisfiable: true` — both are correct, because
they measure different things. The probe replaces the body with `return 0` (the `i64`
default); `0 != -1` holds, so by the definition in **Weakness** (below) this is a *true*
positive: `ensures: result != -1` does not constrain the op→opcode *mapping* — a constant
non-sentinel body (`return 5`) satisfies it for every valid `op`. What the clause *does*
prove is dispatch **totality**: every valid `op` reaches an arm before the `-1` fallthrough
(delete an arm and verification fails). Totality is the silent-fallback property #81
targets, and — absent a quantifier to say "result is the correct opcode for `op`" — it is
the strongest property a `!= sentinel` postcondition can express. So read the
`trivially_satisfiable: true` as accurate (the clause pins totality, not the mapping), not
as a probe artifact to dismiss. This is *not* the constant-result false positive noted in
**Weakness**: `binop_opcode`'s correct result varies per `op`, so it is not genuinely the
type default.

> Vow has no surface quantifier (`forall i in 0..n`) today, so "covers all valid
> inputs" is expressed as `requires` (pin the finite domain) + a postcondition
> that excludes the failure value, letting ESBMC enumerate the finite branch
> structure. Bounded quantifiers are tracked as a roadmap item (#467/#470).

### 6. Relational / cross-function (uniqueness, agreement)

**Intent:** state a property that spans more than one function or more than one
argument.

**When:** tags in a family must be distinct; two collections must have equal
length; a result must relate two inputs.

The argument-relational form is directly expressible:

```vow
fn build_pairs(ids: Vec<i64>, names: Vec<i64>) -> Vec<Pair> vow {
    requires: ids.len() == names.len()
} { ... }
```

The *cross-function uniqueness* form — "`tok_kw_fn() != tok_kw_let()` for every
pair in the family" — is expressible only as O(n²) pairwise inequalities, which
does not scale to dozens of tag constants. **The better fix is structural, not
contractual:** encode each family as a base+offset range or a generated table so
uniqueness is a property of the *encoding* rather than something every function
must restate. Treat a wall of zero-argument tag constants as an API smell that
shape-6 contracts cannot economically repair.

### 7. Loop invariant / frame

Covered in `contracts.md` (counter bounds, search-range invariants, the
inductiveness requirement). The methodology point: an invariant is strong when it
is the property the loop *maintains toward its postcondition*, not merely
`i >= 0`. See `contracts.md` §Loop Invariants and the worked CEGIS cycle in
`examples.md`.

## Hollow contracts: three failure modes to detect

A contract can pass verification while proving nothing. There are three distinct
ways this happens; a contract-quality tool should distinguish them.

### Weakness

The clause is satisfiable and true, but so loose that an incorrect
implementation also satisfies it (`ensures: result >= 0` on a computed value).
This is the 354-contract problem.

**Detection (the body-replace probe).** `vow contracts --verify` ships this check
(#81). It mutates the implementation in the strongest possible way — replaces the
whole body with a trivial `return <type-default>` — and re-verifies the
`ensures`. If the contract still proves against that body, a constant-returning
implementation satisfies it, so it does not constrain the real computation: each
such `ensures` is reported `trivially_satisfiable: true`. This is exactly the
`body-replace` mutation of `vowc mutants` with ESBMC as the oracle.

The signal is **one-sided (sound, not complete)**: a `true` verdict is a proof of
weakness (a specific trivial body satisfies the contract), but a `false` verdict
does not prove strength — the probe uses a single default value and skips
non-scalar returns, returned parameters, and φ-merged/branchy results, so it can
miss weak contracts it cannot witness this way. It is informational and never
changes the exit code; pair it with the static `quality` field. The one known
false positive is a function whose correct result genuinely *is* the type default
(e.g. a constant `ensures result == 0` on a `{ 0 }` body) — an equivalent mutant,
the standard caveat of mutation testing.

### Tautology

The clause is true independent of the program — `ensures: true`,
`ensures: result == result`, `ensures: x >= 0 || x < 0`. **Detection:** the clause
is valid (provable) with the function body removed; a cheap check folds constant
clauses and flags any clause with no dependence on parameters or `result`.

### Vacuity (antecedent failure)

The clause is proved only because its **preconditions are unsatisfiable**, so the
path it guards is dead and the postcondition never has to hold. Because Vow
lowers `requires` to `__ESBMC_assume`, a contradictory or over-strong precondition
makes *any* `ensures` provable — an assume-false / dead-path proof.

This is the classic vacuity of Beer, Ben-David, Eisner, and Rodeh, *"Efficient
Detection of Vacuity in Temporal Model Checking"* (Formal Methods in System
Design, 2001): a subformula is vacuous when replacing it changes nothing about
the result. Their industrial data is the reason to take it seriously — across
years of hardware verification at IBM, ~20% of formulas were trivially valid on
first runs, and trivial validity *always* indicated a real defect in the design,
spec, or environment.

**Detection (the reachability probe).** `vow contracts --verify` ships this check
(#81). For any function carrying a `requires`, it re-runs ESBMC over the same
model with a `vow_reach` label planted immediately after the `requires` assumes,
under `--error-label vow_reach`. If ESBMC reports the label **unreachable**
(`VERIFICATION SUCCESSFUL`), the conjoined preconditions are contradictory and
every `ensures` held only vacuously — all of the function's clauses are reported
`status: "vacuous"` and the command fails closed. If the label is **reachable**
(`VERIFICATION FAILED`), the precondition domain is non-empty and the proof is
live. This is operationally the dual of the classic `ensures: false` re-check —
asking "is the post-`requires` point reachable?" instead of "does `assert(false)`
still pass?" — but it needs only one extra run per function and is unaffected by
body divergence, since the label precedes the body. The label sits after the
requires prefix rather than at the function end precisely so an unbounded loop or
an `assume(0)` deeper in the body cannot make it spuriously unreachable.

**Interesting witnesses.** Beer et al. also propose the dual of a counterexample:
for a proof that holds, emit a non-trivial *witness* — concrete inputs that
exercise the property for a substantive reason — so the author can confirm the
proof is not hollow. Vow's structured output is well-suited to carrying a witness
alongside each `Verified` result.

## When to write contracts

### Builtins and `extern` blocks

Runtime functions (`Vec.push`, `String.from`, `HashMap.insert`) are implemented in
Rust/C and cannot be verified by ESBMC. Their behavior enters verification through
the `vow` contract on the `extern "C"` block, which becomes an **assumed**
(`__ESBMC_assume`) surface for callers. Because these contracts are *assumed, not
checked*, they are the most dangerous place for an error: a wrong `ensures` on an
extern block silently weakens every proof that depends on it. Extern contracts
must be reviewed as assumptions, audited against the runtime implementation, and
kept minimal. (Omitting the block is a `MissingContract` error — see `errors.md`.)

### Library functions (written in Vow)

Public Vow functions are fully within verification reach. Give each one its true
domain precondition and the strongest postcondition shape that applies (3–6, not
2). Add contracts when the function's contract is *known*, which is usually at
definition time for pure utilities and after the signature stabilizes for APIs.

### Application code (including agent-generated)

Vow's target author is an AI agent, and the failure mode to design against is
*volume over substance* — an agent emitting many `ensures: result >= 0` clauses
because the prompt said "add contracts." Skill guidance should push the opposite:
for each function, identify which shape applies (equality, round-trip, dispatch
totality, relational) and write that one; prefer one discriminating contract to
five weak ones. The Specification Pattern System of Dwyer, Avrunin, and Corbett
(ICSE 1999) — a survey-validated catalog built specifically to turn imprecise
intent into precise specifications — is the model for guiding an author from "this
should be valid" to a postcondition that says what *valid* means.

## Expressibility and verifiability matrix

Whether a shape is usable depends on five independent axes, not just "can the
syntax say it":

- **expressible** — surface Vow has the syntax
- **typechecked** — the checker validates the clause to `bool` (added in #81 Phase 0)
- **lowerable** — the lowerer emits IR for the clause
- **modelable** — the C emitter / ESBMC model supports the operations used
  (pure, non-effectful helpers only; see `is_modelable` in the C emitter)
- **backend** — ESBMC actually discharges it within bounds

| Shape | expressible | typechecked | lowerable | modelable | backend |
|-------|:-----------:|:-----------:|:---------:|:---------:|:-------:|
| 1. Domain precondition | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2. Output-range postcond. | ✓ | ✓ | ✓ | ✓ | ✓ (but weak) |
| 3. Exact equality | ✓ | ✓ | ✓ | ✓ | ✓ within overflow bounds |
| 4. Round-trip / inverse | ✓ | ✓ | ✓ | ✓ if partner is pure & modelable | ✓ for arithmetic |
| 5. Dispatch totality | ✓ | ✓ | ✓ | ✓ (pure dispatch) | ✓ over finite domain |
| 6a. Argument-relational | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6b. Cross-fn uniqueness | ✓ (O(n²)) | ✓ | ✓ | ✓ | ✓ but unscalable → prefer structural encoding |
| 7. Loop invariant | ✓ | ✓ | ✓ | ✓ | partial (bounded / k-induction) |
| Bounded quantifier (`forall i in 0..n`) | ✗ (no surface syntax) | — | — | — | — (roadmap #467/#470) |

Contract expressions must be **pure** — they cannot call effectful functions
(`grammar.md` §Contract Purity). A property that needs an effectful helper is
blocked at the *modelable* axis, not the expressible one; classify such gaps as
model limitations, not contract-language limitations.

## Tooling

`vow contracts --verify` performs the **per-obligation** quality analysis tracked
in #81 / roadmap WS-3.2. Each clause gets an individual verification verdict (via
ESBMC `--multi-property`), plus the three quality signals above:

- **Tautology** — the static `quality` field flags constant clauses (no ESBMC).
- **Vacuity** — a contradictory `requires` is caught by the `--error-label`
  reachability probe and reported `status: "vacuous"` (fail-closed).
- **Weakness** — the body-replace probe reports `trivially_satisfiable: true` for
  an `ensures` a trivial `return <default>` body satisfies (informational).

The `summary` carries `vacuous` and `trivially_satisfiable` counts alongside the
status and quality tallies, so an author or CI can gate on hollow proofs.

**CI weak-gate.** `scripts/check_contract_quality.py` ratchets on the static
quality of the self-hosted compiler's own contracts: it reads
`vow contracts compiler/main.vow` and fails if the `weak` or `tautological` count
exceeds a committed baseline, so a new `ensures result >= 0` cannot slip in
unnoticed. It runs in `scripts/full_test.sh`. The baseline is an upper bound to
ratchet down as contracts harden. The dispatch-totality example above
(`binop_opcode`, `ensures: result != -1`) and `binop_result_ty`
(`ensures: result == ITY_BOOL() || result == ITY_U64() || result == ITY_I64()`)
are enforced in `compiler/lower.vow` today.

**Tag families are structural, not contracted.** The bulk of the old `weak`
count was nullary tag constants — `fn IOP_VOW_REQ() -> i64 { 73 }`, the `ITY_*`,
`EXPR_*`, `BINOP_*`, `RSUM_KIND_*`, … enum families. A per-constant `ensures
result >= 0` proves nothing: a constant's value is the only fact about it, and
that fact is structural (each is a distinct literal). So these carry **no**
contract. Their correctness is established where it matters — at use sites: the
dispatch-totality contracts above prove every valid tag is handled, the IR
validator and serializer round-trips exercise every kind, and the binary
fixed-point bootstrap miscompiles if any two tags collide. Removing the
contracts cut the compiler's `weak` count from 408 to 11; the remaining bit-packers — the
region/span packers and friends (`region_pack`/`region_kind`/`region_val`,
`span_pack`, `item_kind`, `marker_caller_store`, `suffix_len`) — were then hardened
with exact functional
postconditions: `item_kind` with `result == v / 4294967296`, and `suffix_len` with a
per-suffix conditional mapping (`(suffix != tok_suffix_i64() || result == 3) && …`, one
conjunct per suffix plus an unknown-suffix → `0` clause), bringing `weak` to **0** (#81).
The CI weak-gate now holds that baseline: no weak
contract may enter the self-hosted compiler.

## References

- N. Polikarpova, C. A. Furia, Y. Pei, Y. Wei, B. Meyer. *What Good Are Strong
  Specifications?* ICSE 2013. https://arxiv.org/abs/1208.3337
- I. Beer, S. Ben-David, C. Eisner, Y. Rodeh. *Efficient Detection of Vacuity in
  Temporal Model Checking.* Formal Methods in System Design 18:141–163, 2001.
- M. Dwyer, G. Avrunin, J. Corbett. *Property Specification Patterns for
  Finite-State Verification.* ICSE 1999.
- B. Meyer. *Object-Oriented Software Construction* (Design by Contract).

---

# Vow Error Catalog

Every Vow error has a machine-readable `error_code` in the JSON output. This document lists all error codes, their phase, meaning, an example trigger, and how to fix them.

## Compile-Time Errors

These appear in the `diagnostics` array of the build output JSON.

### UnterminatedString

**Phase:** Lexer
**Meaning:** A string literal was opened with `"` but never closed.

```vow
fn f() -> () [io] {
    print_str("hello);
}
```

**Fix:** Close the string with a matching `"`.

### InvalidCharacter

**Phase:** Lexer
**Meaning:** The source contains a character the lexer does not recognize.

```vow
fn f() -> i64 {
    x @ y
}
```

**Fix:** Remove the invalid character. Vow has no `@` operator.

### UnexpectedToken

**Phase:** Parser
**Meaning:** The parser encountered a token it did not expect at that position.

```vow
module M 123
```

**Fix:** Check the syntax around the reported span. Common causes: missing `{`, `}`, `(`, `)`, or a keyword in the wrong position.

### MissingDelimiter

**Phase:** Parser
**Meaning:** A matching delimiter (`}`, `)`, `]`) is missing.

```vow
fn f() -> i64 {
    42
```

**Fix:** Add the missing closing delimiter.

### TypeMismatch

**Phase:** Type Checker
**Meaning:** An expression has a different type than expected.

```vow
fn f() -> i32 {
    true
}
```

**Output:** `function body has type 'bool' but declared return type is 'i32'`

**Fix:** Change the expression or the declared type to match.

### LiteralOutOfRange

**Phase:** Type Checker
**Meaning:** An integer literal appears in a typed context (annotated `let`, function argument, struct field, or const declaration) whose target type cannot hold the literal's value. The check runs after context coercion, so the offending literal is the one written in the source, not a widened intermediate.

```vow
let x: u8 = 300;
const NEG: u16 = -1;
```

**Output:** `literal 300 does not fit in u8 (range 0..=255)`

**Fix:** Use a value within the target type's range, change the target type, or write an explicit narrowing intrinsic (`i64_to_u8_try`, `i64_to_u8_wrap`, `i64_to_u8_sat`) if you intend to convert a wider value at runtime.

### NarrowingCastNotAllowed

**Phase:** Type Checker
**Meaning:** The `as` operator was used to convert a wider integer type to a narrower one. `as` is widening-only; narrowing must use a named intrinsic so the agent chooses an explicit semantics (range-checked vs. truncating vs. saturating). See `grammar.md` §Type Cast.

```vow
fn f(big: i64) -> u8 {
    big as u8
}
```

**Output:** `cannot cast 'i64' to 'u8' via 'as'; use 'i64_to_u8_try', 'i64_to_u8_wrap', or 'i64_to_u8_sat' to choose the narrowing semantics`

**Fix:** Replace the cast with the narrowing intrinsic that matches your intent:
- `i64_to_u8_try(big) -> Option<u8>` — reject out-of-range with `None`
- `i64_to_u8_wrap(big) -> u8` — truncate (keep low bits)
- `i64_to_u8_sat(big) -> u8` — clamp to `0..=255`

### ShiftCountOutOfRange

**Phase:** Type Checker
**Meaning:** A constant-expression shift count is greater than or equal to the bit-width of the left operand. Shifting an `N`-bit value by `>= N` bits is undefined in the underlying C model and is rejected at compile time when the count is statically known. Dynamic shift counts (non-const expressions) get a Vow contract on the operation and are checked by ESBMC and at runtime in debug mode.

```vow
fn f(x: u8) -> u8 {
    x << 8
}
```

**Output:** `shift count 8 is out of range for u8 (max 7)`

**Fix:** Use a count less than the LHS bit-width. To shift a narrow value by a larger amount, widen first: `(x as u32) << 8` is legal (it shifts the widened `u32` value by 8), but the result is `u32`; to get back to `u8`, use a narrowing intrinsic such as `u32_to_u8_wrap`.

### StaticLiteralRequired

**Phase:** Type Checker
**Meaning:** A compiler intrinsic requires a string literal operand so it can be lowered without allocation.

```vow
fn f(s: String, key: String) -> i64 {
    string_matches_literal_at(s, 0, key)
}
```

**Output:** `string_matches_literal_at requires a string literal as its third argument`

**Fix:** Pass a literal directly, for example `string_matches_literal_at(s, 0, "name")`.

### EffectViolation

**Phase:** Type Checker
**Meaning:** A function calls another function with effects not declared in its own signature.

```vow
fn f() -> () {
    print_str("hi");
}
```

**Fix:** Add the required effect to the function signature: `fn f() -> () [io]`.

### LinearTypeViolation

**Phase:** Type Checker
**Meaning:** A value of a `linear struct` type is used in a way that is immediately invalid before region inference runs, such as consuming it twice, consuming it inside a loop that may execute more than once, or consuming it after only some control-flow paths already consumed it.

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> Handle {
    let h2: Handle = h;
    let h3: Handle = h;  // h was already consumed
    h2
}
```

**Fix:** Restructure ownership so each path uses a consumed linear value at most once. Obligations that are simply left live at scope exit are reported later as `RegionLinear`.

### RegionLinear

**Phase:** Region Inference
**Meaning:** A `linear struct` value can remain live when its owning region closes. Returning the value transfers the linear obligation to the caller; consuming it before the close satisfies the obligation.

```vow
linear struct Handle { fd: i64 }

fn f() -> i64 {
    let h: Handle = Handle { fd: 1 };
    0
}
```

**Fix:** Consume the value before the region closes, or return it so the caller receives the obligation.

### NonExhaustiveMatch

**Phase:** Type Checker
**Meaning:** A `match` expression does not cover all possible variants.

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
    }
}
```

**Fix:** Add a `_ => ...` wildcard arm or cover all variants (`Option::None => ...`).

### UnsupportedPattern

**Phase:** Type Checker
**Meaning:** A parsed `match` pattern or scrutinee is not in the subset that
the compiler can lower safely. Match currently accepts enum-valued scrutinees,
qualified unit variants, qualified tuple variants with `_` or immutable
identifier payloads, and final catchall `_` or immutable identifier arms.

```vow
fn f(n: i64) -> i64 {
    match n {
        0 => 5,
        _ => 9,
    }
}
```

**Output:** `literal match patterns are not supported`

**Fix:** Use `if`/`else` comparisons for scalar or literal cases. For enum
payloads, bind each payload to `_` or an immutable identifier and inspect it
separately. Unsupported patterns fail before lowering and never produce an
executable.

### ImmutableAssignment

**Phase:** Type Checker
**Meaning:** A binding not declared `mut` was reassigned. Bindings are immutable
by default; `mut` is required only for whole-binding reassignment `x = e`. Field
writes (`s.f = e`) and index writes (`v[i] = e`) are allowed through any binding.

```vow
fn f() -> i64 {
    let x: i64 = 1;
    x = 2;
    x
}
```

**Fix:** Declare the binding `mut`: `let mut x: i64 = 1;`.

### UnusedMut

**Phase:** Type Checker
**Meaning:** A `let mut` binding is never reassigned, so the `mut` is dead. Only
whole-binding reassignment counts as a use of `mut` — a binding mutated solely
via `s.f = e`, `v[i] = e`, or a method call does not need `mut`.

```vow
fn f() -> i64 {
    let mut x: i64 = 1;
    x
}
```

**Fix:** Remove `mut`: `let x: i64 = 1;`.

### UnknownMethod

**Phase:** Type Checker
**Meaning:** A method call uses a name that does not exist on the receiver type.

```vow
fn f() -> () {
    let v: Vec<i64> = Vec::new();
    v.psh(42);
}
```

**Output:** `unknown method 'psh' on type 'Vec<i64>'`

**Fix:** Check the method name for typos. Use `--help` to see available methods for each type.

### UnsupportedFeature

**Phase:** Type Checker
**Meaning:** A language feature that is not supported in Vow was used.

```vow
trait Foo {
    fn bar() -> i64;
}
```

**Output:** `trait blocks are not supported in Vow`

**Fix:** Remove the unsupported construct. Vow does not support traits or impl blocks.

### BTreeMapKeyTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `K` not equal to `i64`. Phase 1 of the BTreeMap stdlib only supports `i64` keys; the runtime helpers and ESBMC C model are hard-coded to i64.

```vow
fn f() -> () {
    let m: BTreeMap<bool, i64> = BTreeMap::new();
    m.insert(true, 1);
}
```

**Output:** `BTreeMap key type must be i64; found 'bool'`

**Fix:** Use `BTreeMap<i64, V>`. If you need string or struct keys, hash or intern them to `i64` at the call site and keep a side-table for the originals.

### BTreeMapValueMustBeNonLinear

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with a `V` that is or transitively contains a `linear struct`. Non-linear containers like `BTreeMap`, `Vec`, and `HashMap` cannot hold linear values because their internal shift/copy operations are bitwise and would silently duplicate the linear ownership obligation.

```vow
linear struct Token { id: i64 }
fn f() -> () {
    let m: BTreeMap<i64, Token> = BTreeMap::new();
}
```

**Output:** `BTreeMap value type must be non-linear; found 'Token'`

**Fix:** Either drop the `linear` qualifier on the struct, or keep handles in a `Vec<i64>` indirection and consume the linear values via direct function calls outside the map.

### MissingContract

**Phase:** Type Checker
**Meaning:** An `extern "C"` block was declared without a `vow { ... }` contract. Every foreign function call requires a mandatory contract specifying expected behavior.

```vow
extern "C" {
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

**Output:** `extern block requires a vow contract`

**Fix:** Add a `vow { ... }` block to the extern declaration with `requires` and/or `ensures` clauses.

### ContractTypeMismatch

**Phase:** Type Checker
**Meaning:** A `requires`, `ensures`, or `invariant` clause expression does not have type `bool`.

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a + b
} {
    a + b
}
```

**Output:** `` `requires` clause has type `i64` but must be `bool` ``

**Fix:** Ensure every contract clause is a boolean expression (comparison, logical operator, or a call to a predicate function returning `bool`).

### VowRequiresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs that violate a `requires` precondition. This is a **static** verification error — it means the function's callers can reach it with invalid arguments.

**Fix:** Strengthen the `requires` clause, or fix the callers to pass valid arguments.

### VowEnsuresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs where the function's return value does not satisfy the `ensures` postcondition.

**Fix:** Fix the function body to satisfy the postcondition, or weaken the `ensures` clause.

### VowInvariantViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found a loop iteration where the `invariant` does not hold.

**Fix:** Strengthen the invariant or fix the loop body.

### EsbmcNotFound

**Phase:** Verification
**Meaning:** ESBMC is not installed or not on `$PATH`. When verification is enabled (the default for `vowc build`, always for `vowc verify`), the compiler checks for ESBMC upfront before compilation. If ESBMC is not found, the build aborts immediately with exit code 1.

**Fix:** Install ESBMC, or use `--no-verify` to skip verification: `vowc build --no-verify <file>`.

### RegionConflict

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Meaning:** A heap-typed value's required lifetime cannot be satisfied by the regions the surrounding code provides. This fires when an interprocedural store-effect constraint is unsatisfiable against the **inferred** region — that is, the value's `region(I) = LUB(must_outlive(I))` resolves to a concrete block strictly narrower than the target container's region.

> **Coverage note (as of issue #314):** the check is now semantic, consulting
> the inferred region populated by §4.1 step 3's LUB pass rather than the
> raw IR opcode. A fresh allocation routed through a callee's store-effect
> chain into a parameter container has its inferred region widened to
> `Caller(HiddenRegionIdx(N))` by §4.1 step 2's must-outlive marker
> propagation, where `N` is the precise slot index implied by the
> destination (issue #317 slot-aware inference). Such single-slot routings
> satisfy the constraint and are accepted. Allocations whose caller-region
> markers require more than one hidden caller-arena slot (for example, the
> value is stored into two distinct parameter targets, or returned and also
> stored into a parameter) have no single caller arena that outlives every
> destination, so their inferred region widens to the root region (`Root`) —
> a strictly wider placement than any one escaped pointer requires, hence
> sound (leak-but-safe) — and they compile without a blocking error (issue
> #871). Such a widen-to-root placement is not silent, though: it surfaces a
> non-blocking `RegionRootEscape` **note** (issue #366; see below), so the
> permanent root-region placement is still visible. `RegionConflict`
> therefore fires only when a value's inferred region is a concrete block
> strictly narrower than the target container's region.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). For routings that compile cleanly but you'd like to know about (root-region placement), see `RegionRootEscape` below. See `docs/design/arena_memory.md` §4.4 for the full rejection vs. visibility distinction.

### RegionRootEscape

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Severity:** Note (informational — does not fail the build)
**Meaning:** A heap allocation may land in the never-freed root region (`__vow_root_arena`). The note fires in either of two cases:

1. The allocation's inferred region is `Caller` and the surrounding function publishes a `FreshInCaller` return summary or store effect — so the value may flow up the caller chain to `main`.
2. The allocation's region **widens to the root region** without an intrinsic root pin (`pin_to_root` / a literal) — either because it is routed into more than one distinct hidden caller slot (a multi-slot widen), or because it reaches a container through a Phi over caller containers (a Phi widen). Both are sound (leak-but-safe) placements, but the allocation lives for the whole process.

This is a memory-cost decision the compiler surfaces visibly per `docs/design/arena_memory.md` §4.4: silent root-region placement caused growth-with-no-signal in earlier compiler versions, and the note restores that signal without conflating it with unsoundness (`RegionConflict`).

The note is conservative — it fires for any qualifying allocation in a function that could route to a caller or that widens to root, even if the actual concrete chain in this program doesn't reach `main`. False positives are tolerated because the diagnostic is non-blocking. A widen-to-root allocation is flagged even when it is also returned: being returned does not undo a root-region placement.

```json
{
  "error_code": "RegionRootEscape",
  "severity": "note",
  "message": "allocation may live in the root region: routed via store-effect chain to a caller whose target_region ultimately resolves to root",
  "hints": [
    "if intentional (e.g. program-lifetime data), no action needed; if you want this allocation freed earlier, restructure so the value is returned rather than stored into a parameter container"
  ]
}
```

**Fix:** Often none — if the program is short-lived (a checker, a CLI tool) or the values are genuinely program-lifetime, the note is informational. To free the allocation earlier, restructure so the value is **returned** from the constructing function rather than stored into a parameter container; the canonical `FreshInCaller` return path (`fn make_X() -> X`) does not trigger the note for the returned value or any allocation installed as a field of the returned struct (e.g. `Item { name: String::from("hi") }`). The exemption applies only to the *currently-installed* field initializers — a field overwritten before the return (`x.f = A; x.f = B; return x`) does not suppress the dead allocation `A`, which fires the note as expected (per-block last-write dedup, issue #326).

### VerificationSkipped

**Phase:** Verification (Warning surfaced alongside `BuildStatus::Skipped`)
**Meaning:** The function carries a `vow {}` block but its body uses opcodes the verifier's C model cannot represent — most commonly `RegionAlloc` and `FieldSet` produced by struct construction, also `Load`/`Store`, `RemF*`, and the `Linear*` family. The function is skipped before any C is emitted or ESBMC is invoked. The contract becomes documentary: runtime checks still apply in `--mode debug`, but no static proof is attempted.

```json
{
  "error_code": "VerificationSkipped",
  "severity": "warning",
  "message": "skipped verification of `ir_inst_set_region`: function `ir_inst_set_region` is not modelable in the verifier (contains unsupported opcode `RegionAlloc`)",
  "hints": [
    "the contract is documentary; runtime checks still apply in --mode debug"
  ]
}
```

**Why the build fails closed.** Per `CLAUDE.md`'s "Contract Authoring" guidance, contracts express semantic correctness and must not be weakened to fit the verifier. When the verifier's bounded model checker cannot represent a function's body, the function is skipped with a structured warning instead of tripping the defense-in-depth `__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")` that historically broke the bootstrap on every vowed struct-builder. But a skipped contract is still an unproved contract, so the build lifts its overall status to `Skipped` (exit 1). Use `--no-verify` if you explicitly want a non-failing path that does not invoke ESBMC at all (`Unverified`, exit 0).

**Fix:** Refactor the function so its body uses only modelable opcodes — typically by splitting allocation/initialisation away from the contract-bearing computation. Alternatively, run with `--no-verify` if the contract is intentionally documentary.

## Runtime Errors

These are emitted to stderr as JSON when a compiled program runs (debug mode for VowViolation).

**Exit status.** Every runtime abort below terminates the process with the reserved exit status **`134`** (128 + `SIGABRT`, the conventional "aborted" status), never a plain `1`. A runtime abort is an environment or soundness failure, not an application result. `134` is **reserved for aborts by convention**: a runtime abort never *spontaneously* collides with an application's own `return N` from `main`, so a program that does not itself return — or `process_exit` — `134` can treat any `134` exit as a runtime abort (a checker that returns `0`/`1`/`2` for accepted/rejected/declined will never mistake an out-of-memory or a contract violation for a genuine "rejected"). The runtime does not *enforce* the reservation — `process_exit(134)` and `return 134i32` still exit `134` — so a program that deliberately uses `134` opts out of the distinction; applications that care should reserve around it. The JSON envelope on stderr still names the specific abort. This is separate from the *compiler* exit codes in [`cli.md`](cli.md), which describe `vowc build`/`vowc verify`.

### VowViolation

**When:** Debug mode only (`--mode debug`). A `requires`, `ensures`, or `invariant` predicate evaluates to false at runtime.

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

The `blame` field indicates who is at fault:
- `Caller` — a `requires` was violated (the caller passed bad arguments)
- `Callee` — an `ensures` or `invariant` was violated (the function has a bug)

**Fix:** See the `description` and `values` fields to understand which predicate failed and with what runtime values.

### ArithmeticOverflow

**When:** A checked arithmetic operator (`+!`, `-!`, `*!`, `/!`, `%!`) overflows at runtime.

```json
{"error":"ArithmeticOverflow"}
```

**Fix:** Use wrapping arithmetic (`+`, `-`, etc.) if overflow is acceptable, or add bounds contracts to prevent overflow.

### UnwrapOnNone

**When:** `.unwrap()` is called on `Option::None`.

```json
{"error":"UnwrapOnNone"}
```

**Fix:** Use `match` to handle `None`, or add contracts that guarantee the value is `Some`.

### IndexOutOfBounds

**When:** A `Vec` index access (`v[i]` or `v[i] = val`) uses an index outside `0..v.len()`.

```json
{"error":"IndexOutOfBounds"}
```

**Fix:** Add a bounds check before indexing, or add contracts: `requires: i >= 0, requires: i < v.len()`.

### RegionLiteralMutation

**When:** A `Vec`, `String`, or `HashMap` mutation is attempted on a literal-backed container — one whose descriptor carries the `VOW_CAP_RODATA` sentinel (backing lives in `.rodata` or was pinned to the root region). Calls that statically trace a mutating target to a literal are rejected during compilation with this code; a runtime fallback emits the JSON shape below if an unchecked mutation reaches a `VOW_CAP_RODATA` descriptor. See `docs/design/arena_memory.md` §6.1, §7.3.

```json
{"error":"RegionLiteralMutation","operation":"String::push_str","origin":"rodata"}
```

A plain-text hint follows on the next line (not a JSON field). The hint text is dispatched on the operation's type prefix:

```
hint: make an explicit mutable copy with String::from(value) before mutating  # for String::* operations
hint: construct a mutable Vec and copy entries before mutating                # for Vec::*    operations
hint: construct a mutable HashMap and copy entries before mutating  # for HashMap::* operations
```

The `operation` field identifies the source-level method that trapped (e.g., `Vec::push`, `Vec::pop`, `HashMap::insert`, `String::clear`). The `origin` field identifies the storage class of the immutable backing; today only `rodata` is emitted.

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(value)`, or construct a fresh mutable container and copy the entries you need before mutating.

### StackOverflow

**When:** The native call stack is exhausted, typically due to unbounded recursion.

```json
{"error":"StackOverflow"}
```

In debug or sanitize mode, the diagnostic includes call depth and the function that was executing when the overflow occurred:

```json
{"error":"StackOverflow","depth":10693,"function":"recurse"}
```

The signal handler is installed in **all** build modes. The `depth` and `function` fields are only available in debug/sanitize mode where call-depth instrumentation is emitted.

**Fix:** Add a base case to recursive functions, or restructure the algorithm to use iteration instead of recursion.

### OutOfMemory

**When:** A runtime arena operation (`__vow_arena_open` or `__vow_arena_alloc`) failed because the underlying `malloc` returned null. Non-recoverable from within Vow (`docs/design/arena_memory.md` §3.3, §16).

```json
{"error":"OutOfMemory","operation":"arena_alloc"}
```

The `operation` field is `arena_open` for the initial chunk allocation or `arena_alloc` for a later fallback chunk allocation.

Like every runtime abort, an OOM exits with the reserved status **`134`** (see *Exit status* above), so it is distinguishable from an application's own `exit 1`.

**Fix:** Reduce working-set size, raise the process memory limit, or run on a machine with more memory. This is not a Vow program error.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.

---

# Vow Standard Library

The standard library is a curated set of reusable, contract-annotated Vow modules
under `stdlib/`. Each module is a self-contained directory: one or more library
`.vow` files plus a `main.vow` that demonstrates and exercises the API.

This is a **reference collection**, not a globally-importable package set. Vow has
no module search path today (see [Consumption model](#consumption-model)). Modules
carry contracts, but only some are statically verifiable under the current ESBMC
model — read [Verification status](#verification-status) before relying on a
contract as a proof rather than a runtime check.

In all examples below, `vow` refers to `build/vowc`. Always run `ulimit -v 2000000`
before invoking the compiler or any binary it produces.

## Modules at a glance

| Module path           | Provides                                                                 | ESBMC status |
|-----------------------|-------------------------------------------------------------------------|---------------------------|
| `stdlib/math`         | `arithmetic`, `number_theory`, `vec_math` — integer & vector math        | VerifyFailed (env)        |
| `stdlib/heap`         | `min_heap`, `max_heap` — binary heaps over `i64`                          | VerifyFailed (env)        |
| `stdlib/stack`        | `stack` — Vec-backed LIFO stack over `i64`                               | Skipped     |
| `stdlib/geometry`     | `point`, `shape` — 2D points, circles, rectangles                        | **Verified**              |
| `stdlib/bignum`       | `bignum` — arbitrary-precision signed integers                          | Skipped     |
| `stdlib/gc`           | `gc` — mark-and-sweep garbage collector over `i64` slots                 | VerifyFailed              |

These are the `vow verify <module>/main.vow` results, measured against ESBMC 8.3.0. `(env)` marks an environmental verifier limitation, not a contract
defect. The statuses reflect the verifier's memory model, **not** the soundness of
the contracts — see [Verification status](#verification-status).

## Consumption model

`use` declarations resolve to a single directory: `use foo` loads `<dir>/foo.vow`,
where `<dir>` is the directory of the **entry file** passed to `vow build`/`vow verify`.
All transitive `use`s in dependency modules resolve against that **same** directory.
There is no search path, and `--module-root` is only available on `vow test` — not
`vow build` or `vow verify`.

Two practical ways to use a stdlib module:

**1. Run the module's own demo in place.** Each module ships a `main.vow`. Build with
`--no-verify` — most stdlib modules do not pass `vow verify` yet (see
[Verification status](#verification-status)), and the point here is to *run* the demo,
not to verify it:
```
$ ulimit -v 2000000; build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo
$ ulimit -v 2000000; /tmp/math_demo
```

**2. Copy the module's `.vow` file(s) into your project directory.** Because `use`
resolves against your entry file's directory, the library file must sit next to
your program. For a single-file module:
```
$ cp stdlib/math/arithmetic.vow myproject/arithmetic.vow
```
```vow
module Main
use arithmetic

fn main() -> i32 [io] {
    print_i64(clamp(15, 0, 10));   // 10
    0
}
```
For a multi-file module, copy **all** sibling files together — e.g. `stdlib/geometry`
ships `shape.vow` which internally does `use point`, so `point.vow` must be copied
alongside it.

> A real import mechanism (a module search path so `use std.math.arithmetic`
> resolves from any location) is future work. Until then, treat stdlib modules as
> vendored source you copy in, exactly like the self-hosted compiler's own modules.

## Verification status

The verifier statuses below were measured with `vow verify` against ESBMC 8.3.0.
They are **pre-existing properties of the code and the verifier**, unchanged by the
move into `stdlib/`. A `Skipped`/`VerifyFailed` status does not mean a contract is
wrong — in `--mode debug` every contract is still enforced at runtime via
`__vow_violation`.

| Module          | `vow verify` result | Why                                                                                                   |
|-----------------|---------------------|-------------------------------------------------------------------------------------------------------|
| `geometry`      | `Verified`          | The vowed shape functions use exact `i64` overflow bounds and are fully modelable. (`point_distance_sq` carries no contract, so it is not a proof obligation — see the geometry section.) |
| `math`          | `VerifyFailed`      | The old `abs`/`<stdlib.h>` collision is resolved — user functions are namespaced `vow_user_fn_<id>` in the emitted ESBMC model. The remaining blocker is genuine: `pow`'s `ensures result >= 0` is refuted by an `i64` overflow counterexample (a large `base`/`exp` wraps negative). A contract-hardening gap (overflow guard needed), not environmental. |
| `heap`          | `VerifyFailed`*     | A `Vec`-typed argument to a helper hits a C-model type mismatch; most heap functions are `Skipped` because `Vec`/region allocation (`RegionAlloc`) is not modelable. |
| `stack`         | `Skipped`           | `stack_push` allocates a `Vec` (`RegionAlloc`), which the verifier cannot model; contracts are documentary. |
| `bignum`        | `Skipped`           | `Vec`-based limb arithmetic allocates per call (`RegionAlloc`); not modelable. 24 `RegionRootEscape` notes (the demo intentionally holds results for program lifetime). |
| `gc`            | `VerifyFailed`      | ESBMC produces a `gc_add_root` precondition counterexample related to in-module caller-`requires` checking (cf. issue #764). |

\* Environmental verifier limitation, not a contract defect.

**Takeaway for agents:** only `geometry`'s `vow verify` passes today — and that proves
the *vowed* checks reachable from its demo, not every function (e.g. `point_distance_sq`
carries no contract and is not a proof obligation). For
the others, the contracts are precise specifications that are enforced at runtime in
`--mode debug`; static proof is gated on verifier-model improvements (Vec/region
modeling and the #764 caller-`requires` fix). When you build
on these modules and need a *static* guarantee, prefer `geometry`'s pattern: keep
hot paths in plain `i64` with explicit overflow `requires`.

---

## math

Three modules under `stdlib/math/`. Each is independent (no cross-`use`); copy only
the one you need. All functions are `pub`.

### math.arithmetic

Integer primitives with overflow-guarded contracts. The `safe_*` family operates on
**non-negative** inputs only — they are overflow-checked unsigned-style helpers, not
general signed wrappers.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `abs` | `(x: i64) -> i64` | `requires x > -9223372036854775807`; `ensures result >= 0`; `ensures result == x \|\| result == 0 - x` | Guards `i64::MIN` negation overflow. |
| `min` | `(a, b: i64) -> i64` | `ensures result <= a`; `result <= b`; `result == a \|\| result == b` | Tight: result is one of the inputs. |
| `max` | `(a, b: i64) -> i64` | `ensures result >= a`; `result >= b`; `result == a \|\| result == b` | |
| `clamp` | `(x, lo, hi: i64) -> i64` | `requires lo <= hi`; `ensures lo <= result <= hi` | |
| `sign` | `(x: i64) -> i64` | `ensures -1 <= result <= 1` | -1 / 0 / 1. |
| `safe_add` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a <= I64_MAX - b`; `ensures result == a + b` | Non-negative inputs only. |
| `safe_sub` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a >= b`; `ensures 0 <= result <= a` | |
| `safe_mul` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, b == 0 \|\| a <= I64_MAX / b`; `ensures result == a * b` | |
| `safe_div` | `(a, b: i64) -> i64` | `requires a >= 0, b > 0`; `ensures 0 <= result <= a` | `b > 0`, not just `b != 0`. |
| `safe_mod` | `(a, b: i64) -> i64` | `requires a >= 0, b > 0`; `ensures 0 <= result < b` | |
| `pow` | `(base, exp: i64) -> i64` | `requires base >= 0, exp >= 0`; `ensures result >= 0` | O(exp) — no fast exponentiation; no overflow guard on the running product. |
| `midpoint` | `(a, b: i64) -> i64` | `requires a >= 0, a <= b`; `ensures a <= result <= b` | Overflow-safe `a + (b-a)/2`. |
| `diff` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0`; `ensures result >= 0` | `|a - b|`. |
| `divides` | `(d, n: i64) -> bool` | `requires d != 0` | |
| `is_even` / `is_odd` | `(x: i64) -> bool` | — | |

Representative contract — overflow guard expressed in the precondition rather than
via checked arithmetic:
```vow
pub fn safe_mul(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: b == 0 || a <= 9223372036854775807 / b,
    ensures: result == a * b,
    ensures: result >= 0
}
```

### math.number_theory

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `gcd` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a > 0 \|\| b > 0`; `ensures result > 0` | Euclid; loop invariants `x >= 0, y >= 0`. |
| `lcm` | `(a, b: i64) -> i64` | `requires a > 0, b > 0`; `ensures result > 0` | No overflow guard on `(a/g)*b`. |
| `is_prime` | `(n: i64) -> bool` | `requires n >= 0` | Trial division to `i*i <= n`. |
| `power_mod` | `(base, exp, modulus: i64) -> i64` | `requires base >= 0, exp >= 0, modulus > 1, modulus <= 3037000499`; `ensures 0 <= result < modulus` | Modulus bound = `isqrt(I64_MAX)`, prevents `(r*b)` overflow. |
| `factorial` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 1` | No upper bound on `n` — product overflows past 20!. |
| `fibonacci` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0` | Iterative; overflows past F(92). |
| `isqrt` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0, result*result <= n` | Floor integer sqrt; postcondition is the real spec. |
| `largest_divisor` | `(n: i64) -> i64` | `requires n > 1`; `ensures 1 <= result < n` | Largest proper divisor. |
| `count_divisors` | `(n: i64) -> i64` | `requires n > 0`; `ensures result >= 1` | |

### math.vec_math

Operates on `Vec<i64>`. None of the summation helpers guard against accumulator
overflow — use on bounded data, or add `requires` bounds at the call site.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `vec_sum` | `(v: Vec<i64>) -> i64` | — | No overflow guard. |
| `vec_min` / `vec_max` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | |
| `vec_mean` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | Integer mean. |
| `vec_dot` | `(a, b: Vec<i64>) -> i64` | `requires a.len() == b.len()` | |
| `vec_count` | `(v: Vec<i64>, target: i64) -> i64` | `ensures 0 <= result <= v.len()` | Invariant `count <= i`. |
| `vec_all_in_range` | `(v: Vec<i64>, lo, hi: i64) -> bool` | `requires lo <= hi` | |
| `vec_is_sorted` | `(v: Vec<i64>) -> bool` | — | Ascending. |
| `vec_prefix_sum` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |
| `vec_reverse` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |

---

## heap

`stdlib/heap/min_heap.vow` and `max_heap.vow` are structural mirrors (a min-heap and
a max-heap over `i64`), with the comparator flipped. Both are value types: every
mutator takes a heap by value and returns a new one.

The defining contract pattern is the **size-shadow invariant** `size == data.len()`,
threaded through every mutator. This is what lets ESBMC reason about in-bounds
`data[i]` access without a universal quantifier:
```vow
pub fn min_heap_push(h: MinHeap, val: i64) -> MinHeap vow {
    requires: h.size == h.data.len(),
    requires: h.size < 9223372036854775807,
    ensures: result.size == h.size + 1,
    ensures: result.size == result.data.len()
}
```

| Function (min; `max_*` mirrors) | Signature | Key contracts |
|---------------------------------|-----------|---------------|
| `min_heap_new` | `() -> MinHeap` | `ensures result.size == 0, result.data.len() == 0` |
| `min_heap_len` | `(h) -> i64` | `ensures result == h.size` |
| `min_heap_is_empty` | `(h) -> bool` | `ensures result == (h.size == 0)` |
| `min_heap_push` | `(h, val: i64) -> MinHeap` | size-shadow in/out; `ensures result.size == h.size + 1` |
| `min_heap_peek` | `(h) -> i64` | `requires h.size > 0, size-shadow`; `ensures result == h.data[0]` |
| `min_heap_pop` | `(h) -> MinHeap` | `requires h.size > 0, size-shadow`; `ensures result.size == h.size - 1` |
| `min_heap_clear` | `(h) -> MinHeap` | size-shadow in; `ensures result.size == 0` |
| `is_min_heap` | `(h) -> bool` | `requires size-shadow` — runtime check of the heap-order property |

**Heap-order is a runtime predicate, by design.** Vow has no universal quantifier, so
the property `∀i. data[parent(i)] <= data[i]` cannot be written as an `ensures`.
`is_min_heap` / `is_max_heap` check it at runtime instead; the static contracts cover
index safety and the size-shadow invariant only.

---

## stack

`stdlib/stack/stack.vow` — a `Vec<i64>`-backed LIFO stack (value type). `node.vow` in
the same directory is a vestigial `Node` struct kept for the demo; the stack does not
use it.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `stack_new` | `() -> Stack` | — |
| `stack_push` | `(s, val: i64) -> Stack` | `ensures result.size == s.size + 1` |
| `stack_peek` | `(s) -> i64` | `requires s.size > 0` |
| `stack_size` | `(s) -> i64` | — |
| `stack_is_empty` | `(s) -> bool` | — |

**Known gaps (move-verbatim; tracked follow-up):** no `stack_pop`; no size-shadow
invariant (`size == data.len()`) like `heap` has; `stack_peek` has no `ensures`
relating the result to `data[size-1]`; functions are not marked `pub`; `node.vow` is
unused.

---

## geometry

`stdlib/geometry/point.vow` (a `Point` struct) and `shape.vow` (a `Shape` enum with
circle/rectangle area and perimeter). **The only module whose `vow verify` passes
today** — its shape functions use exact derived overflow bounds. Note this means the
*vowed* checks verify (`vow verify stdlib/geometry/main.vow` → `Verified`); it is not a
proof of the whole API, since `point_distance_sq` carries no contract (see Known gaps).

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `point_new` / `point_x` / `point_y` | `Point` accessors | — |
| `point_distance_sq` | `(a, b: Point) -> i64` | — (no overflow guard — gap for large coordinates) |
| `circle_area` | `(r: i64) -> i64` | `requires 0 <= r <= 1753413056`; `ensures result >= 0` |
| `rect_area` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, h == 0 \|\| w <= I64_MAX / h`; `ensures result >= 0` |
| `circle_perimeter` | `(r: i64) -> i64` | `requires 0 <= r <= 1537228672809129301`; `ensures result >= 0` |
| `rect_perimeter` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, w <= 4611686018427387903 - h`; `ensures result >= 0` |

Each magic bound is the exact threshold below which the arithmetic cannot overflow —
e.g. `circle_area` caps `r` at `floor(sqrt(I64_MAX/3))` because it computes `r*r*3`:
```vow
fn circle_area(r: i64) -> i64 vow {
    requires: r >= 0,
    requires: r <= 1753413056,
    ensures: result >= 0
}
```

**Known gaps:** the `Shape` enum is declared but the area/perimeter functions are
free functions that don't dispatch on it; `point_distance_sq` lacks an overflow
guard; `shape_at` is a demo artifact, not a real API.

---

## bignum

`stdlib/bignum/bignum.vow` — arbitrary-precision **signed** integers with a
small-int fast path (`enum BigNum { Small(i64), Big(BigMag) }`). `Small(v)` holds
any value fitting in `i64` with **no heap allocation**; `Big(m)` holds a `BigMag`
magnitude (base 2³² limbs, `Vec<u64>`, sign-magnitude) for `|value| > i64::MAX`.
Pure core language; no builtins beyond `Vec`/`String`/`u64`/`i64`. A non-negative
`BigNum` is the natural number (`Nat`) an arbitrary-precision `Nat` consumer needs;
the binary limb base makes the bitwise operations trivial limb-wise ops, which is
why this module can back a proof kernel's `Nat` / `BitVec` reductions past the 2⁶⁴
ceiling (issue #838). The fast path measured ~3–5× faster and ~40× less peak
memory on small-op-heavy loops vs. the always-allocating representation.

**Public API (selected):**
- Construct: `bignum_zero`, `bignum_from_i64`, `bignum_from_u64`, `bignum_from_string`
- Convert: `bignum_to_string`, `bignum_to_u64` (`Option<u64>`; `None` if negative or > u64)
- Predicates: `bignum_is_zero`, `bignum_is_negative`, `bignum_is_positive`
- Compare: `bignum_cmp`, `bignum_cmp_abs`, `bignum_eq`, `bignum_lt`, `bignum_gt`, `bignum_le`, `bignum_ge`
- Arithmetic: `bignum_negate`, `bignum_abs`, `bignum_add`, `bignum_sub`, `bignum_monus`, `bignum_mul`, `bignum_div`, `bignum_mod`, `bignum_divmod`
- Bitwise (on magnitude): `bignum_and`, `bignum_or`, `bignum_xor`, `bignum_shl`, `bignum_shr`
- Higher-level: `bignum_pow(base, exp: i64)`, `bignum_gcd`, `bignum_factorial(n: i64)`

**Contracts present:** `bignum_div`/`bignum_mod`/`bignum_divmod` require
`!bignum_is_zero(b)`; `bignum_pow` requires `exp >= 0`; `bignum_shl`/`bignum_shr`
require `n >= 0`; `bignum_factorial` requires `n >= 0` (internal `bigmag_sub_abs`
requires `bigmag_cmp_abs(a, b) >= 0`).

**Semantics to know:**
- **Canonicalization invariant:** a value fits `i64` ⟺ it is `Small`. Every
  result-producing op returns through `bignum_normalize`, which demotes a `Big`
  magnitude back to `Small` when it fits — so `cmp`/`eq`/`to_string` never see two
  encodings of one value. The `BigMag` magnitude keeps the usual limb invariant
  (non-empty, no leading-zero limbs except canonical zero `[0]`, `sign ∈ {-1, 1}`,
  each limb `< 2³²`); none of this is stated as a struct invariant or `ensures`.
- The `Small`/`Small` fast path uses conservative magnitude bounds (`2⁶²−1` for
  `add`/`sub`/`monus`, `2³¹` for `mul`) so it never overflows `i64`; values outside
  the bound fall to the limb path and re-normalize. Result correctness is identical
  to the all-`Big` representation.
- Division truncates toward zero; the remainder's sign matches the dividend.
- `bignum_monus` is truncated (Nat) subtraction — `max(a − b, 0)`, saturating at 0.
- `bignum_to_u64` returns `Option::None` when the value is negative or exceeds `u64`.
- Bitwise `and`/`or`/`xor` act on the **magnitude** (Nat semantics) and return a
  non-negative result; `shl`/`shr` shift the magnitude and preserve the sign
  (= multiply / floor-divide by 2ⁿ; a logical bit shift for non-negative operands).
- `bignum_pow`/`bignum_factorial` take a native `i64` exponent/argument, not a BigNum.
- `bignum_gcd` operates on absolute values; the result is non-negative.
- Multiplication is O(n·m) schoolbook (no Karatsuba).
- The limb algorithms live in internal `bigmag_*` functions over the `BigMag`
  magnitude (`bigmag_add`, `bigmag_mul`, `bigmag_divmod`, `bigmag_strip_zeros`,
  `bigmag_to_string`, …); the public `bignum_*` API wraps them with the fast path
  and `bignum_normalize`. `bignum_to_bigmag`/`bignum_normalize`/`u64_to_decimal*`
  and the `bigmag_*` set are internal, not part of the public API.

**Verification:** `Skipped` — limb arithmetic allocates `Vec`s per call (`RegionAlloc`),
which the verifier cannot model. Contracts are runtime-enforced in `--mode debug`.

---

## gc

`stdlib/gc/gc.vow` — a mark-and-sweep garbage collector over a heap of `i64` values
with explicit roots and reference edges (`struct GcHeap`). Slots are opaque integer
handles returned by `gc_alloc`; never fabricate them.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `gc_new` | `() -> GcHeap` | — |
| `gc_alloc` | `(h, val: i64) -> i64` | — (returns a slot; reuses freed slots) |
| `gc_add_root` | `(h, slot: i64)` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_remove_root` | `(h, slot: i64)` | `requires 0 <= slot < values.len()` (does **not** require alive — you may unroot a freed slot) |
| `gc_add_ref` | `(h, from, to: i64)` | `requires` both in range and alive |
| `gc_read` | `(h, slot: i64) -> i64` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_write` | `(h, slot, val: i64)` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_is_alive` | `(h, slot: i64) -> bool` | `requires 0 <= slot < values.len()` |
| `gc_count` | `(h) -> i64` | — |
| `gc_collect` | `(h) -> i64` | — (returns count of newly-freed objects) |

**Semantics to know:**
- `gc_collect` invalidates every slot not reachable from a root; calling
  `gc_read`/`gc_write` on a freed slot violates its precondition.
- Roots and references are not deduplicated — adding a root twice needs two
  `gc_remove_root` calls.
- The heap stores only `i64`; represent richer object graphs as indices/tagged ints.
- Mark/sweep handles cycles naturally via the mark bit; no separate cycle detection.

**Verification:** `VerifyFailed` — ESBMC produces a `gc_add_root` precondition
counterexample tied to how in-module caller-`requires` are checked (cf. issue #764).
Contracts are runtime-enforced in `--mode debug`.

---

## Known gaps and roadmap

These are tracked follow-ups, intentionally **not** addressed by the reorg that
created `stdlib/` (which moved code verbatim):

- **Static verifiability.** Make `Vec`/region-allocating functions modelable so
  `stack`, `bignum`, and most of `heap` can be statically verified; resolve the
  `gc_add_root` caller-`requires` counterexample (#764).
- **Contract hardening.** Add struct/representation invariants and `ensures` clauses
  to `bignum` and `gc`; add a size-shadow invariant and `stack_pop` to `stack`; add
  an overflow guard to `point_distance_sq` and to `pow` (so its `ensures result >= 0`
  holds under `i64`); wire the `Shape` enum into `geometry`'s area/perimeter functions.
- **Consistency.** Mark all intended-public functions `pub` (currently only `math`
  and `heap` do); remove or rebuild the vestigial `stack/node.vow`.
- **Distribution.** A module search path so stdlib modules can be imported without
  copying source into the consuming project.

---

# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions. The fifth shows an EOF-safe interactive command loop using `stdin_read_line()`. The sixth shows bounded-memory streaming file input.

## 1. Safe Division — Requires Pattern

### Goal

Write a division function that is safe (cannot divide by zero).

### Step 1: Write the spec

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    divide(10, 0);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/divide.vow
```

```json
{"status":"Verified","executable":"examples/divide","diagnostics":[],"counterexamples":[]}
```

ESBMC proves the contract: whenever `y != 0` holds, the division is safe.

### Step 3: Runtime behavior (debug mode)

The `main()` calls `divide(10, 0)` which violates `requires: y != 0`. In debug mode:

```
$ vow build --mode debug --no-verify examples/divide.vow -o /tmp/divide_debug
$ /tmp/divide_debug
```

Stderr:
```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"examples/divide.vow","offset":56,"values":{"y":0}}
```

The `blame: "Caller"` tells you: `main()` passed `y=0`, which violates the precondition.

---

## 2. CEGIS Broken → Fixed — The Core Workflow

### Goal

Write `safe_sub(a, b)` that always returns a non-negative result.

### Step 1: Initial attempt (broken)

```vow
module CegisBroken

fn safe_sub(a: i64, b: i64 where b >= 0) -> i64 vow {
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 2: Build

```
$ vow build examples/cegis_broken.vow
```

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": { "file": "examples/cegis_broken.vow", "offset": 76, "length": 20 }
    }
  ]
}
```

### Step 3: Diagnose

The counterexample shows `a = -9223372036854775808` (i64 min), `b = 0`. Then `a - b = a`, which is negative. The `ensures: result >= 0` is violated.

**Root cause:** We need `a >= b` to guarantee a non-negative result, and `a >= 0` to prevent negative inputs.

### Step 4: Fix

```vow
module CegisFixed

fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 5: Verify

```
$ vow build examples/cegis_fixed.vow
```

```json
{"status":"Verified","executable":"examples/cegis_fixed","diagnostics":[],"counterexamples":[]}
```

Verified. With `a >= 0`, `b >= 0`, and `a >= b`, ESBMC proves `result >= 0`.

---

## 3. Vec Fill — Loop Invariant

### Goal

Fill a vector with `n` elements and prove its length equals `n`.

### Step 1: Write the spec

```vow
module VecFill

fn fill_vec(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    requires: n <= 8,
    ensures: result.len() == n
} {
    let v: Vec<i64> = Vec::new();
    let mut i: i64 = 0;
    while i < n vow {
        invariant: i >= 0,
        invariant: i <= n
    } {
        v.push(i);
        i = i + 1;
    }
    v
}

fn main() -> i32 [io] {
    let v: Vec<i64> = fill_vec(5);
    print_i64(v.len());
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/vec_fill.vow
```

```json
{"status":"Verified","executable":"examples/vec_fill","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `requires: n <= 8` keeps iterations tractable for verification
- `invariant: i >= 0, invariant: i <= n` is inductive: true on entry, preserved by the loop body
- The Vec model tracks `len`, so ESBMC can reason about `result.len() == n`

---

## 4. Linear Search — Break-with-Value

### Goal

Search a vector for a target value and return its index, or `-1` if not found. Uses `loop` with `break <value>` to produce a result directly from the loop expression.

### Step 1: Write the spec

```vow
module Search

fn linear_search(data: Vec<i64>, target: i64) -> i64
    vow { requires: data.len() > 0 }
{
    let mut i: i64 = 0;
    let n: i64 = data.len();
    let result: i64 = loop {
        if i >= n {
            break -1;
        }
        if data[i] == target {
            break i;
        }
        i = i + 1;
    };
    result
}

fn main() -> i32 [io] {
    let data: Vec<i64> = Vec::new();
    data.push(10);
    data.push(20);
    data.push(30);
    data.push(40);
    data.push(50);

    let idx: i64 = linear_search(data, 30);
    print_i64(idx);

    let idx2: i64 = linear_search(data, 99);
    print_i64(idx2);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/search.vow
```

```json
{"status":"Verified","executable":"examples/search","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `loop { ... break <value>; ... }` is an expression that evaluates to the break value
- All `break` expressions in a `loop` must produce the same type (`i64` here)
- `break <value>` is only allowed in `loop`, not in `while` (which always evaluates to `()`)
- The result is bound with `let result: i64 = loop { ... };`

---

## 5. Command Loop — EOF-Safe `stdin_read_line`

### Goal

Write a line-oriented command interpreter that reads from stdin, dispatches commands, skips empty lines, and exits cleanly on EOF. This is the canonical pattern for CI-safe interactive programs.

### Step 1: Write the program

```vow
module CmdLoop

fn trim_newline(s: String) -> String {
    let n: i64 = s.len();
    if n == 0 { return s; }
    let last: i64 = s.byte_at(n - 1);
    if last == 10 {
        if n >= 2 {
            let prev: i64 = s.byte_at(n - 2);
            if prev == 13 {
                return s.substring(0, n - 2);
            }
        }
        return s.substring(0, n - 1);
    }
    s
}

fn skip_spaces(s: String, start: i64) -> i64 {
    let mut i: i64 = start;
    let n: i64 = s.len();
    while i < n {
        if s.byte_at(i) != 32 { return i; }
        i = i + 1;
    }
    i
}

fn main() -> i32 [read, io] {
    let mut line: String = stdin_read_line();
    while line.len() > 0 {
        let cmd: String = trim_newline(line);

        if cmd.len() > 0 {
            if cmd.eq(String::from("quit")) {
                return 0;
            }

            if cmd.eq(String::from("hello")) {
                print_str(String::from("Hello, world!\n"));
            } else {
                if cmd.len() >= 5 {
                    let prefix: String = cmd.substring(0, 5);
                    if prefix.eq(String::from("echo ")) {
                        let start: i64 = skip_spaces(cmd, 5);
                        let text: String = cmd.substring(start, cmd.len());
                        print_str(text);
                        print_str(String::from("\n"));
                    } else {
                        print_str(String::from("unknown: "));
                        print_str(cmd);
                        print_str(String::from("\n"));
                    }
                } else {
                    print_str(String::from("unknown: "));
                    print_str(cmd);
                    print_str(String::from("\n"));
                }
            }
        }

        line = stdin_read_line();
    }
    0
}
```

### Step 2: Build

```
$ vow build --no-verify examples/cmdloop.vow -o /tmp/cmdloop
```

```json
{"status":"Unverified","executable":"/tmp/cmdloop","diagnostics":[],"counterexamples":[]}
```

No contracts here — this example focuses on the I/O pattern, not verification.

### Step 3: Run with piped input

```
$ printf 'hello\necho Vow is great\n\nbogus\nquit\n' | /tmp/cmdloop
Hello, world!
Vow is great
unknown: bogus
```

The `quit` command causes an early `return 0`. Empty lines are silently skipped.

### Step 4: Run with EOF (no quit)

```
$ printf 'hello\necho test\n' | /tmp/cmdloop
Hello, world!
test
```

When stdin is exhausted, `stdin_read_line()` returns `""` (length 0), the `while` condition fails, and the program exits cleanly with code 0.

### Key points

- **EOF detection:** `stdin_read_line()` returns `""` at EOF. Check `.len() > 0` to exit the loop.
- **Newline stripping:** `stdin_read_line()` includes the trailing `\n` (or `\r\n`). Strip it with `byte_at` + `substring` before comparing commands.
- **Empty line handling:** After trimming, `cmd.len() == 0` means the line was blank — skip it.
- **Effects:** `stdin_read_line()` requires `[read]`; `print_str()` requires `[io]`. The `main` function declares both.
- **CI-safe:** No blocking reads, no prompts — the program processes whatever stdin provides and exits at EOF. Safe to run in pipelines and test harnesses.

## 6. Streaming File Input

`fs_read(path)` materializes the entire file as one `String`. Use `fs_open` plus `fs_read_line` for newline-delimited files that may be large.

```vow
module StreamingFile

fn main() -> i32 [read, io] {
    let argv: Vec<String> = args();
    if argv.len() < 2 {
        eprintln_str(String::from("usage: streaming_file <path>"));
        return 1;
    }

    let h: i64 = fs_open(argv[1]);
    if h < 0 {
        eprintln_str(String::from("could not open input"));
        return 1;
    }

    let mut lines: i64 = 0;
    let mut bytes: i64 = 0;
    let mut line: String = fs_read_line(h);
    while line.len() > 0 {
        lines = lines + 1;
        bytes = bytes + line.len();
        line = fs_read_line(h);
    }

    if fs_status(h) != 1 {
        fs_close(h);
        eprintln_str(String::from("read error"));
        return 1;
    }
    if fs_close(h) != 0 {
        eprintln_str(String::from("close error"));
        return 1;
    }

    print_i64(lines);
    print_str(String::from("\n"));
    print_i64(bytes);
    print_str(String::from("\n"));
    0
}
```

Key points:

- `fs_read_line(handle)` includes the trailing newline when present.
- Blank lines are returned as `"\n"`; EOF returns `""`.
- Check `fs_status(handle)` after `fs_read_line(handle)` returns `""`: `1` means EOF, `-1` means invalid handle or read error.
- Close each successful handle with `fs_close(handle)` and check for a non-zero close result.

## 7. BTreeMap basic usage

`BTreeMap<i64, V>` is the deterministic alternative to `HashMap` — sorted ascending by key, binary-search lookup. Use it when iteration order affects program output (codegen, serialization, or any reproducible build).

```vow
module BTreeMapExample

fn fetch(m: BTreeMap<i64, i64>) -> Option<i64> [io] {
    let r: Option<i64> = m.get(7);
    let v: i64 = r?;
    print_i64(v);
    print_str(String::from("\n"));
    Option::Some(v)
}

fn main() -> i32 [io] {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    m.insert(7, 42);
    let prev: Option<i64> = m.insert(7, 99);
    // prev is Some(42); the second insert overwrote the first.
    fetch(m);
    print_i64(m.len());
    0
}
```

Note that `.insert` returns `Option<V>` (the previous value, if any), and `.get` returns `Option<V>`. Use `?` to short-circuit on `None`. Phase 1 only supports `i64` keys; using any other key type raises `BTreeMapKeyTypeMustBeI64`.

### Why BTreeMap and not HashMap

`HashMap.insert` returns `()` and its iteration order is unspecified. For maps whose iteration is observable in the output binary, the byte-identical bootstrap requirement (`stage1 == stage2` sha256) demands deterministic order. `BTreeMap` provides it; `HashMap` does not.

---

# JSON Schemas

## build-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/build-result.schema.json",
  "title": "BuildResult",
  "description": "JSON output from `vow build` on stdout",
  "type": "object",
  "required": ["status", "executable", "diagnostics", "counterexamples"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["Verified", "Unverified", "Skipped", "CompileFailed", "VerifyFailed"],
      "description": "Build outcome. `Skipped` means ESBMC was invoked but at least one vowed function could not be modelled; the build fails closed with exit 1 (distinct from `Unverified`, which means ESBMC was not invoked, e.g. `--no-verify`/`--dump-ir`, exit 0)."
    },
    "executable": {
      "type": ["string", "null"],
      "description": "Path to compiled binary, or null on failure or when source has no main function (library module)"
    },
    "diagnostics": {
      "type": "array",
      "items": { "$ref": "diagnostic.schema.json" },
      "description": "Compiler diagnostics (parse errors, type errors, vow violations)"
    },
    "message": {
      "type": "string",
      "description": "Error detail (present only when status is CompileFailed)"
    },
    "function": {
      "type": "string",
      "description": "Function name (present only when status is VerifyFailed)"
    },
    "counterexample": {
      "type": "string",
      "description": "Legacy counterexample description (present only when status is VerifyFailed)"
    },
    "counterexamples": {
      "type": "array",
      "items": { "$ref": "counterexample.schema.json" },
      "description": "Structured counterexamples from ESBMC verification"
    },
    "verify_status": {
      "type": "string",
      "enum": ["timeout", "unknown", "error", "tool_not_found", "panicked"],
      "description": "Verification sub-status (present only when the verification backend did not produce a proof or counterexample; \"panicked\" signals the verifier worker thread crashed and the build fails closed)"
    },
    "verify_message": {
      "type": "string",
      "description": "Verification backend error detail (present only when verify_status is set)"
    }
  },
  "allOf": [
    {
      "if": { "properties": { "status": { "const": "CompileFailed" } } },
      "then": { "required": ["message"] }
    },
    {
      "if": { "properties": { "status": { "const": "VerifyFailed" } } },
      "then": { "required": ["function", "counterexample"] }
    }
  ],
  "additionalProperties": false
}
```

## complexity-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.org/schemas/complexity-result.schema.json",
  "title": "ComplexityReport",
  "description": "Output of `vow complexity <file>` on stdout. Byte-identical across the Rust and self-hosted compilers. Non-integer metrics are fixed-3-decimal numbers computed in integer fixed-point (scale 1000), never native floats.",
  "type": "object",
  "required": ["schema_version", "kind", "tool", "files", "summary"],
  "properties": {
    "schema_version": { "const": "1" },
    "kind": { "const": "complexity_report" },
    "tool": { "const": "vow" },
    "files": {
      "type": "array",
      "items": { "$ref": "#/$defs/File" }
    },
    "summary": { "$ref": "#/$defs/Summary" }
  },
  "additionalProperties": false,
  "$defs": {
    "File": {
      "type": "object",
      "required": ["file", "complexity_score", "functions_over_threshold", "nloc", "functions", "module"],
      "properties": {
        "file": { "type": "string", "description": "Source path as passed on the command line." },
        "complexity_score": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Max complexity_score across the file's functions." },
        "functions_over_threshold": { "type": "integer", "minimum": 0, "description": "Count of functions whose score exceeds the threshold (--max-score if passed, else the recommended 80)." },
        "nloc": { "type": "integer", "minimum": 0, "description": "Source line count of the file." },
        "functions": { "type": "array", "items": { "$ref": "#/$defs/Function" } },
        "module": { "$ref": "#/$defs/Module" }
      },
      "additionalProperties": false
    },
    "Module": {
      "description": "Module-level coupling aggregates (experimental tier). fan_in counts in-file callers only.",
      "type": "object",
      "required": ["tier", "functions", "fan_in_max", "fan_out_max", "henry_kafura_max"],
      "properties": {
        "tier": { "const": "experimental" },
        "functions": { "type": "integer", "minimum": 0 },
        "fan_in_max": { "type": "integer", "minimum": 0, "description": "Max in-file callers of any function." },
        "fan_out_max": { "type": "integer", "minimum": 0, "description": "Max distinct callees of any function." },
        "henry_kafura_max": { "type": "integer", "minimum": 0, "description": "Max nloc*(fan_in*fan_out)^2, saturated. Unvalidated." }
      },
      "additionalProperties": false
    },
    "Vow": {
      "description": "Vow-surface metrics (experimental tier).",
      "type": "object",
      "required": ["tier", "effects", "effect_breadth", "effect_fanout", "linear_values", "linear_consumes", "linear_borrows", "contract"],
      "properties": {
        "tier": { "const": "experimental" },
        "effects": { "type": "array", "items": { "enum": ["io", "panic", "read", "unsafe", "write"] }, "description": "Declared effects, canonical order." },
        "effect_breadth": { "type": "integer", "minimum": 0, "maximum": 5, "description": "popcount of the effect bitset." },
        "effect_fanout": { "type": "integer", "minimum": 0, "description": "Distinct in-module callees that are themselves effectful." },
        "linear_values": { "type": "integer", "minimum": 0, "description": "Count of linear-struct literals constructed in the function." },
        "linear_consumes": { "type": "integer", "minimum": 0, "description": "IOP_LINEAR_CONSUME count (linear resource moves)." },
        "linear_borrows": { "type": "integer", "minimum": 0, "description": "IOP_LINEAR_BORROW count." },
        "contract": { "$ref": "#/$defs/Contract" }
      },
      "additionalProperties": false
    },
    "Function": {
      "type": "object",
      "required": ["name", "line", "complexity_score", "score_factors", "size", "structural", "vow", "verification"],
      "properties": {
        "name": { "type": "string" },
        "line": { "type": "integer", "minimum": 1, "description": "1-based source line of the function." },
        "complexity_score": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Readability / refactor-priority gate. NOT a defect predictor." },
        "score_factors": { "$ref": "#/$defs/ScoreFactors" },
        "size": { "$ref": "#/$defs/Size" },
        "structural": { "$ref": "#/$defs/Structural" },
        "vow": { "$ref": "#/$defs/Vow" },
        "verification": { "$ref": "#/$defs/Verification" }
      },
      "additionalProperties": false
    },
    "Verification": {
      "description": "Verification-difficulty metrics (experimental tier, Vow-unique).",
      "type": "object",
      "required": ["tier", "loops_total", "loops_without_invariant", "max_loop_nesting", "contract_predicate_cost"],
      "properties": {
        "tier": { "const": "experimental" },
        "loops_total": { "type": "integer", "minimum": 0 },
        "loops_without_invariant": { "type": "integer", "minimum": 0, "description": "Loops the BMC must unwind without an invariant." },
        "max_loop_nesting": { "type": "integer", "minimum": 0 },
        "contract_predicate_cost": { "type": "integer", "minimum": 0, "description": "predicate_nodes + free_vars (value identifiers, excluding callees/method names/result) + quantifier flag, summed across clauses." }
      },
      "additionalProperties": false
    },
    "Contract": {
      "description": "Contract surface (experimental tier).",
      "type": "object",
      "required": ["requires", "ensures", "invariants", "predicate_nodes", "predicate_depth", "free_vars", "has_vec_quantification"],
      "properties": {
        "requires": { "type": "integer", "minimum": 0 },
        "ensures": { "type": "integer", "minimum": 0 },
        "invariants": { "type": "integer", "minimum": 0, "description": "Function-level invariant clauses (loop invariants are counted under verification)." },
        "predicate_nodes": { "type": "integer", "minimum": 0, "description": "Total AST nodes across all clause predicates." },
        "predicate_depth": { "type": "integer", "minimum": 0 },
        "free_vars": { "type": "integer", "minimum": 0, "description": "Distinct value identifiers across clauses; excludes the result binding, function callee identifiers, and method names." },
        "has_vec_quantification": { "type": "boolean", "description": "A predicate indexes a Vec (no quantifier syntax exists; this is the proxy)." }
      },
      "additionalProperties": false
    },
    "ScoreFactors": {
      "description": "Sub-scores the gate is built from. cognitive_sub/size_sub/base are in [0,1] (fixed 3 decimals).",
      "type": "object",
      "required": ["cognitive_sub", "size_sub", "vow_bump", "base", "over_threshold"],
      "properties": {
        "cognitive_sub": { "type": "number", "description": "anchor_map(cognitive, --cog-anchor)." },
        "size_sub": { "type": "number", "description": "anchor_map(nloc, --nloc-anchor)." },
        "vow_bump": { "type": "number", "description": "Experimental Vow-surface bump (scale 1000). Sum of effect-breadth, linear-consume, and contract-predicate penalties; capped at 150." },
        "base": { "type": "number", "description": "soft-OR of cognitive_sub and size_sub." },
        "over_threshold": { "type": "boolean" }
      },
      "additionalProperties": false
    },
    "Size": {
      "description": "Stable baseline metrics. Co-reported with every structural metric.",
      "type": "object",
      "required": ["nloc", "tokens", "stmts", "params"],
      "properties": {
        "nloc": { "type": "integer", "minimum": 0, "description": "Source lines spanned by the function." },
        "tokens": { "type": "integer", "minimum": 0, "description": "Halstead length N (total operators + operands)." },
        "stmts": { "type": "integer", "minimum": 0, "description": "Statement count (recursive; trailing expression counts as one)." },
        "params": { "type": "integer", "minimum": 0 }
      },
      "additionalProperties": false
    },
    "Structural": {
      "type": "object",
      "required": ["cyclomatic", "cyclomatic_ir", "cognitive", "max_nesting", "halstead"],
      "properties": {
        "cyclomatic": { "type": "integer", "minimum": 1, "description": "AST decision-count form: base 1 + if/while/for/loop + (match arms - 1) + each &&/|| + each ?." },
        "cyclomatic_ir": { "type": "integer", "minimum": -1, "description": "IR branch-count cross-check: (number of IOP_BRANCH) + 1, or -1 if the function has no IR (e.g. a body-less declaration). Agrees with `cyclomatic` modulo &&/||/? lowering. Experimental tier." },
        "cognitive": { "type": "integer", "minimum": 0, "description": "Vow-adapted Cognitive Complexity (nesting-aware). Headline structural metric." },
        "max_nesting": { "type": "integer", "minimum": 0, "description": "Deepest structural nesting depth." },
        "halstead": { "$ref": "#/$defs/Halstead" }
      },
      "additionalProperties": false
    },
    "Halstead": {
      "type": "object",
      "required": ["n1", "n2", "N1", "N2", "vocabulary", "length", "volume", "difficulty", "effort"],
      "properties": {
        "n1": { "type": "integer", "minimum": 0, "description": "Distinct operators." },
        "n2": { "type": "integer", "minimum": 0, "description": "Distinct operands." },
        "N1": { "type": "integer", "minimum": 0, "description": "Total operators." },
        "N2": { "type": "integer", "minimum": 0, "description": "Total operands." },
        "vocabulary": { "type": "integer", "minimum": 0, "description": "n1 + n2." },
        "length": { "type": "integer", "minimum": 0, "description": "N1 + N2." },
        "volume": { "type": "number", "description": "length * log2(vocabulary). Fixed 3 decimals, saturated." },
        "difficulty": { "type": "number", "description": "(n1/2) * (N2/n2). Fixed 3 decimals." },
        "effort": { "type": "number", "description": "difficulty * volume. Fixed 3 decimals, saturated." }
      },
      "additionalProperties": false
    },
    "Summary": {
      "type": "object",
      "required": ["functions", "nloc_total", "threshold", "functions_over_threshold", "thresholds_exceeded"],
      "properties": {
        "functions": { "type": "integer", "minimum": 0 },
        "nloc_total": { "type": "integer", "minimum": 0 },
        "threshold": { "type": "integer", "description": "--max-score if passed, else the recommended 80." },
        "functions_over_threshold": { "type": "integer", "minimum": 0 },
        "thresholds_exceeded": { "type": "array", "items": { "type": "string" }, "description": "Names of functions whose score exceeds the threshold." }
      },
      "additionalProperties": false
    }
  }
}
```

## contracts-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/contracts-result.schema.json",
  "title": "ContractsResult",
  "description": "JSON output from `vow contracts` on stdout",
  "type": "object",
  "required": ["contracts", "summary"],
  "properties": {
    "contracts": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["vow_id", "function", "kind", "description", "blame", "source", "status", "quality"],
        "properties": {
          "vow_id": {
            "type": "integer",
            "description": "Unique contract identifier within the program"
          },
          "function": {
            "type": "string",
            "description": "Function containing this contract"
          },
          "kind": {
            "type": "string",
            "enum": ["requires", "ensures", "invariant"],
            "description": "Contract kind"
          },
          "description": {
            "type": "string",
            "description": "Full contract text"
          },
          "blame": {
            "type": "string",
            "enum": ["Caller", "Callee"],
            "description": "Blame assignment: Caller for requires, Callee for ensures/invariant"
          },
          "source": {
            "type": "object",
            "required": ["file", "offset"],
            "properties": {
              "file": {
                "type": "string",
                "description": "Source file path"
              },
              "offset": {
                "type": "integer",
                "description": "Byte offset in source file"
              }
            },
            "additionalProperties": false
          },
          "status": {
            "type": "string",
            "enum": ["proven", "proven-ir", "failed", "unknown", "timeout", "error", "not_verified", "skipped", "vacuous"],
            "description": "Verification status"
          },
          "quality": {
            "type": "string",
            "enum": ["weak", "tautological", "substantive"],
            "description": "Static, no-ESBMC classification of the clause shape: weak (an ensures that only bounds result by a constant), tautological (constant clause that says nothing), or substantive (equality/relational/inverse/call). See contracts-methodology.md"
          },
          "trivially_satisfiable": {
            "type": "boolean",
            "description": "`--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational; never affects the exit code. See contracts-methodology.md"
          }
        },
        "additionalProperties": false
      }
    },
    "summary": {
      "type": "object",
      "required": ["total", "proven", "failed", "unknown", "timeout", "error", "not_verified", "skipped", "quality"],
      "properties": {
        "total": { "type": "integer" },
        "proven": { "type": "integer" },
        "failed": { "type": "integer" },
        "unknown": { "type": "integer" },
        "timeout": { "type": "integer" },
        "error": { "type": "integer" },
        "not_verified": { "type": "integer" },
        "skipped": { "type": "integer" },
        "vacuous": { "type": "integer" },
        "trivially_satisfiable": { "type": "integer" },
        "quality": {
          "type": "object",
          "description": "Static contract-quality tallies independent of verification status",
          "required": ["weak", "tautological", "substantive"],
          "properties": {
            "weak": { "type": "integer" },
            "tautological": { "type": "integer" },
            "substantive": { "type": "integer" }
          },
          "additionalProperties": false
        }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
```

## counterexample

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/counterexample.schema.json",
  "title": "Counterexample",
  "description": "A structured counterexample from ESBMC verification failure",
  "type": "object",
  "required": ["function", "values", "violation", "vow_id", "source", "blame"],
  "properties": {
    "function": {
      "type": "string",
      "description": "Name of the function whose verification query failed"
    },
    "values": {
      "type": "object",
      "additionalProperties": { "type": "string" },
      "description": "Map of source names or ESBMC variable names to counterexample values"
    },
    "violation": {
      "type": "string",
      "description": "Description of the violated contract clause"
    },
    "vow_id": {
      "type": "integer",
      "minimum": 0,
      "description": "Function-local ID of the violated vow clause"
    },
    "source": {
      "oneOf": [
        {
          "type": "object",
          "required": ["file", "offset", "length"],
          "properties": {
            "file": { "type": "string", "description": "Source file path" },
            "offset": { "type": "integer", "minimum": 0, "description": "Byte offset of the vow clause" },
            "length": { "type": "integer", "minimum": 0, "description": "Byte length of the vow clause" }
          },
          "additionalProperties": false
        },
        { "type": "string" },
        { "type": "null" }
      ],
      "description": "Source location of the violated vow clause; Rust emits a span object, self-hosted emits the source path string"
    },
    "blame": {
      "type": "string",
      "enum": ["caller", "callee", "none"],
      "description": "Who is responsible for the violation"
    },
    "call_sites": {
      "type": "array",
      "description": "Caller locations relevant to caller-blame failures",
      "items": {
        "type": "object",
        "required": ["caller_function", "file", "offset", "length"],
        "properties": {
          "caller_function": { "type": "string" },
          "file": { "type": "string" },
          "offset": { "type": "integer", "minimum": 0 },
          "length": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
      }
    },
    "violating_args": {
      "type": "array",
      "description": "Callee parameters and caller argument spans for caller-blame precondition failures",
      "items": {
        "type": "object",
        "required": ["param", "value", "arg_offset", "arg_length"],
        "properties": {
          "param": { "type": "string" },
          "value": {
            "type": "string",
            "description": "Counterexample value for the caller argument. The empty string means the value could not be statically recovered; arg_offset and arg_length still identify the caller argument."
          },
          "arg_offset": { "type": "integer", "minimum": 0 },
          "arg_length": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
      }
    },
    "execution_path": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["block_id", "offset", "length"],
        "properties": {
          "block_id": { "type": "integer", "minimum": 0 },
          "offset": { "type": "integer", "minimum": 0 },
          "length": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
      }
    },
    "branch_decisions": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["condition_offset", "condition_length", "taken"],
        "properties": {
          "condition_offset": { "type": "integer", "minimum": 0 },
          "condition_length": { "type": "integer", "minimum": 0 },
          "taken": { "type": "string", "enum": ["then", "else"] }
        },
        "additionalProperties": false
      }
    },
    "replay": {
      "type": "string",
      "enum": ["confirmed", "diverged", "skipped"],
      "description": "Differential-test outcome, present only with --replay-cex"
    },
    "replay_reason": {
      "type": "string",
      "description": "Human-readable explanation for a diverged or skipped replay"
    }
  },
  "additionalProperties": false
}
```

## diagnostic

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/diagnostic.schema.json",
  "title": "Diagnostic",
  "description": "A single compiler diagnostic (error, warning, or note)",
  "type": "object",
  "required": ["error_code", "message", "severity", "span"],
  "properties": {
    "error_code": {
      "type": "string",
      "enum": [
        "UnterminatedString",
        "InvalidCharacter",
        "UnexpectedToken",
        "MissingDelimiter",
        "TypeMismatch",
        "StaticLiteralRequired",
        "EffectViolation",
        "LinearTypeViolation",
        "NonExhaustiveMatch",
        "UnsupportedPattern",
        "ImmutableAssignment",
        "UnusedMut",
        "VowRequiresViolated",
        "VowEnsuresViolated",
        "VowInvariantViolated",
        "UnknownMethod",
        "UnsupportedFeature",
        "LoweringWarning",
        "MissingContract",
        "ContractTypeMismatch",
        "EsbmcNotFound",
        "IoError",
        "RegionConflict",
        "RegionLinear",
        "RegionRootEscape"
      ],
      "description": "Machine-readable error code"
    },
    "message": {
      "type": "string",
      "description": "Human-readable error message"
    },
    "severity": {
      "type": "string",
      "enum": ["error", "warning", "note"],
      "description": "Diagnostic severity"
    },
    "span": {
      "type": "object",
      "required": ["file", "offset", "length"],
      "properties": {
        "file": { "type": "string", "description": "Source file path" },
        "offset": { "type": "integer", "minimum": 0, "description": "Byte offset from start of file" },
        "length": { "type": "integer", "minimum": 0, "description": "Byte length of the span" }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
```

## mutants-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.org/schemas/mutants-result.schema.json",
  "title": "MutantsOutput",
  "description": "Output of `vow-mutants run` populates a directory (default `mutants.out/`). Each file has its own schema; the union is documented here.",
  "$defs": {
    "Mutant": {
      "description": "Per-mutant catalog record (in mutants.json).",
      "type": "object",
      "required": ["name", "file", "line", "col", "off", "len", "kind", "from", "to", "label", "clause_index"],
      "properties": {
        "name":         { "type": "string", "description": "Stable cargo-mutants-style name: `file:line:col: <label>`." },
        "file":         { "type": "string", "description": "Repo-relative source path. Independent of --workdir." },
        "line":         { "type": "integer", "minimum": 1, "description": "1-based source line of `off`." },
        "col":          { "type": "integer", "minimum": 1, "description": "1-based source column of `off`." },
        "off":          { "type": "integer", "minimum": 0, "description": "Byte offset of the mutation site." },
        "len":          { "type": "integer", "minimum": 0, "description": "Byte length of the mutation span." },
        "kind":         { "enum": ["op-flip", "const-flip", "body-replace", "contract-weaken"] },
        "from":         { "type": "string", "description": "Original source text at the site (empty for body-replace)." },
        "to":           { "type": "string", "description": "Replacement text spliced in for this mutation." },
        "label":        { "type": "string", "description": "Human-readable summary of the mutation." },
        "clause_index": { "type": "integer", "minimum": 0, "description": "Disambiguates sibling contract clauses on the same function. 0 for non-contract sites." }
      },
      "additionalProperties": false
    },
    "Outcome": {
      "description": "Per-mutant verdict (in outcomes.json).",
      "type": "object",
      "required": ["id", "name", "status", "tier", "oracle_ms"],
      "properties": {
        "id":         { "type": "integer", "minimum": 0, "description": "Position of this mutant within the shard's `mutants.json` array." },
        "name":       { "type": "string", "description": "Same name as the corresponding Mutant record." },
        "status":     { "enum": ["caught", "missed", "timeout", "unviable", "unrun"] },
        "tier":       { "enum": [1, 2], "description": "Oracle tier that produced the verdict." },
        "oracle_ms":  { "type": "integer", "minimum": 0, "description": "Per-mutant oracle wall-clock in milliseconds. Non-deterministic — excluded from the determinism guarantee." }
      },
      "additionalProperties": false
    },
    "Summary": {
      "description": "Aggregate counts across all of this shard's mutants. Same shape as the stdout summary line.",
      "type": "object",
      "required": ["total", "caught", "missed", "timeout", "unviable", "unrun", "shard"],
      "properties": {
        "total":     { "type": "integer", "minimum": 0 },
        "caught":    { "type": "integer", "minimum": 0 },
        "missed":    { "type": "integer", "minimum": 0 },
        "timeout":   { "type": "integer", "minimum": 0 },
        "unviable":  { "type": "integer", "minimum": 0 },
        "unrun":     { "type": "integer", "minimum": 0, "description": "Tier-1 survivors not run because the Tier-2 budget was exhausted." },
        "shard":     { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" }
      },
      "additionalProperties": false
    },
    "MutantsJson": {
      "description": "Format of `mutants.out/mutants.json`. Written before testing begins.",
      "type": "object",
      "required": ["version", "tool", "shard", "mutants"],
      "properties": {
        "version":  { "const": 1 },
        "tool":     { "const": "vow-mutants" },
        "shard":    { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" },
        "mutants":  { "type": "array", "items": { "$ref": "#/$defs/Mutant" } }
      },
      "additionalProperties": false
    },
    "OutcomesJson": {
      "description": "Format of `mutants.out/outcomes.json`. Written after all mutants in this shard have been classified.",
      "type": "object",
      "required": ["version", "summary", "outcomes"],
      "properties": {
        "version":  { "const": 1 },
        "summary":  { "$ref": "#/$defs/Summary" },
        "outcomes": { "type": "array", "items": { "$ref": "#/$defs/Outcome" } }
      },
      "additionalProperties": false
    }
  }
}
```

## test-result

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/test-result.schema.json",
  "title": "TestResult",
  "description": "JSON output from `vow test` on stdout",
  "type": "object",
  "required": ["status", "total", "passed", "failed", "skipped", "tests", "contract_density"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["TestsPassed", "TestsFailed"],
      "description": "Overall test outcome"
    },
    "total": {
      "type": "integer",
      "description": "Total number of tests discovered"
    },
    "passed": {
      "type": "integer",
      "description": "Number of tests that passed"
    },
    "failed": {
      "type": "integer",
      "description": "Number of tests that failed"
    },
    "skipped": {
      "type": "integer",
      "description": "Number of tests that were skipped"
    },
    "tests": {
      "type": "array",
      "items": { "$ref": "#/$defs/TestEntry" },
      "description": "Per-test results"
    },
    "contract_density": {
      "$ref": "#/$defs/ContractDensity",
      "description": "Contract density across tested modules"
    }
  },
  "$defs": {
    "TestEntry": {
      "type": "object",
      "required": ["file", "name", "status", "stdout", "stderr", "duration_ms", "diagnostics", "counterexamples"],
      "properties": {
        "file": { "type": "string", "description": "Path to the test .vow file" },
        "name": { "type": "string", "description": "Test name (file stem)" },
        "status": {
          "type": "string",
          "enum": ["passed", "failed", "timeout", "skipped", "compile_error", "verify_failed", "contract_skipped"],
          "description": "Per-test outcome (`contract_skipped`: ESBMC never invoked because a vowed function is non-modelable; fail-closed, counts toward `failed`)"
        },
        "exit_code": {
          "type": ["integer", "null"],
          "description": "Process exit code, null on compile/verify failure or timeout"
        },
        "stdout": { "type": "string", "description": "Captured stdout from test binary" },
        "stderr": { "type": "string", "description": "Captured stderr from test binary" },
        "duration_ms": { "type": "integer", "description": "Wall-clock duration in milliseconds" },
        "diagnostics": {
          "type": "array",
          "items": { "$ref": "diagnostic.schema.json" },
          "description": "Compiler diagnostics (on compile_error)"
        },
        "counterexamples": {
          "type": "array",
          "items": { "$ref": "counterexample.schema.json" },
          "description": "ESBMC counterexamples (on verify_failed)"
        }
      }
    },
    "ContractDensity": {
      "type": "object",
      "required": ["functions_total", "functions_with_vows", "density_pct"],
      "properties": {
        "functions_total": { "type": "integer", "description": "Total non-main functions across tested modules" },
        "functions_with_vows": { "type": "integer", "description": "Functions with at least one vow block" },
        "density_pct": { "type": "number", "description": "Percentage of functions with vow contracts" }
      }
    }
  }
}
```

## vow-violation

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/vow-violation.schema.json",
  "title": "VowViolation",
  "description": "Runtime vow violation emitted to stderr (debug mode only). This is emitted by the vow-runtime C code, not by serde.",
  "type": "object",
  "required": ["error", "vow_id", "blame", "description", "file", "offset"],
  "properties": {
    "error": {
      "type": "string",
      "const": "VowViolation",
      "description": "Always the string VowViolation"
    },
    "vow_id": {
      "type": "integer",
      "minimum": 0,
      "description": "Numeric ID of the violated vow"
    },
    "blame": {
      "type": "string",
      "enum": ["Caller", "Callee"],
      "description": "Who is blamed: Caller for requires violations, Callee for ensures/invariant"
    },
    "description": {
      "type": "string",
      "description": "The contract predicate text"
    },
    "file": {
      "type": "string",
      "description": "Source file path"
    },
    "offset": {
      "type": "integer",
      "minimum": 0,
      "description": "Byte offset of the vow in the source file"
    },
    "values": {
      "type": "object",
      "additionalProperties": {
        "type": ["integer", "number", "boolean"]
      },
      "description": "Runtime values of free variables in the predicate (optional, present when bindings exist)"
    }
  },
  "additionalProperties": false
}
```
"##
    .to_string()
}

fn skill_support_files() -> &'static [(&'static str, &'static str)] {
    &[
        (
            r#"reference/grammar.md"#,
            r#"# Vow Grammar Reference

Complete grammar for the Vow programming language. Vow source files use the `.vow` extension.

**Line comments.** `//` starts a line comment extending to end of line. Comments are stripped during lexing and never enter the token stream. Block comments (`/* */`) are not supported. Machine-relevant intent belongs in contracts; comments are for non-semantic rationale.

## Module Declaration

Every file begins with a module declaration:

```
module <Name>
```

`<Name>` is a PascalCase identifier. There is no semicolon.

## Use Declarations

Import other modules with dot-separated paths:

```
use foo.bar
```

This resolves to `<rootdir>/foo/bar.vow` relative to the main source file.

## Const Declarations

Named constants with compile-time values:

```vow
const MAX_SIZE: i64 = 1024;
const NEG_ONE: i64 = -1;
const DEBUG: bool = true;
```

Supported value forms: integer literals, boolean literals, negated integer literals. Constants are inlined at every use site (zero runtime cost). The type must be any of the 10 integer types (`i8`, `i16`, `i32`, `i64`, `i128`, `u8`, `u16`, `u32`, `u64`, `u128`) or `bool`. Integer constants are subject to the same compile-time range check as integer literals. Constants are referenced by name in expressions like any other identifier.

## Functions

### Pure Function

```vow
fn add(x: i64, y: i64) -> i64 {
    x + y
}
```

### Function with Effects

```vow
fn main() -> i32 [io] {
    print_str("hello");
    0
}
```

Effects appear in brackets after the return type: `[io]`, `[read, write]`, `[io, panic]`.

### Function with Vow Block

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

The `vow` block sits between the signature and the body. Clauses:
- `requires: <expr>` — precondition (blame: Caller)
- `ensures: <expr>` — postcondition (blame: Callee); use `result` for the return value
- `invariant: <expr>` — loop invariant (blame: Callee)

Multiple clauses are separated by commas:

```vow
fn clamp(x: i64, lo: i64, hi: i64) -> i64 vow {
    requires: lo <= hi,
    ensures: result >= lo,
    ensures: result <= hi
} {
    if x < lo { lo } else { if x > hi { hi } else { x } }
}
```

### Where Clauses (Refinement Types on Parameters)

```vow
fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}
```

`where` constraints on parameters become additional `requires` in verification. Each `where` clause can only reference its own parameter — it cannot reference other parameters.

### Public Functions

```vow
pub fn api_function(x: i64) -> i64 {
    x
}
```

## Types

### Primitive Types

| Type   | Description              |
|--------|--------------------------|
| `i8`   | 8-bit signed integer     |
| `i16`  | 16-bit signed integer    |
| `i32`  | 32-bit signed integer    |
| `i64`  | 64-bit signed integer    |
| `i128` | 128-bit signed integer (verifier may time out; see below) |
| `u8`   | 8-bit unsigned integer   |
| `u16`  | 16-bit unsigned integer  |
| `u32`  | 32-bit unsigned integer  |
| `u64`  | 64-bit unsigned integer  |
| `u128` | 128-bit unsigned integer (verifier may time out; see below) |
| `f32`  | 32-bit float (limited support — avoid in contracts) |
| `f64`  | 64-bit float (limited support — avoid in contracts) |
| `bool` | Boolean                  |
| `()`   | Unit type                |
| `!`    | Never type (diverges)    |

There is no `isize`/`usize`. Vow targets 64-bit only; `Vec::len()` returns `i64`,
indices are `i64`. This is deliberate — it preserves binary fixed point
reproducibility across compilations. See [ADR 0001](../adr/0001-numeric-tower-narrow-ints.md).

**128-bit verification:** `i128`/`u128` arithmetic codegens via Cranelift's
`I128` and verifies via ESBMC's `__int128`. Predicates over 128-bit values may
exceed reasonable SMT solver timeouts; the `--no-128-verify` flag skips
verification for functions whose contracts mention 128-bit values while still
generating native code for them.

**Struct field layout:** every struct field up to 64 bits wide occupies one
8-byte slot regardless of declared type (narrow ints are padded); `i128`/`u128`
fields occupy two consecutive 8-byte slots (16 bytes). There is no packing or
natural-alignment layout today; FFI structs that need a specific C layout must
shim through `Vec<u8>` or extern wrappers.

### Built-in Parameterized Types

| Type               | Description                     |
|--------------------|---------------------------------|
| `Vec<T>`           | Growable array                  |
| `Option<T>`        | Optional value (Some/None)      |
| `Result<T, E>`     | Success or error                |
| `String`           | UTF-8 string (backed by Vec<u8>)|
| `HashMap<K, V>`    | Key-value map (linear scan)     |
| `BTreeMap<K, V>`   | Sorted key-value map (binary search; ascending iteration). `K` must be `i64`; `V` may be any non-linear type |

### User-Defined Types

Structs and enums (see below).

## Literals

### Integer Literals

```vow
42
-1
0
```

Unsuffixed integer literals default to `i64` in expression position, and
**context-coerce** to any of the 10 integer types when the
surrounding context fixes one — `let` bindings, function arguments, struct
fields, and the typed operand of an arithmetic, bitwise, or comparison
operator. The same coercion applies to constant expressions composed entirely
of unsuffixed integer literals (e.g. `1 + 2`, `1 << 3`, `-5`).

Out-of-range literals in a typed context are a compile-time error:

```vow
let x: u8 = 300;   // error: LiteralOutOfRange — 300 does not fit in u8
let y: i8 = 200;   // error: LiteralOutOfRange — i8 range is -128..=127
```

**Suffixed integer literals** force the type at the literal:

```vow
42u8     42u16     42u32     42u64     42u128
42i8     42i16     42i32     42i64     42i128
```

Suffixed forms are supported for all 10 integer widths. They override context
coercion and are still subject to the same compile-time range check.

### Float Literals

```vow
3.14
-0.5
```

### Boolean Literals

```vow
true
false
```

### String Literals

```vow
"hello, world"
"line one\nline two"
"tab\there"
"null\0byte"
"escaped\\backslash"
"escaped\"quote"
```

Supported escape sequences: `\n`, `\t`, `\r`, `\\`, `\"`, `\0`.

String literals have type `String` and are backed by a read-only static
descriptor. Passing or returning a literal does not allocate. To obtain a
mutable, arena-owned copy, use `String::from("...")`.

## Operators

### Wrapping Arithmetic (default)

| Operator | Meaning        |
|----------|----------------|
| `+`      | Add (wrapping) |
| `-`      | Sub (wrapping) |
| `*`      | Mul (wrapping) |
| `/`      | Div (wrapping) |
| `%`      | Rem (wrapping) |

Wrapping operators silently wrap on overflow. For unsigned operands, including
`u8`, division and remainder use unsigned semantics.

### Checked Arithmetic

| Operator | Meaning           |
|----------|-------------------|
| `+!`     | Add (checked)     |
| `-!`     | Sub (checked)     |
| `*!`     | Mul (checked)     |
| `/!`     | Div (checked)     |
| `%!`     | Rem (checked)     |

Checked operators abort with `ArithmeticOverflow` on overflow.

### Saturating Arithmetic

Saturating arithmetic uses named compiler intrinsics rather than a third
operator family. The `u8` intrinsics are:

| Function | Signature | Behavior |
|----------|-----------|----------|
| `add_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps sums above 255 to 255 |
| `sub_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps differences below 0 to 0 |
| `mul_sat_u8` | `fn(a: u8, b: u8) -> u8` | clamps products above 255 to 255 |

These functions are pure and have direct verifier semantics; they do not
lower to wrapping arithmetic.

### Comparison Operators

| Operator | Meaning                |
|----------|------------------------|
| `==`     | Equal                  |
| `!=`     | Not equal              |
| `<`      | Less than              |
| `<=`     | Less than or equal     |
| `>`      | Greater than           |
| `>=`     | Greater than or equal  |

### Bitwise Operators

| Operator | Meaning      |
|----------|--------------|
| `&`      | Bitwise AND  |
| `\|`     | Bitwise OR   |
| `^`      | Bitwise XOR  |
| `<<`     | Left shift   |
| `>>`     | Right shift  |

Bitwise `& | ^` require integer operands of the same type and work on all 10
integer widths. `>>` is **arithmetic** (sign-extending) for signed types
(`i8`..`i128`) and **logical** (zero-extending) for unsigned types
(`u8`..`u128`).

**Shift count type.** The right operand of `<<` and `>>` is `u32`. Unsuffixed
integer literals on the right side context-coerce to `u32`: given
`let x: u8 = ...`, `x << 3` is well-typed (`3` coerces to `u32`). The left
operand keeps its own integer type; the shift result has the left operand's
type.

**Shift count range.** A const-expression shift count `>= bit-width(LHS)` is a
compile-time error (`ShiftCountOutOfRange`). For example, `(x: u8) << 8` does
not compile. Dynamic shift counts (`x << n` where `n` is not a const
expression) get a contract on the operation that ESBMC checks: the count must
be less than the LHS width at the point of the shift.

Unsuffixed literal coercion still applies for `&`, `|`, `^` operands: with
`let x: u64 = ...`, `3 & x` and `x | 0xff` type-check because the literal
side coerces to `u64`. Use a suffix to force a different type explicitly.

### Logical Operators

| Operator | Meaning    |
|----------|------------|
| `&&`     | Logical AND (short-circuit) |
| `\|\|`   | Logical OR (short-circuit) |
| `!`      | Logical NOT|

`&&` and `||` use short-circuit evaluation: for `a && b`, `b` is only evaluated if `a` is true; for `a || b`, `b` is only evaluated if `a` is false.

### Operator Precedence

From loosest to tightest, Vow follows the usual C/Rust precedence for logical and bitwise operators:

`||`, `&&`, comparisons (`== != < <= > >=`), `|`, `^`, `&`, `<< >>`, `+ -`, `* / %`

Unary `-`, `!`, `&`, and `?` bind tighter than every binary operator.

Single `&` is overloaded by position: prefix `&expr` is borrow, while infix `lhs & rhs` is bitwise AND.

### Unary Operators

| Operator | Meaning    |
|----------|------------|
| `-`      | Negation (not allowed on `u64`) |
| `!`      | Logical NOT|
| `&`      | Borrow     |
| `?`      | Unwrap (propagate error) |

### Type Cast

`as` is **widening-only** across integer types. Any narrower integer can be
cast to any wider integer; signed sources sign-extend, unsigned sources
zero-extend:

```vow
let a: i32 = -1;
let b: i64 = a as i64;     // sign-extend: -1_i64
let c: u8  = 200;
let d: u64 = c as u64;     // zero-extend: 200_u64
let e: u32 = 1;
let f: i64 = e as i64;     // unsigned-to-signed widening, value preserved
```

`as` between signed and unsigned of **the same width** is also allowed
(machine-level bit reinterpretation): `i64 as u64`, `u64 as i64`, `i32 as u32`,
etc.

**Narrowing via `as` is a compile-time error** (`NarrowingCastNotAllowed`):

```vow
let big: i64 = 300;
let small: u8 = big as u8;     // error — narrowing not allowed via `as`
```

To narrow, use a named intrinsic that makes the intent explicit. For every
narrowing pair `(src, tgt)` the compiler exposes three free functions:

| Intrinsic                         | Behavior on out-of-range input          |
|-----------------------------------|-----------------------------------------|
| `<src>_to_<tgt>_try(x) -> Option<tgt>` | returns `Option::None`             |
| `<src>_to_<tgt>_wrap(x) -> tgt`   | truncates (low bits, two's-complement)  |
| `<src>_to_<tgt>_sat(x) -> tgt`    | clamps to the target type's range       |

Example:

```vow
let big: i64 = 300;
match i64_to_u8_try(big) {
    Option::Some(b) => use_byte(b),
    Option::None    => fallback(),
}
```

These intrinsics are emitted by the compiler so ESBMC sees their semantics
directly in the verification C model.

For the `u8` target, the available narrowing source types are `i16`, `i32`,
`i64`, `i128`, `u16`, `u32`, `u64`, and `u128`. Each source provides all three
forms, for example `u16_to_u8_try`, `u16_to_u8_wrap`, and `u16_to_u8_sat`.

No implicit conversions: `i64 + u64` and `u8 + i32` are type errors. The
operands must already have the same type. The compiler does not coerce
across integer types at operator sites — only literals coerce, per the
[Integer Literals](#integer-literals) rules.

## Let Bindings

### Immutable

```vow
let x: i64 = 42;
x = 43;   // error[ImmutableAssignment]: declare it with `let mut x`
```

Bindings are immutable by default. Reassigning a binding that was not declared
`mut` is a compile error (`ImmutableAssignment`). `mut` is required **only** for
whole-binding reassignment `x = e`; field writes (`s.f = e`) and index writes
(`v[i] = e`) are permitted through any binding and do not require the base to be
`mut`.

### Mutable

```vow
let mut i: i64 = 0;
i = i + 1;
```

A `let mut` binding that is never reassigned is a compile error (`UnusedMut`) —
drop the `mut`. Because only whole-binding reassignment counts as a use of `mut`,
a binding mutated solely via `s.f = e`, `v[i] = e`, or a method call should be
declared `let`, not `let mut`.

### Pattern Destructuring

```vow
let (a, b): (i64, i64) = (1, 2);
```

## Control Flow

### If / Else

```vow
if x > 0 {
    x
} else {
    0 - x
}
```

`if`/`else` is an expression — both branches must have the same type. There is no `else if` keyword; nest `if` inside `else`:

```vow
if x < lo {
    lo
} else {
    if x > hi {
        hi
    } else {
        x
    }
}
```

### While Loop

```vow
while i > 0 {
    i = i - 1;
}
```

### While Loop with Invariant

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    v.push(i);
    i = i + 1;
}
```

### For-Each Loop

```vow
for x in vec {
    print_i64(x);
}
```

Iterates over each element of a `Vec<T>`. The loop variable `x` is bound to each element in turn. Desugars to a `while` loop with index arithmetic — zero verification overhead.

### For-Each Loop with Invariant

```vow
for x in vec vow {
    invariant: total >= 0
} {
    total = total + x;
}
```

### Loop (Infinite)

`loop` creates an infinite loop. The expression returns the type of the `break` value:

```vow
let idx: i64 = loop {
    if data[i] == target {
        break i;
    }
    i = i + 1;
    if i >= n { break -1; }
};
```

ESBMC cannot verify unbounded `loop` constructs — use `while` with invariants for verifiable loops.

### Break

`break` exits the innermost loop. Inside `loop`, `break value` sets the loop's result:

```vow
break;           // exit while or loop (loop returns Unit)
break value;     // exit loop with a value (only inside loop, not while)
```

### Continue

`continue` skips the remaining statements in the current loop iteration and jumps back to the loop header:

```vow
continue;        // skip to next iteration of while, loop, or for
```

Inside `while` and `loop`, `continue` emits back-edge values for any mutated variables. Inside `for`, it also advances the loop index.

### Return

```vow
return;
return value;
```

## Struct Definitions

```vow
struct Point {
    x: i64,
    y: i64,
}
```

### Linear Structs

```vow
linear struct FileHandle {
    fd: i64,
}
```

Linear struct values carry a linear obligation. The obligation must either be consumed before the value's owning region closes or transferred to the caller by returning the value.

### Struct Literals

Struct literal names must be PascalCase:

```vow
let p: Point = Point { x: 1, y: 2 };
```

### Field Access

```vow
p.x
```

### Field Assignment

```vow
p.x = 10;
```

### Passing Semantics

Structs are heap-allocated. A struct value is a pointer to a heap region, so passing a struct to a function passes the pointer — the function operates on the same heap data, not a copy. Field assignments inside the called function are visible to the caller:

```vow
fn shift_right(p: Point, dx: i64) {
    p.x = p.x + dx;
}

fn main() -> i32 [io] {
    let p: Point = Point { x: 0, y: 0 };
    shift_right(p, 5);
    print_i64(p.x);  // 5 — mutation visible to caller
    0
}
```

This enables in-place mutation patterns (e.g., make/unmake in search trees) without cloning. The same aliasing semantics apply when structs are stored in containers — see [Indexing](#indexing). To avoid aliasing, construct a fresh struct literal with the desired field values.

**Note:** For `linear struct` types, passing the value to a function consumes it; the caller cannot access it afterward. Returning a linear value transfers the obligation to the caller, so this is the normal way to hand an updated linear value back out of a function.

## Enum Definitions

```vow
enum Shape {
    Circle(i64),
    Rect(i64, i64),
    Empty,
}
```

Variant kinds: unit (`Empty`), tuple (`Circle(i64)`), struct (`Named { x: i64 }`).

### Enum Construction

```vow
let s: Shape = Shape::Circle(5);
let none: Option<i64> = Option::None;
let some: Option<i64> = Option::Some(42);
```

### Built-in Enums

`Option<T>` has variants `Some(T)` and `None`.
`Result<T, E>` has variants `Ok(T)` and `Err(E)`.

## Pattern Matching

```vow
match value {
    Pattern1 => expr1,
    Pattern2 => expr2,
    _ => default_expr,
}
```

Match is an expression. The scrutinee must have an enum type, including an
applied built-in enum such as `Option<T>` or `Result<T, E>`. All arms must
return the same type. Patterns must be exhaustive.

### Pattern Kinds

| Implemented pattern                         | Example              |
|---------------------------------------------|----------------------|
| Wildcard                                    | `_`                  |
| Immutable identifier binding                | `value`              |
| Qualified enum variant (unit)               | `Option::None`       |
| Qualified enum variant (tuple payload)      | `Option::Some(value)` |

Tuple-variant payloads may contain only `_` or immutable identifier bindings.
Nested payload destructuring is not implemented. A catchall `_` or immutable
identifier arm must be the final arm because it matches every enum value.

Mutable identifier, literal (integer, boolean, or string), tuple, struct,
enum-struct, or-pattern, unqualified enum-variant, and nested payload patterns
are not implemented. Parsed unsupported forms produce
`error[UnsupportedPattern]`; forms that the parser cannot represent produce
`error[UnexpectedToken]`. Both are compile-time failures and no executable is
produced.

## Method Calls

```vow
v.push(42);
v.len()
s.byte_at(0)
m.contains_key(k)
```

### Vec<T> Methods

| Method         | Signature                        |
|----------------|----------------------------------|
| `Vec::new()`   | `() -> Vec<T>`                   |
| `Vec::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> Vec<T>` for flat scalar `T` |
| `.push(val)`   | `(T) -> ()`                      |
| `.pop()`       | `() -> ()`                       |
| `.len()`       | `() -> i64`                      |
| `.clear()`     | `() -> ()` — frees buffer, resets to empty |
| `.truncate(n)` | `(i64) -> ()` — shrinks to n elements, frees excess memory |
| `v[i]`         | Index read — copies slot value; aliases heap types (panics if out of bounds) |
| `v[i] = val`   | Index write — copies value into slot |

### String Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `String::from(s)`   | `(String) -> String` — mutable copy |
| `String::new()`     | `() -> String`              |
| `String::from_raw_parts_copy(ptr, len)` | `(i64, i64) -> String` |
| `.len()`            | `() -> i64`                 |
| `.byte_at(i)`       | `(i64) -> i64`              |
| `.push_byte(b)`     | `(i64) -> ()`               |
| `.push_str(s)`      | `(String) -> ()`            |
| `.clear()`          | `() -> ()` — frees buffer, resets to empty |
| `.contains(s)`      | `(String) -> bool`          |
| `.eq(s)`            | `(String) -> bool`          |
| `.substring(start, end)` | `(i64, i64) -> String` |
| `.parse_i64()`      | `() -> Option<i64>`         |
| `.parse_u64()`      | `() -> Option<u64>`         |

### HashMap<K, V> Methods

| Method              | Signature                   |
|---------------------|-----------------------------|
| `HashMap::new()`    | `() -> HashMap<K, V>`       |
| `.insert(k, v)`     | `(K, V) -> ()`              |
| `.get(k)`           | `(K) -> V`                  |
| `.contains_key(k)`  | `(K) -> bool`               |
| `.remove(k)`        | `(K) -> ()`                 |
| `.len()`            | `() -> i64`                 |

### BTreeMap<K, V> Methods

Keys must be `i64` (K violations raise `BTreeMapKeyTypeMustBeI64`). Values may be any
non-linear type — primitives, structs, `Vec<T>`, `Option<T>`, or nested combinations.
A `V` that is or transitively contains a `linear struct` is rejected with
`BTreeMapValueMustBeNonLinear`, because the runtime/verifier shift values bitwise and
would silently duplicate a linear obligation.
Storage is two parallel sorted arrays (binary-search lookup, sorted-insert writes).
Iteration order is ascending by key and is **deterministic across runs and compilers** —
prefer `BTreeMap` over `HashMap` for any map whose iteration affects compiler output.

| Method              | Signature                   |
|---------------------|-----------------------------|
| `BTreeMap::new()`   | `() -> BTreeMap<K, V>`      |
| `.insert(k, v)`     | `(K, V) -> Option<V>` (returns the previous value bound to `k`, if any) |
| `.get(k)`           | `(K) -> Option<V>` (returns the value bound to `k`, or `None`)          |
| `.contains(k)`      | `(K) -> bool`               |
| `.len()`            | `() -> i64`                 |

### Option<T> Methods

| Method      | Signature                              |
|-------------|----------------------------------------|
| `.unwrap()` | `() -> T` (panics on None; requires `[panic]` effect) |

The `?` operator on `Option<T>` or `Result<T, E>` propagates `None`/`Err` to the caller (the calling function must return `Option` or `Result`).

## Indexing

```vow
let val: i64 = v[0];
v[i] = new_val;
```

Indexing uses **copy semantics**: `v[i]` copies the 8-byte slot value and `v[i] = val` copies a value into the slot. The base container is not consumed.

For primitive types (`i64`, `bool`), this is a genuine value copy — the result is independent of the container. For heap types (`Vec<T>`, `String`, structs, enums), the 8-byte slot holds a pointer, so indexing copies the pointer, creating an **alias**. Both the container slot and the local variable point to the same heap data:

```vow
let buckets: Vec<Vec<i64>> = Vec::new();
buckets.push(Vec::new());
let b: Vec<i64> = buckets[0];  // b aliases buckets[0]
b.push(42);                     // visible through buckets[0]
```

This aliasing is the intended behavior for arena and hash-table patterns where bucket contents are read and mutated repeatedly through index access.

## Extern Blocks

Declare external C functions (a `vow` contract block is required):

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

Omitting the `vow` block produces a `MissingContract` error (see [errors.md](errors.md)).

## Type Aliases

```vow
type Score = i64
```

## Effect System

Effects are explicit. Every function declares which side effects it may perform. Pure functions (no effects) need no annotation.

### Effect Types

| Effect   | Meaning                              |
|----------|--------------------------------------|
| `io`     | Standard I/O (print, stdin, network) |
| `read`   | File system reads                    |
| `write`  | File system writes                   |
| `panic`  | May panic (unwrap, etc.)             |
| `unsafe` | Unsafe operations (FFI, raw memory)  |

Each effect is independent — `io` is not a superset of `read` or `write`.

### Propagation

A function must declare every effect that any function it calls may produce:

```vow
fn do_io() -> () [io] {
    print_str("hi");
}

fn caller() -> () [io] {
    do_io();
}
```

If `caller` omitted `[io]`, the type checker would emit `EffectViolation`.

### Contract Purity

Contract expressions (`requires`, `ensures`, `invariant`) must be pure — they cannot call effectful functions.

### Builtin Function Signatures

#### FFI Wrapper Intrinsics

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `pin_to_root`    | `fn(value: String) -> String` and `fn<T>(value: Vec<T>) -> Vec<T>` for flat scalar `T` | `[]` |

`pin_to_root` is a compiler intrinsic, not a user-defined generic. Each call site is monomorphised from the argument type. It always deep-copies the supported heap value into root storage; it does not inspect descriptor tags and does not claim idempotency. The current supported forms are `String` and `Vec<T>` where `T` is a flat scalar slot type (`i*`, `u*`, `f32`, `f64`, `bool`). Pointer-containing payloads, user structs, enums, and maps require hand-written deep-copy wrappers at the FFI boundary.

`String::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` bytes from a raw C pointer into a fresh `String`. `Vec::from_raw_parts_copy(ptr: i64, len: i64)` copies `len` flat scalar slots into a fresh `Vec<T>`. The surface length type is `i64`; the code generator converts pointer and length values to the platform pointer-sized ABI type at the FFI boundary. Both helpers have a `FreshInCaller` return summary.

For pointer-containing C payloads, a wrapper must be written per type: call the extern, recursively copy every Vow-owned heap subobject into the target region, free every C-owned pointer according to the extern's ownership contract, then return the Vow-placed value. A bytewise copy of a pointer-containing payload is unsound because it preserves stale pointers into C-owned storage.

#### Print / IO

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `print_str`      | `fn(s: String) -> ()`                      | `[io]`     |
| `print_i64`      | `fn(v: i64) -> ()`                         | `[io]`     |
| `print_u64`      | `fn(v: u64) -> ()`                         | `[io]`     |
| `eprintln_str`   | `fn(s: String) -> ()`                      | `[io]`     |

#### Debug

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `debug_str`      | `fn(s: String) -> ()`                      | `[]`       |
| `debug_i64`      | `fn(v: i64) -> ()`                         | `[]`       |
| `debug_u64`      | `fn(v: u64) -> ()`                         | `[]`       |

**Debug print semantics:** Debug prints are effect-free and callable from pure functions. In debug and sanitize modes (`--mode debug`, `--mode sanitize`), they write to stderr. In release and profile modes, the debug call itself is not emitted — no function call occurs. However, argument expressions are still evaluated (a direct literal such as `"label"` is static, while `String::from("label")` still allocates a mutable copy). They are also no-ops during verification. Use them to trace values inside pure kernel code without restructuring the effect hierarchy.

#### Filesystem

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `fs_read`        | `fn(path: String) -> String`               | `[read]`   |
| `fs_open`        | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_read_line`   | `fn(handle: i64) -> String`                | `[read]`   |
| `fs_status`      | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_close`       | `fn(handle: i64) -> i64`                   | `[read]`   |
| `fs_write`       | `fn(path: String, data: String) -> i64`    | `[write]`  |
| `fs_exists`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_mkdir`       | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_listdir`     | `fn(path: String) -> Vec<String>`          | `[read]`   |
| `fs_remove`      | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_remove_dir`  | `fn(path: String) -> i64`                  | `[io]`     |
| `fs_is_dir`      | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_is_symlink`  | `fn(path: String) -> i64`                  | `[read]`   |
| `fs_rename`      | `fn(old: String, new: String) -> i64`      | `[io]`     |

#### String Operations

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `string_substr`       | `fn(s: String, start: i64, len: i64) -> String`  | `[]`    |
| `string_split`        | `fn(s: String, delim: String) -> Vec<String>`    | `[]`    |
| `string_starts_with`  | `fn(s: String, prefix: String) -> i64`           | `[]`    |
| `string_ends_with`    | `fn(s: String, suffix: String) -> i64`           | `[]`    |
| `string_matches_literal_at` | `fn(s: String, pos: i64, literal: String literal) -> i64` | `[]` |
| `string_trim`         | `fn(s: String) -> String`                        | `[]`    |
| `string_to_upper`     | `fn(s: String) -> String`                        | `[]`    |
| `string_to_lower`     | `fn(s: String) -> String`                        | `[]`    |
| `string_replace`      | `fn(s: String, from: String, to: String) -> String` | `[]` |
| `string_join`         | `fn(parts: Vec<String>, sep: String) -> String`  | `[]`    |

#### Conversion

**Formatting** uses two baselines; widen via `as` for narrower types:

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `int_to_string`  | `fn(v: i64) -> String`                     | `[]`       |
| `uint_to_string` | `fn(v: u64) -> String`                     | `[]`       |
| `i64_to_string`  | `fn(v: i64) -> String` (alias of `int_to_string`) | `[]` |

```vow
let small: u8 = 42;
print_str(uint_to_string(small as u64));  // widen then format
```

**Parsing** exposes a try-form for every integer width:

| Function       | Signature                                |
|----------------|------------------------------------------|
| `parse_i8`     | `fn(s: String) -> Option<i8>`            |
| `parse_i16`    | `fn(s: String) -> Option<i16>`           |
| `parse_i32`    | `fn(s: String) -> Option<i32>`           |
| `parse_i64`    | `fn(s: String) -> Option<i64>` (also see `String.parse_i64()`) |
| `parse_i128`   | `fn(s: String) -> Option<i128>`          |
| `parse_u8`     | `fn(s: String) -> Option<u8>`            |
| `parse_u16`    | `fn(s: String) -> Option<u16>`           |
| `parse_u32`    | `fn(s: String) -> Option<u32>`           |
| `parse_u64`    | `fn(s: String) -> Option<u64>` (also see `String.parse_u64()`) |
| `parse_u128`   | `fn(s: String) -> Option<u128>`          |

Each `parse_X` returns `Option::None` for malformed input, empty strings, or
values outside the target type's range.

In particular, `parse_u8` accepts decimal values from `0` through `255` and
returns `Option::None` for negative or larger values.

**Narrowing intrinsics** (per [Type Cast](#type-cast)): for every narrowing
pair the compiler emits `<src>_to_<tgt>_try`, `<src>_to_<tgt>_wrap`, and
`<src>_to_<tgt>_sat` free functions with the semantics described in that
section.

#### Collections

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `vec_sort`       | `fn(v: Vec<i64>) -> Vec<i64>`              | `[]`       |

#### Time

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `time_unix`      | `fn() -> i64`                              | `[io]`     |
| `time_unix_ms`   | `fn() -> i64`                              | `[io]`     |

#### System

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `num_cpus`       | `fn() -> i64`                              | `[io]`     |
| `memory_root_arena_bytes` | `fn() -> u64`                    | `[io]`     |
| `memory_peak_bytes` | `fn() -> u64`                           | `[io]`     |
| `memory_alloc_count_since_start` | `fn() -> u64`              | `[io]`     |

`num_cpus()` returns the number of available logical CPUs (from `std::thread::available_parallelism`), or `1` if the query fails. Used to size worker pools (e.g. the default `--verify-jobs` value).

`memory_root_arena_bytes()` returns the current bytes retained by root-region arena chunks. `memory_peak_bytes()` returns the peak live bytes retained by all open arena chunks since process start. `memory_alloc_count_since_start()` returns the number of successful Vow arena allocation requests since process start. These queries do not allocate; they are effectful because they observe runtime process state.

#### Encoding

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `hex_encode`     | `fn(data: Vec<u8>) -> String`              | `[]`       |
| `hex_decode`     | `fn(s: String) -> Vec<u8>`                 | `[]`       |

#### Input

| Function         | Signature                                  | Effects    |
|------------------|--------------------------------------------|------------|
| `args`           | `fn() -> Vec<String>`                      | `[read]`   |
| `stdin_read`     | `fn() -> String`                           | `[read]`   |
| `stdin_read_line`| `fn() -> String`                           | `[read]`   |
| `stdin_ready`    | `fn() -> bool`                             | `[read]`   |

#### Process Management

| Function              | Signature                                        | Effects |
|-----------------------|--------------------------------------------------|---------|
| `process_exit`        | `fn(code: i64) -> !`                             | `[io]`  |
| `process_run`         | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_get_stdout`  | `fn() -> String`                                 | `[io]`  |
| `process_get_stderr`  | `fn() -> String`                                 | `[io]`  |
| `process_start`       | `fn(cmd: String, args: Vec<String>) -> i64`      | `[io]`  |
| `process_wait`        | `fn(pid: i64) -> i64`                            | `[io]`  |
| `process_wait_timeout`| `fn(pid: i64, timeout_ms: i64) -> i64`           | `[io]`  |
| `process_kill`        | `fn(pid: i64) -> i64`                             | `[io]`  |
| `process_stdout_for`  | `fn(pid: i64) -> String`                         | `[io]`  |
| `process_stderr_for`  | `fn(pid: i64) -> String`                         | `[io]`  |

**`args` semantics:** `args()` returns all process arguments including the program name at index 0 (matching C `argv` and Rust `std::env::args()` conventions). For `./my_program foo bar`, `args()` returns `["./my_program", "foo", "bar"]`. Use `args[1]` onward for user-supplied arguments. The Vec is empty only if the OS provides no arguments (unusual). Returns an empty String element if an argument is empty (`""`). Non-UTF-8 arguments are included as-is (byte content preserved).

**`fs_read` semantics:** `fs_read(path)` opens the file at `path`, reads its entire contents, and returns a String. Returns `""` (empty String) on any error (file not found, permission denied, I/O error, non-UTF-8 path). Does not block on regular files. Callers should check `result.len() == 0` to detect failure.

**Streaming file input:** `fs_open(path)` opens a file for incremental reading and returns a positive handle, or `-1` on path/open error. `fs_read_line(handle)` reads one line from the current cursor and returns it as a String, including the trailing newline when present. It returns `""` at EOF, for an invalid handle, or after a read error. A blank line is returned as `"\n"`, so newline-delimited callers can distinguish a real blank line from EOF by content. After `fs_read_line(handle)` returns `""`, call `fs_status(handle)` to distinguish EOF from error: `0` means the handle is open with no EOF/error state, `1` means EOF, and `-1` means invalid handle or read error. `fs_status(handle)` reports the result of the most recent `fs_read_line(handle)` call on that open handle; read it immediately after a `""` return because later reads may update it. `fs_close(handle)` releases the handle and returns `0` on success or `-1` for an invalid/already-closed handle. Long-running programs must close handles they no longer need. All streaming handle operations use the `[read]` effect, including `fs_close`, because closing a read handle releases read-stream state and does not mutate filesystem contents. The current runtime stores streaming handles in one process-global table, and `fs_read_line` holds that table lock while it reads the next line. This keeps the API simple for single-stream file processing, but it is not intended for latency-sensitive concurrent reads from multiple slow handles.

**Filesystem return values:** `fs_write`, `fs_mkdir`, `fs_remove`, `fs_remove_dir`, and `fs_rename` return `i64`: 0 on success, non-zero on failure. `fs_open`, `fs_status`, and `fs_close` use the streaming status codes above. `fs_exists`, `fs_is_dir`, and `fs_is_symlink` are predicates: they return 1 for true, 0 for false. Errors (null pointer, invalid UTF-8) also return 0, so callers cannot distinguish "false" from "error". `fs_is_symlink` uses `lstat`-equivalent semantics: a symlink reports 1 even when its target is a regular file or directory.

**`string_starts_with` / `string_ends_with` / `string_matches_literal_at` return values:** Return `i64`: 1 if true, 0 if false.

**`string_matches_literal_at` literal operand:** The third argument must be written as a string literal at the call site. The compiler lowers that literal to static bytes plus an explicit byte length, so no temporary `String` allocation is created and embedded NUL bytes are preserved. Passing a variable or computed `String` as the third argument is a type-check error (`StaticLiteralRequired`). Use `string_starts_with`, `string_ends_with`, or `String` methods when the needle must be dynamic.

**`process_run` vs `process_start`:** `process_run(cmd, args)` runs a subprocess synchronously and returns its exit code. After it returns, `process_get_stdout()` and `process_get_stderr()` retrieve the captured output of the most recent `process_run` call. `process_start(cmd, args)` launches a subprocess asynchronously and returns a process ID. Use `process_wait(pid)` to wait for completion and get the exit code, and `process_stdout_for(pid)` / `process_stderr_for(pid)` to retrieve output.

**`process_wait_timeout`:** `process_wait_timeout(pid, timeout_ms)` polls a process started with `process_start` until it exits or the timeout (in milliseconds) elapses. Returns the exit code on completion, `-1` on error, or `-2` on timeout. After a timeout, the process is still running; use `process_kill(pid)` to terminate it.

**`process_kill`:** `process_kill(pid)` sends a kill signal to a running process and waits for it to exit. Returns 0 on success, -1 on error. No-op (returns 0) if the process has already completed.

**`stdin_read` vs `stdin_read_line`:** `stdin_read()` reads the entire stdin stream into a single String (unbounded memory). `stdin_read_line()` reads one line at a time, including the trailing newline. Returns `""` (empty string) at EOF. The returned String is runtime scratch storage valid until the next `stdin_read_line()` call. Process each line before reading the next one for bounded memory; use `pin_to_root(line)` before the next read when a line must be stored, returned, passed to a function that may store it, mutated, or otherwise retained. The direct scratch line is read-only. The scratch buffer keeps the largest line capacity seen so far, so memory is bounded by maximum line length rather than total input, but one very large line can retain that capacity for the process lifetime.

```vow
let lines: Vec<String> = Vec::new();
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // Without pin_to_root, lines.push(line) would store the scratch alias, not a copy.
    lines.push(pin_to_root(line));
    line = stdin_read_line();
}
```

```vow
let mut line: String = stdin_read_line();
while str_len(line) > 0 {
    // process line (has trailing \n)
    line = stdin_read_line();
}
```

**`stdin_ready`:** `stdin_ready()` returns `true` if `stdin_read_line()` would return immediately without blocking, `false` otherwise. Uses a non-blocking poll with zero timeout. Use this in computation loops that must remain responsive to external input:

```vow
while !stdin_ready() && depth < max_depth {
    // continue searching
    depth = depth + 1;
}
if stdin_ready() {
    let cmd: String = stdin_read_line();
    // handle command
}
```

## Canonical Form

The canonical printer normalizes source: `parse → print → parse` is idempotent. Effects are sorted alphabetically, indentation uses 4 spaces, trailing expressions omit semicolons.
"#,
        ),
        (
            r#"reference/cli.md"#,
            r#"# Vow CLI Reference

## Commands

### `vow build` (default)

Compile source to native executable. Verifies contracts by default.

```
vow build [OPTIONS] <source.vow>
vow [OPTIONS] <source.vow>          # legacy (equivalent)
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `build/<stem>` | Output executable path                  |
| `--mode <debug\|release\|profile\|sanitize>` | `release` | Build mode: debug inserts runtime vow checks, profile inserts call counters and prints report on normal exit, sanitize adds debug checks + Vec provenance tracking |
| `--no-verify`     | (off)       | Skip ESBMC static verification            |
| `--dump-ir`       | (off)       | Print IR text to stdout and exit (no JSON output, no codegen) |
| `--debug-trace <off\|calls\|full>` | `off` | Emit JSON trace lines to stderr at runtime |
| `--no-cache`    | (off)       | Disable verification result caching, and (for `--no-verify` builds) the compile-object cache. See "Compile-object cache behavior" below |
| `--max-k-step <N>` | `50`     | ESBMC incremental BMC max iterations          |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |
| `--replay-cex`    | (off)       | Differential test the verifier model against runtime semantics (same as `vow verify --replay-cex`; see below) |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this compilation to `<path>` (load directly at ui.perfetto.dev). Captures per-phase spans, codegen/link, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Distinct from `--mode profile`, which instruments the *compiled program*. Pure side artifact: never affects codegen, the build JSON, or the cache. |

**Compile-object cache behavior.** The on-disk compile-object cache (`$VOW_CACHE_DIR` or `~/.cache/vow/`, where each entry is a `<key>.o` artifact keyed by a content hash of all dependencies, mode, and trace settings) is automatically disabled whenever ESBMC verification is active. This guarantees the linked binary always comes from the same codegen run whose IR was verified, closing the integrity gap where a stale or attacker-supplied `.o` could be linked against freshly-verified IR. Concretely the cache only activates on `vow build --no-verify` invocations; it is bypassed on the default `vow build` path. `--no-cache` additionally disables the cache for `--no-verify` builds.

### `vow verify`

Verify contracts only — no executable output. Emits the same JSON format as `vow build` but `executable` is always `null`.

```
vow verify [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver; auto selects per-function via heuristic |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode: bv (bit-vector) or ir (integer/real arithmetic); ir requires z3 |
| `--timeout <N>` | `300` (or `30` when `--encoding` is `auto`) | ESBMC per-function timeout in seconds. Under `--encoding auto`, a 30s default is applied so the BV-timeout fallback to `--encoding ir --solver z3` can trigger when bit-vector solving takes too long. With explicit encodings, a 300s safety watchdog bounds the run; explicit `--timeout` overrides both. `--timeout 0` is honoured as an immediate watchdog kill |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs |
| `--replay-cex`    | (off)       | Differential test of the verifier model against runtime semantics. After ESBMC reports a counterexample, build a `--mode debug` harness that calls the failing function with the counterexample's concrete inputs and check that the runtime `VowViolation` agrees (same `vow_id` and blame). Adds a `replay` field to each counterexample (see "Counterexample replay" below). Opt-in, off by default; also accepted by `vow build`. |
| `--perfetto <path>` | (off) | Write a gzipped Chrome Trace Event Format trace of this verification run to `<path>` (load directly at ui.perfetto.dev). Captures frontend phase spans, per-function ESBMC proof spans, the compiler→ESBMC handoff, and time-series CPU/RSS for the compiler and each ESBMC process. Pure side artifact. |

### `vow contracts`

List all contracts (requires, ensures, invariant) in a program. Runs frontend only by default (no codegen, no verification).

```
vow contracts [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `--verify`        | (off)       | Run ESBMC verification and report per-contract status |
| `--no-cache`      | (off)       | Disable verification result caching        |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations       |
| `--solver <boolector\|z3\|bitwuzla\|auto>` | `auto` | ESBMC SMT solver (with --verify)           |
| `--encoding <bv\|ir\|auto>` | `auto` | ESBMC encoding mode (with --verify); ir requires z3 |
| `--verify-jobs <N>` | `num_cpus/2` | Accepted for CLI parity with build/verify/test; currently a no-op (the contracts verifier is serial) |

**Exit code.** With `--verify`, `vow contracts` fails closed exactly like `vow build --verify` and `vow verify`: it exits **1** if any contract's `status` is not proven — i.e. any `failed`, `timeout`, `unknown`, `error`, `skipped`, or `vacuous` — and **0** only when every contract is `proven`/`proven-ir`. Without `--verify` every contract is `not_verified` and the command exits 0. (This is independent of the static `quality` classification, which never affects the exit code.)

When `--verify` is requested but ESBMC is not installed, the command still emits the full contracts-result JSON schema with every entry's `status` set to `error` and exits with code 1 (fail-closed). Install ESBMC, or omit `--verify`, to obtain proven/failed/unknown statuses.

### `vow skill`

Generate or install the Claude Code skill document for the current compiler version. The skill is embedded in the compiler binary, ensuring the documentation always matches the installed toolchain.

```
vow skill              # print skill document to stdout (default: print)
vow skill print        # print concise Claude Code SKILL.md entrypoint
vow skill print --bundle  # print self-contained bundle for raw API harnesses
vow skill install      # prompt for local or global install target
vow skill install --local   # install to ./.claude/skills/vow/
vow skill install --global  # install to $HOME/.claude/skills/vow/ on Linux
```

`print` writes the concise installed `SKILL.md` entrypoint (with YAML frontmatter) to stdout. `print --bundle` writes a complete self-contained skill document to stdout for non–Claude Code harnesses that cannot load supporting files.

`install` writes `SKILL.md` plus supporting files under `reference/`, `examples/`, and `schemas/`. Claude Code discovers the skill from the `.claude/skills/` directory and uses the frontmatter description/`when_to_use` metadata to load it for `.vow` file work as well as creation and verification-debugging prompts before a `.vow` file exists.

When no scope flag is provided, `install` prompts on stderr for local (`./.claude`) or global (`$HOME/.claude`) installation. Scripts and agents should pass `--local` or `--global` explicitly. `--local` requires the current directory to contain both `.git` and `.claude/`; otherwise it exits with an error and writes nothing. `--global` installs under `$HOME/.claude/skills/vow/` and fails if `$HOME` is unset or empty.

**Auto-install on build.** The first time `vow build` (or the legacy `vow <source.vow>` form) runs in a directory that already contains a `.claude/` subtree but no `.claude/skills/vow/SKILL.md`, the compiler installs the skill silently. This bootstraps Claude Code projects without requiring an explicit `vow skill install`. Unlike explicit `--local`, auto-install only requires `.claude/`; it does not require the directory to be a git checkout. Auto-install is skipped when `.claude/` does not exist (so it never pollutes non–Claude Code projects) and when the skill file is already present (so user edits are never overwritten). Auto-install never fails the build.

### `vow test`

Discover, compile, run, and report on Vow test files. Tests are normal `.vow` programs with `main() -> i32` — no test-specific syntax.

```
vow test [OPTIONS] [<path>]
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `<path>`          | `.`         | Directory to scan or single `.vow` file    |
| `--verify`        | (off)       | Run ESBMC verification on test files       |
| `--filter <pat>`  | (none)      | Only run tests whose file stem contains pat |
| `--module-root <path>` | (auto)  | Resolve `use` declarations against `<path>`. Defaults to the scan path when it's a directory, otherwise the entry file's parent directory. |
| `--mode debug`    | (default)   | Insert runtime vow checks                 |
| `--mode release`  | `debug`     | Omit all vow checks for performance       |
| `--timeout <ms>`  | `30000`     | Per-test execution timeout in milliseconds |
| `--max-k-step <N>` | `50`       | ESBMC incremental BMC max iterations (with --verify) |
| `--verify-jobs <N>` | `num_cpus/2` | Max concurrent ESBMC verification jobs (with --verify) |

Test discovery: files matching `test_*.vow` or `*_test.vow` under the given directory **and its subdirectories**, sorted alphabetically. Each test must contain `main() -> i32` returning 0 on success.

**Module resolution for directory scans.** When `<path>` is a directory, every discovered test resolves its `use` declarations against `<path>` rather than the test file's own parent directory. This lets internal-unit tests live in a subdirectory like `compiler/tests/test_region.vow` and still `use region;` to import the module under test (which lives at `compiler/region.vow`). Single-file invocations (`vow test path/to/test_foo.vow`) keep the default behaviour of resolving `use` against the file's parent directory.

**Test Output JSON:**

```json
{
  "status": "TestsPassed",
  "total": 3,
  "passed": 3,
  "failed": 0,
  "skipped": 0,
  "tests": [
    {
      "file": "compiler/test_arith.vow",
      "name": "test_arith",
      "status": "passed",
      "exit_code": 0,
      "stdout": "7",
      "stderr": "",
      "duration_ms": 72,
      "diagnostics": [],
      "counterexamples": []
    }
  ],
  "contract_density": {
    "functions_total": 1,
    "functions_with_vows": 0,
    "density_pct": 0.0
  }
}
```

| Status Field   | Meaning                                           |
|----------------|---------------------------------------------------|
| `TestsPassed`  | All tests passed                                  |
| `TestsFailed`  | One or more tests failed                          |

Per-test status: `passed`, `failed`, `timeout`, `compile_error`, `verify_failed`, `contract_skipped`, `skipped`.

`contract_skipped` means ESBMC was never invoked because a vowed function is non-modelable (distinct from `verify_failed`, where ESBMC proved a violation). Both are fail-closed — a `contract_skipped` test counts toward `failed` and yields a `TestsFailed` overall status.

### `vow decl`

Emit declaration file output only.

```
vow decl [OPTIONS] <source.vow>
```

**Options:**

| Flag              | Default     | Description                                |
|-------------------|-------------|--------------------------------------------|
| `-o, --output`    | `<source>.vow.d` | Output declaration file path          |

### `vow mutants` (self-hosted only)

Run mutation testing on a Vow source tree. Implemented in the self-hosted compiler only; the Rust bootstrap compiler emits an error pointing the user to `build/vowc`. See `docs/mutants.md` for full details on output schema, mutation kinds, skip-list, and known limitations.

```
vowc mutants version
vowc mutants list  [--root DIR] [--shard X/Y]
vowc mutants run   [--root DIR] [--shard X/Y]
                   [--tier1-cmd 'cmd'] [--tier2-cmd 'cmd']
                   [--tier1-timeout-secs N] [--tier2-timeout-secs N]
                   [--tier2-budget-secs N]
                   [--workdir DIR] [--output-dir DIR] [--force-unlock]
```

| Flag | Default | Notes |
|---|---|---|
| `--root` | `compiler` | Directory whose `*.vow` files are mutated. `test_*.vow` files are excluded. |
| `--shard X/Y` | `0/1` | Round-robin split of the deterministic mutant ID space. Mutant `id` is selected iff `id % Y == X`. |
| `--tier1-cmd` | `scripts/bootstrap.sh --skip-cargo` | Fast oracle. Anything but exit 0 = caught at Tier 1. |
| `--tier2-cmd` | `scripts/full_test.sh` | Full oracle. Only run on Tier-1 survivors. |
| `--tier1-timeout-secs` | `180` | Per-mutant Tier-1 wall-clock cap. |
| `--tier2-timeout-secs` | `3600` | Per-mutant Tier-2 wall-clock cap. |
| `--tier2-budget-secs` | `7200` | Per-shard total Tier-2 budget. Once exhausted, surviving Tier-1 mutants are emitted with `status:"unrun"`. |
| `--workdir` | `/tmp/vow-mutants-<ms>` | Path of the throwaway `git worktree` used for all mutations. |
| `--output-dir` | `mutants.out` | Directory for `mutants.json`, `outcomes.json`, status text files, `diff/`, `logs/`. |
| `--force-unlock` | off | Remove a stale `output_dir/.lock` before starting. |

Output schemas: see `docs/spec/schemas/mutants-result.schema.json`.

### `vow complexity`

Report per-function complexity metrics as deterministic, **byte-identical** JSON (the Rust and self-hosted compilers produce identical output). Every structural metric sits next to its size; the single 0–100 `complexity_score` is a readability / refactor-priority gate — explicitly **not** a defect predictor. The component vector, not the scalar, is the source of truth; gating on the scalar alone is opt-in and discouraged as the sole signal.

```
vow complexity <source.vow>
               [--cog-anchor N] [--nloc-anchor N]
               [--max-score N] [--max-cognitive N] [--max-cyclomatic N]
```

| Flag | Default | Notes |
|---|---|---|
| `--cog-anchor <N>` | `15` | Cognitive-complexity value mapped to sub-score `0.800` (SonarQube's default flag line). |
| `--nloc-anchor <N>` | `60` | NLOC value mapped to sub-score `0.800` (~50–60 line guidance). |
| `--max-score <N>` | (unset) | CI gate: exit nonzero if any function's `complexity_score` exceeds N. The recommended line is 80, but gating is opt-in only. |
| `--max-cognitive <N>` | (unset) | CI gate: exit nonzero if any function's `cognitive` exceeds N. |
| `--max-cyclomatic <N>` | (unset) | CI gate: exit nonzero if any function's `cyclomatic` exceeds N. |

**Exit code.** Nonzero on frontend/read failures, malformed numeric flags, or when a `--max-*` threshold is passed and exceeded. With no `--max-*` flag the command is pure reporting once the input is readable and valid — no threshold gates by default (per the decouple-language-from-prover principle).

**Numeric convention.** The non-integer metrics (`halstead.volume`/`difficulty`/`effort` and `score_factors.*`) are emitted as fixed-3-decimal JSON numbers computed in **integer fixed-point** (scale 1000) — never native floats — so both compilers stay byte-identical. `complexity_score` is an integer in `[0, 100]`. The score's saturating anchor map uses a rational curve (`0.800` at the anchor, asymptoting to `1.000`), not an exponential, because the self-hosted compiler has no floating point.

**Contract identifier convention.** `vow.contract.free_vars` counts distinct value identifiers referenced by clause predicates. It excludes the `result` binding, function callee identifiers, and method names; receiver and argument expressions still count when they are values.

Output schema: see `docs/spec/schemas/complexity-result.schema.json`.

### `vow --help`

`vow --help` is agent-first. It emits versioned JSON capability data for the tool, command set,
language surface, result schemas, and implementation status. `--help --human` exists only as a
legacy compatibility mode and is not the canonical interface.

```
vow --help               # versioned JSON tool-help protocol
vow --help --human       # legacy compatibility text
vow build --help         # same JSON (works on all subcommands)
vow verify --help --human  # same legacy text (works on all subcommands)
```

## Exit Codes

| Code | Meaning                                                                            |
|------|------------------------------------------------------------------------------------|
| `0`  | Success (`Verified` or `Unverified`)                                               |
| `1`  | Failure (`CompileFailed`, `VerifyFailed`, or `Skipped`)                            |

`vow build` and `vow verify` both fail closed on `Skipped`: if ESBMC was asked to verify a vowed
function but the verifier could not model the function body, the contract was not statically
proved, so the run exits non-zero. Use `--no-verify` if you genuinely want to skip verification —
that path produces `Unverified` (exit 0).

The table above is the exit status of the `vowc` **compiler**. A **compiled Vow program** exits
with whatever its `main` returns, with one reserved exception: any runtime abort — out-of-memory,
contract violation, arithmetic overflow, unwrap-on-`None`, index-out-of-bounds, region-literal
mutation, stack overflow, or a sanitizer trap — terminates with the reserved status **`134`**. By
convention `134` is reserved for aborts: it is never produced *spontaneously* by a normal `main`
return, so a program that does not itself return or `process_exit(134)` can treat any `134` as a
runtime abort rather than an application result. The reservation is a convention, not enforced — a
program that deliberately exits `134` opts out. See the *Exit status* note under Runtime Errors in
[`errors.md`](errors.md) for the full list and rationale.

## Build Output JSON

`vow build` and `vow verify` emit a single JSON object to stdout. Schema: [`schemas/build-result.schema.json`](schemas/build-result.schema.json).

**Note:** `--dump-ir` suppresses JSON output — only IR text is printed.

### Status Values

| Status          | Meaning                                     |
|-----------------|---------------------------------------------|
| `Verified`      | Compiled + every vowed function's contract was statically proved by ESBMC. |
| `Unverified`    | Compiled but ESBMC was not invoked (e.g. `--no-verify`, `--dump-ir`). Exit 0. |
| `Skipped`       | ESBMC was invoked but at least one vowed function could not be modelled (e.g. body uses `Linear*`, `Load`/`Store`, `RemF*`, or has effects). Struct construction (`RegionAlloc`) and field reads/writes (`FieldGet`/`FieldSet`) **are** modelled via the user-struct heap model. Each skipped function appears as a `VerificationSkipped` *Warning* in `diagnostics[]`. Their contracts are runtime-checked under `--mode debug` but were not statically proved; the run fails closed with exit 1. |
| `CompileFailed` | Parse error, type error, module load error, link failure, or a diagnostic-emission I/O failure (e.g. a broken stderr/stdout pipe other than the tolerated case, or a full disk) |
| `VerifyFailed`  | ESBMC produced a non-Verified outcome: a counterexample, timeout, `VERIFICATION UNKNOWN` (`verify_status: "unknown"`), tool error, the tool was not found, or the verifier worker thread crashed (`verify_status: "panicked"`). Inspect `counterexamples[]` (definitive failures) and `verify_status`/`verify_message` (soft failures) to distinguish. |

### Verified Example

```json
{
  "status": "Verified",
  "executable": "examples/divide",
  "diagnostics": [],
  "counterexamples": []
}
```

### CompileFailed Example

```json
{
  "status": "CompileFailed",
  "executable": null,
  "diagnostics": [
    {
      "error_code": "TypeMismatch",
      "message": "function body has type `bool` but declared return type is `i32`",
      "severity": "error",
      "span": {
        "file": "bad.vow",
        "offset": 25,
        "length": 8
      }
    }
  ],
  "message": "type error",
  "counterexamples": []
}
```

### VerifyFailed Example

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "values": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": {
        "file": "examples/cegis_broken.vow",
        "offset": 76,
        "length": 20
      },
      "blame": "callee"
    }
  ]
}
```

For caller-blame failures where a verified function violates a callee's
`requires` clause, the counterexample reports the callee clause in `violation`
and `vow_id`, and includes the caller expression in `call_sites`. When the
callee precondition binds a parameter, `violating_args` names the callee
parameter, the counterexample value when available, and the caller argument span.
If `violating_args[].value` is `""`, Vow could not statically recover the
caller argument value; `arg_offset` and `arg_length` still identify the
argument expression.

### Fields Reference

| Field              | Type                | When Present      | Description                               |
|--------------------|---------------------|-------------------|-------------------------------------------|
| `status`           | string              | Always            | One of the four status values             |
| `executable`       | string \| null      | Always            | Path to binary, null on compile failure or library module (no main) |
| `diagnostics`      | array               | Always            | Compiler diagnostics (see schema)         |
| `message`          | string              | CompileFailed     | Error category ("parse error", "type error", "module load error", link error detail, or "failed to emit frontend diagnostics: {io_error}") |
| `function`         | string              | VerifyFailed      | Function where verification failed        |
| `counterexample`   | string              | VerifyFailed      | Legacy description string                 |
| `counterexamples`  | array               | Always            | Structured counterexamples (see schema)   |
| `verify_status`    | string              | On backend failure | `"timeout"`, `"unknown"`, `"error"`, `"tool_not_found"`, or `"panicked"` (verifier worker thread crashed — no counterexample available) |
| `verify_message`   | string              | On backend failure | ESBMC/backend error detail                |

### Counterexample replay

With `--replay-cex`, each object in `counterexamples[]` gains a `replay` field — a **differential test** of the verifier's IR-to-C model against the executable's debug-mode runtime semantics. It is **not part of the soundness proof**: it neither strengthens nor weakens the static verdict, and the exit code is unchanged. It exists to catch *drift* between the two independent lowerings — `vow-verify`'s C emitter (`requires` → `__ESBMC_assume`, `ensures`/`invariant` → `__ESBMC_assert`) and `vow-codegen`'s debug runtime checks.

For each counterexample, Vow maps the ESBMC assignment back to concrete Vow inputs, synthesizes a `--mode debug` harness that calls the failing function with those inputs, runs it, and compares the observed `VowViolation`.

| `replay` value | Meaning |
|----------------|---------|
| `"confirmed"`  | The harness fired `VowViolation` with the **same `vow_id` and the same blame** the counterexample predicted. High-confidence: the model agrees with runtime. |
| `"diverged"`   | The harness exited cleanly, or fired a *different* `vow_id`/blame. Either the verifier C model is wrong (a model false-positive) or the counterexample values do not reach the violation in real execution. `replay_reason` explains which. |
| `"skipped"`    | Replay was not attempted (e.g. an input type outside v1 scope, a Unit/aggregate parameter, the function is not defined in the entry file, or harness compilation failed). `replay_reason` gives the cause. |

**v1 input scope.** Reconstruction supports scalar parameters (`i64`, `u64`, `bool`) and bounded `Vec` of those scalars. `String`, `HashMap`, `BTreeMap`, struct, reference, and nested-aggregate parameters are reported as `"skipped"` with a reason. The self-hosted compiler's v1 reconstructs scalars only and reports `Vec` parameters as `"skipped"` (the Rust compiler additionally reconstructs bounded `Vec`s); both report identical outcomes for scalar and aggregate-skip cases. Replaying a counterexample for a function whose entry file already defines `main` is `"skipped"` by the self-hosted compiler.

`replay`/`replay_reason` are present on a counterexample only when `--replay-cex` was passed.

## Contracts Output JSON

`vow contracts` emits a single JSON object to stdout. Schema: [`schemas/contracts-result.schema.json`](schemas/contracts-result.schema.json).

### Example (without --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "not_verified",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 0, "failed": 0, "timeout": 0, "error": 0, "not_verified": 1, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Example (with --verify)

```json
{
  "contracts": [
    {
      "vow_id": 0,
      "function": "divide",
      "kind": "requires",
      "description": "requires y != 0",
      "blame": "Caller",
      "source": { "file": "divide.vow", "offset": 42 },
      "status": "proven",
      "quality": "substantive"
    }
  ],
  "summary": { "total": 1, "proven": 1, "failed": 0, "timeout": 0, "error": 0, "not_verified": 0, "skipped": 0, "vacuous": 0, "trivially_satisfiable": 0, "quality": { "weak": 0, "tautological": 0, "substantive": 1 } }
}
```

### Contract Fields

| Field         | Type    | Description                                              |
|---------------|---------|----------------------------------------------------------|
| `vow_id`      | integer | Unique contract identifier within the program            |
| `function`    | string  | Function containing this contract                        |
| `kind`        | string  | `"requires"`, `"ensures"`, or `"invariant"`              |
| `description` | string  | Full contract text                                       |
| `blame`       | string  | `"Caller"` (requires) or `"Callee"` (ensures/invariant)  |
| `source`      | object  | `{ "file": string, "offset": integer }`                  |
| `status`      | string  | `"proven"`, `"proven-ir"`, `"failed"`, `"unknown"`, `"timeout"`, `"error"`, `"not_verified"`, `"skipped"`, or `"vacuous"` |
| `quality`     | string  | Static clause-shape classification (no ESBMC): `"weak"`, `"tautological"`, or `"substantive"` |
| `trivially_satisfiable` | bool | `--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational — never affects the exit code. See `docs/spec/contracts-methodology.md`. |

### Status Values

| Status          | Meaning                                              |
|-----------------|------------------------------------------------------|
| `not_verified`  | Verification not requested (no `--verify` flag)      |
| `proven`        | ESBMC proved this contract holds for all inputs (bit-vector encoding, overflow modeled) |
| `proven-ir`     | ESBMC proved this contract under integer-arithmetic encoding after BV timed out; overflow is not modeled by IR, but the BV caller preconditions still guard against it |
| `failed`        | ESBMC found a counterexample violating this contract |
| `unknown`       | ESBMC could not conclude for this contract — either `VERIFICATION UNKNOWN` was reported for the containing function (the incremental-BMC forward condition was unable to prove or falsify), or the function's verification failed overall and ESBMC's per-clause `--multi-property` run returned no individual verdict for this clause |
| `timeout`       | ESBMC timed out on the containing function (BV and — when applicable — IR fallback both timed out) |
| `error`         | ESBMC error or tool not found                        |
| `skipped`       | The containing function's body uses opcodes the verifier cannot model (e.g. `Load`/`Store`, `Linear*` consume/borrow, `RemF*`) or the function has effects. (Struct construction and field ops are modelled — see the `Skipped` build-status row.) Contract is documentary; runtime checks still apply under `--mode debug`. Surfaces as a `VerificationSkipped` Warning in the build JSON's `diagnostics[]` and lifts the overall build/verify status to `Skipped` (fail-closed, exit 1) — use `--no-verify` if you want a non-failing path that does not invoke ESBMC at all. |
| `vacuous`       | The containing function's `requires` clauses are contradictory, so every `ensures` is satisfied vacuously — ESBMC proved nothing of substance (antecedent failure). Detected by a second ESBMC run with `--error-label`: a `vow_reach` label planted after the `requires` assumes is unreachable. All of the function's clauses are reported `vacuous` (fail-closed, exit 1). See `docs/spec/contracts-methodology.md`. |

The `proven` / `proven-ir` split and the rule that a resource-limited retry (e.g. the BV→IR fallback) may never report a weakened check as `proven` are the verifier's soundness discipline — the safe-vs-unsafe retry rules are specified in `docs/verifier-discipline.md`.

### Quality Values

`quality` is a static classification of each clause's *shape*, computed without ESBMC and independent of `status`. It surfaces the "proven but trivial" problem: a `weak` contract can be `proven` while constraining almost nothing. See `docs/spec/contracts-methodology.md` for the full taxonomy.

| Quality        | Meaning                                                                                      |
|----------------|----------------------------------------------------------------------------------------------|
| `weak`         | An `ensures` that only bounds `result` by an integer literal (e.g. `result >= 0`). Satisfied by almost any implementation. |
| `tautological` | A constant clause that references no program value (e.g. `true`, `0 >= 0`). Constrains nothing. |
| `substantive`  | Everything else — equality, relational, inverse/round-trip, dispatch-totality, or function-call shapes. The classifier is conservative: anything not provably weak/tautological is reported `substantive`. |

## Trace Output (stderr, --debug-trace)

When `--debug-trace=calls` or `--debug-trace=full` is used, the compiled binary emits JSON lines to stderr:

### calls mode
```json
{"event":"enter","fn":"main"}
{"event":"enter","fn":"divide"}
{"event":"exit","fn":"divide"}
{"event":"exit","fn":"main"}
```

### full mode (adds vow check results)
```json
{"event":"enter","fn":"divide"}
{"event":"vow","fn":"divide","vow_id":0,"passed":true}
{"event":"exit","fn":"divide"}
```

## Profile Output (stderr, profile mode)

When `--mode profile` is used, the compiled binary prints a call-count report to stderr on normal exit (via `atexit`). The report is not printed if the program is killed by a signal or calls `abort()`.

```
--- vow profile report ---
function                                        calls       %
-------------------------------------------------------------
infer                                         4812399   48.2%
is_def_eq_core                                3201882   32.1%
whnf                                           984201    9.9%
main                                                1    0.0%
-------------------------------------------------------------
total calls: 9998483, unique functions: 12
```

The report lists the top 20 most-called functions sorted by call count. No vow checks are emitted in profile mode.

## Runtime Error JSON (stderr, debug/sanitize mode)

When a compiled program runs in debug mode (`--mode debug`) or sanitize mode (`--mode sanitize`) and violates a vow at runtime, it emits JSON to stderr before aborting.

### VowViolation

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

Schema: [`schemas/vow-violation.schema.json`](schemas/vow-violation.schema.json).

### ArithmeticOverflow

```json
{"error":"ArithmeticOverflow"}
```

Emitted when a checked arithmetic operator (`+!`, `-!`, etc.) overflows at runtime.

### UnwrapOnNone

```json
{"error":"UnwrapOnNone"}
```

Emitted when `.unwrap()` is called on `Option::None`.

### IndexOutOfBounds

```json
{"error":"IndexOutOfBounds"}
```

Emitted when a `Vec` index is out of bounds.

### UseAfterFree (sanitize mode only)

```json
{"error":"UseAfterFree","op":"push","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec operation is attempted on a Vec that has already been freed.

### DoubleFree (sanitize mode only)

```json
{"error":"DoubleFree","vec":"0x55a1b2c3d4e0"}
```

Emitted when a Vec is freed twice.

### StaleIndex (sanitize mode only)

```json
{"error":"StaleIndex","index":5,"expected_gen":3,"actual_gen":7,"vec":"0x55a1b2c3d4e0"}
```

Emitted when `__vow_sanitize_check_generation` detects that a Vec slot's generation counter does not match the expected value, indicating the slot was overwritten since the index was recorded.

## Agent Decision Tree

```
Parse JSON from stdout
├── status == "Verified"       → Success. Binary at `executable`.
├── status == "Unverified"     → Compiled but unverified. ESBMC missing or --no-verify.
├── status == "CompileFailed"  → Read `diagnostics[]` for error details.
│   ├── error_code is parse error  → Fix syntax (see grammar.md)
│   └── error_code is type error   → Fix types (see errors.md)
└── status == "VerifyFailed"   → Read `counterexamples[]`.
    ├── Check `inputs` for the violating values
    ├── Check `violation` for which contract failed
    ├── Check `source` for the location
    └── Fix the contract or the implementation, then rebuild
```

Always check stderr for human-readable diagnostics alongside the JSON on stdout.
"#,
        ),
        (
            r#"reference/contracts.md"#,
            r#"# Contract Authoring and Verification

Vow uses ESBMC (bounded model checker) for static contract verification. This document covers contract patterns, verification behavior, and common pitfalls.

## Verification Pipeline

Codegen (Cranelift) and verification run in parallel:

```
Vow Source → Parse → Type Check → IR Lower ─┬─→ Cranelift → executable
                                              └─→ C Emit → ESBMC → proof / counterexample
```

Contract clauses become IR opcodes. The C emitter translates `requires` to `__ESBMC_assume()` (the verifier assumes preconditions hold) and `ensures`/`invariant` to `__ESBMC_assert()` (the verifier checks postconditions).

### ESBMC Configuration

- Verification strategy: **incremental BMC** (`--incremental-bmc`) — base case plus forward condition, **not** k-induction (there is no inductive step). A contract is `proven` only when ESBMC's forward condition establishes completeness within the bound; otherwise the result is `unknown`, never a false `proven`
- Incremental BMC with `--max-k-step` (default: **50**) — loops are verified incrementally up to N iterations
- Architecture: 64-bit
- Array bounds / pointer checks disabled (Vow handles these in its own model)

### Collection Models for Verification

ESBMC is a *bounded* model checker, so it models collection types as
fixed-size arrays and reasons about them up to a finite capacity. These
capacities are an internal property of the verifier, not of the language:

| Type              | Model Capacity | Supported Operations |
|-------------------|----------------|----------------------------------------------|
| `Vec<T>`          | 128            | `new`, `push`, `pop`, `len`, `get`, `set`    |
| `String`          | 256            | `from`, `len`, `push_byte`, `push_str`, `byte_at`, `matches_literal_at` |
| `HashMap<K, V>`   | 64             | `new`, `insert`, `get`, `contains_key`, `len`|
| `BTreeMap<K, V>`  | 64             | `new`, `insert`, `get`, `contains_key`, `len`|

**These bounds are not a language feature and are not user-tunable.** A `Vec`
in a Vow program grows dynamically on the heap with no fixed maximum; the
capacity above only describes how far the *bounded* model checker reasons. The
language and its contracts are deliberately decoupled from what any particular
prover can prove: replace ESBMC with a stronger (or unbounded) checker and the
same source, the same contracts, and the same CLI keep working — the only
difference is that proof covers more (or all) of the state space. For this
reason a `requires`/`ensures` clause must never encode a verifier bound (e.g.
`requires: v.len() <= 128`); see "Verification-Driven Bounds (Anti-Pattern)"
below and `docs/design/verifier-model-bounds.md`.

These models support the same operations as the runtime but with bounded
storage. String literals carry their concrete length and bytes in verification,
and `String::from` copies that model from its source value. The effective string
model capacity is automatically at least the longest static string literal, so
literal byte initializers always fit the model array. Operations whose bytes are
not statically known, such as `String::from_cstr`, produce a nondeterministic
length (0 to max-1). `string_matches_literal_at` is modeled against the
literal's concrete bytes and byte length; the third argument must be a string
literal so the verifier never has to infer static text from a dynamic `String`.

## Blame Model

| Clause      | Blame  | Who is at fault                                    |
|-------------|--------|----------------------------------------------------|
| `requires`  | Caller | The caller passed invalid arguments                |
| `ensures`   | Callee | The function body doesn't satisfy the postcondition|
| `invariant` | Callee | The loop body breaks the invariant                 |

## Counterexample Replay (Differential Test)

`vow verify --replay-cex` (also `vow build --replay-cex`) cross-checks a counterexample against the executable's runtime semantics. After ESBMC reports a violation, Vow maps the symbolic assignment to concrete Vow inputs, builds a `--mode debug` harness that calls the failing function with them, and checks whether the runtime `VowViolation` matches — **same `vow_id` and same blame**.

This is a *differential test*, **not part of the proof**. The static verdict and exit code are unchanged whether or not replay is requested. Its purpose is to detect drift between the two independent lowerings of a contract: the verifier's C model (`requires` → `__ESBMC_assume`, `ensures`/`invariant` → `__ESBMC_assert`) and `vow-codegen`'s debug-mode runtime checks. A `confirmed` replay grounds the counterexample in real execution; a `diverged` replay flags either a model false-positive or values that do not reach the violation at runtime. See `docs/spec/cli.md` → "Counterexample replay" for the JSON shape and v1 input scope.

## Integer Contracts

### Non-zero Guard

```vow
fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}
```

### Range Bounds

Use range bounds only when they reflect genuine semantic constraints (e.g., overflow prevention), not to appease the verifier:

```vow
fn safe_add(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

The bounds here prevent `a + b` from overflowing `i64` — a legitimate semantic concern, not a verifier limitation.

### Equality Postcondition

```vow
fn twice(x: i64) -> i64 vow {
    ensures: result == x + x
} {
    x + x
}
```

### Negation

```vow
fn negate(x: i64) -> i64 vow {
    ensures: result + x == 0
} {
    0 - x
}
```

**Warning:** Fails for `x = -9223372036854775808` (i64 min) due to wrapping overflow. Add `requires: x > -9223372036854775808` if needed.

## Vec Contracts

### Bounds Check

```vow
fn get_element(v: Vec<i64>, i: i64) -> i64 vow {
    requires: i >= 0,
    requires: i < v.len()
} {
    v[i]
}
```

### Fill Pattern with Loop Invariant

See the worked CEGIS example in [examples.md](examples.md#3-vec-fill--loop-invariant).

## String Contracts

### Non-empty String

```vow
fn make_greeting() -> String vow {
    ensures: result.len() > 0
} {
    let s: String = String::from("");
    s.push_byte(72);
    s
}
```

## HashMap Contracts

### Contains Key After Insert

```vow
fn insert_and_check() -> HashMap<i64, i64> vow {
    ensures: result.contains_key(42)
} {
    let m: HashMap<i64, i64> = HashMap::new();
    m.insert(42, 100);
    m
}
```

## Loop Invariants

### Counter Bounds

The most common loop invariant pattern bounds the loop counter:

```vow
while i < n vow {
    invariant: i >= 0,
    invariant: i <= n
} {
    i = i + 1;
}
```

### Search Range

```vow
fn bisect(lo: i64, hi: i64) -> i64 vow {
    requires: hi >= lo
} {
    let mut lo: i64 = lo;
    let hi: i64 = hi;
    while lo + 1 < hi vow {
        invariant: hi - lo >= 0
    } {
        let mid: i64 = lo + (hi - lo) / 2;
        lo = mid;
    }
    lo
}
```

## Where Clause Patterns

Where clauses on parameters become refinement types (additional `requires` for verification):

```vow
fn bounded_add(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a <= 4611686018427387903,
    requires: b <= 4611686018427387903,
    ensures: result >= a,
    ensures: result >= b
} {
    a + b
}
```

Each `where` clause can only reference its own parameter.

## Anti-Patterns

### Tautological Contracts

A contract must constrain behavior the implementation could get wrong. A clause provable from the return type alone, or from a constant/literal body, verifies nothing.

```vow
fn IOP_CONST() -> i64 vow { ensures: result >= 0 } { 0 }
fn sentinel() -> i64 vow { ensures: result == -1 } { -1 }
```

The first is trivially true of the literal `0`; the second restates the body verbatim. Both prove nothing and only enlarge the proof surface.

**Fix:** delete the `vow` block. A postcondition earns its place only when it pins a property of a **computed** result — one that depends on the inputs or control flow and that a wrong implementation would violate (`ensures: result > 0` on a loop-computed `gcd`; `ensures: result == 0 || result == 1` on a branch-computed flag). Named-constant accessors and enum-tag functions returning a literal must carry no contract.

**Crisp rule:** if the clause is true without reading past the signature and a constant body, it is a non-contract — remove it. This is distinct from weakening a real contract (forbidden, see CLAUDE.md "Contract Authoring"): a tautology was never a contract, so deleting it loses no verification value.

### Over-Specifying

```vow
fn add(x: i64, y: i64) -> i64 vow {
    ensures: result == x + y
} {
    x + y
}
```

Fails when `x + y` overflows. The contract mirrors the implementation exactly — it verifies nothing useful and breaks on edge cases.

**Fix:** Add bounds (`requires: x >= 0, ...`) or verify a weaker property.

### Wrapping Arithmetic Overflow

Default arithmetic (`+`, `-`, `*`) wraps on overflow. Contracts that assume no overflow will be violated:

```vow
fn double(x: i64) -> i64 vow {
    ensures: result > x
} {
    x + x
}
```

ESBMC finds: `x = 4611686018427387904` → `result = -9223372036854775808` (wraps negative).

**Fix:** Bound the input or use checked arithmetic (`+!`).

### Non-Inductive Loop Invariant

An invariant must hold at the **start** of every iteration, not just at the end:

```vow
while i < n vow {
    invariant: v.len() == n
} { ... }
```

This is not inductive — `v.len() == n` is only true after the loop.

**Fix:** Use `invariant: i >= 0, invariant: i <= n`.

### Unbound Loop Iterations

Without a bound on loop iterations, ESBMC may timeout (default max-k-step is 50):

```vow
fn fill(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    ensures: result.len() == n
} { ... }
```

ESBMC will only verify this for small `n` values. **Do not** add `requires: n <= 8` to the contract — that would distort the semantic specification. The contract is correct as-is; ESBMC's bounded verification provides partial assurance.

### Verification-Driven Bounds (Anti-Pattern)

**Never** add artificial bounds to contracts solely to help ESBMC verify them:

```vow
// WRONG: bounds exist only to appease the verifier
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    requires: a <= 15,   // <-- verifier artifact, not semantic
    requires: b <= 15,   // <-- verifier artifact, not semantic
    ensures: result > 0
} { ... }
```

```vow
// CORRECT: only genuine semantic constraints
fn gcd(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: a + b > 0,
    ensures: result > 0
} { ... }
```

Contracts express what is mathematically required for correctness. ESBMC verifies within its capabilities (bounded loops, bounded arithmetic, bounded collection models) — if it cannot fully prove a correct contract, that is acceptable. Partial verification is better than a distorted specification. The same rule is why the verifier's collection model capacities (see "Collection Models for Verification") are internal defaults rather than CLI flags or contract clauses: a bound that belongs to the prover must never leak into the language.

## Interpreting Counterexamples

A counterexample in the JSON output:

```json
{
  "function": "safe_sub",
  "values": { "a": "-9223372036854775808", "b": "0" },
  "violation": "ensures result >= 0",
  "vow_id": 1,
  "source": { "file": "cegis_broken.vow", "offset": 76, "length": 20 },
  "blame": "callee"
}
```

| Field       | Meaning                                                        |
|-------------|----------------------------------------------------------------|
| `function`  | Which function's verification query failed                     |
| `values`    | Source or ESBMC variable values in the counterexample           |
| `violation` | Which contract clause was violated                             |
| `vow_id`    | Function-local ID linking to the specific vow clause            |
| `source`    | Byte offset in the source file of the violated clause           |
| `blame`     | Whether the caller, callee, or neither party is responsible     |

When caller code violates a callee's `requires` clause, `violation` and
`vow_id` identify the callee clause. `call_sites` points back to the caller
expression, and `violating_args` identifies the callee parameter and caller
argument span when Vow can recover it. If `violating_args[].value` is `""`,
Vow could not statically recover the caller argument value; `arg_offset` and
`arg_length` still identify the argument expression.

Variable names prefixed with `_esbmc_` are ESBMC internal variables; named inputs map directly to function parameters.

## Unsigned Integer Contracts

The `u64` type works naturally in contracts. Use `as u64` to cast literal values in contract expressions:

```vow
fn safe_add(a: u64, b: u64) -> u64
vow {
    requires: a <= 1000 as u64
    requires: b <= 1000 as u64
    ensures: result >= a
    ensures: result >= b
}
{
    a + b
}
```

ESBMC verifies `u64` contracts using `uint64_t` and unsigned nondet values.

## Extern Block Contracts

Every `extern "C"` block **must** include a `vow { ... }` contract specifying the expected behavior of foreign functions. Omitting the contract is a `MissingContract` error.

```vow
extern "C" vow {
    requires: fd >= 0
    ensures: return >= 0
}
{
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io]
}
```

The contract applies to all functions declared in the block. ESBMC uses `requires` as assumptions and `ensures` as assertions when verifying callers of extern functions.
"#,
        ),
        (
            r#"reference/contracts-methodology.md"#,
            r#"# Contract Methodology: What to Verify

This document answers a question that `contracts.md` does not: given a function,
**which** properties are worth proving, how do you tell a strong contract from a
hollow one, and how do you express the strong shapes within ESBMC's reach.

`contracts.md` is the *reference* (syntax, blame, the verification pipeline,
anti-patterns). This is the *methodology* (judgement). Read that first.

## The core principle: strength, not volume

A proven contract is worth nothing if it would also hold for an incorrect
implementation. The number of contracts a codebase proves is not a quality
signal — the *discriminating power* of each contract is.

This is measurable. Polikarpova, Furia, Pei, Wei, and Meyer (the originator of
Design by Contract), *"What Good Are Strong Specifications?"* (ICSE 2013), found
that testing implementations against **strong** specifications — comprehensive
functional pre/post/invariants — detected roughly **twice as many bugs** as
testing against standard/weak contracts. Their conclusion: *"the quality of
specifications limits the value of verification."*

A concrete Vow example of the trap. Many tag constants in the self-hosted
compiler carry this contract:

```vow
fn EFF_IO() -> i64 vow { ensures: result > 0 } { 1 }
```

ESBMC proves `1 > 0` in milliseconds. But `ensures: result > 0` also holds for
`{ 2 }`, `{ 99 }`, and every other positive constant — it does not pin the value
this function is *supposed* to return. It is a real postcondition, but a **weak**
one: it constrains the output to a half-line, not to a point. Proving 354 of
these does not make the compiler more correct; it makes the verification report
longer.

The fix is not "delete contracts" — it is "make each contract say something only
the correct implementation satisfies."

## A taxonomy of contract shapes

Each shape below lists its *intent*, *when it applies*, a *real Vow example*, and
*strength notes*. The expressibility/verifiability status of every shape is
collected in the matrix at the end.

### 1. Domain precondition (range / validity bound)

**Intent:** restrict the inputs the function promises to handle correctly. Blame
falls on the caller (`requires`).

**When:** the function is only correct on a subset of its parameter types — a byte
in `0..=255`, a non-zero divisor, an in-bounds index.

```vow
fn write_u8(out: Vec<i64>, v: i64) vow {
    requires: v >= 0,
    requires: v <= 255
} { out.push(v); }
```

**Strength:** a precondition is strong when it is the *true* domain of the
function — no wider (which would admit miscompilation) and no narrower (a
verifier-driven bound like `requires: n <= 8`, forbidden by `contracts.md`).
A bounds-check precondition such as `requires: i >= 0, requires: i < v.len()` is
the standard guard for every indexing operation.

### 2. Output-range postcondition (the weak default — use sparingly)

**Intent:** constrain the result to a range.

**When:** the range *is* the full functional spec — e.g. a function whose only
guarantee is non-negativity. This is rare. Most uses are the weak trap above.

```vow
fn item_kind(v: i64) -> i64 vow {
    requires: v >= 0,
    ensures: result >= 0          // weak: any non-negative value satisfies this
} { v / 4294967296 }
```

**Strength:** weak by default. Reach for shape 3, 4, or 5 instead whenever the
function actually computes a *specific* value. If you find yourself writing
`ensures: result >= 0` on a function that returns a computed quantity, ask what
the result *equals* or *inverts*, and assert that.

### 3. Exact functional postcondition (equality)

**Intent:** pin the result to the value the function is defined to produce.

**When:** the output is a closed-form function of the inputs (arithmetic,
bit-packing, encodings).

```vow
fn region_pack(kind: i64, val: i64) -> i64 vow {
    requires: kind >= 0,
    requires: kind <= 3,
    requires: val >= 0,
    requires: val <= 4294967295,
    ensures: result == val * 4 + kind     // exact: only the right answer passes
} { val * 4 + kind }
```

**Strength:** strong — a wrong shift or offset is caught immediately. Note the
preconditions are not verifier appeasement: they bound `val` and `kind` so the
packed result cannot overflow `i64` (a genuine semantic constraint). Contrast
`region_pack` (exact) with `item_pack`/`item_kind` (shape 2, only `>= 0`): the
same bit-packing pattern, one specified strongly and one weakly.

### 4. Round-trip / inverse

**Intent:** prove that an encode/decode (or pack/unpack, serialize/deserialize)
pair compose to the identity on the valid domain.

**When:** two functions are defined as inverses — `pack`/`unpack`,
`encode`/`decode`, `to_bytes`/`from_bytes`.

Specify each direction with an exact closed-form postcondition — shape 3 applied
to the extractor as well as the encoder, so the decoder is pinned to the exact
arithmetic that inverts the pack:

```vow
fn region_kind(r: i64) -> i64 vow {
    requires: r >= 0,
    ensures: result == r - (r / 4) * 4,   // exact extractor
    ensures: result <= 3
} { r - (r / 4) * 4 }
```

Because both directions are pinned to closed forms — `region_pack`'s exact
`ensures: result == val * 4 + kind` (shape 3 above) and the matching
`region_kind`/`region_val` extractors — a `region_pack` then
`region_kind`/`region_val` round-trip recovers `(kind, val)` exactly, and ESBMC
discharges that composition with no separate assertion. The inverse can also be
asserted directly: Vow allows pure-function calls in postconditions, so an
`ensures: region_kind(result) == kind` on `region_pack` is expressible and
modelable when the partner is pure (matrix shape 4). **Strength:** very strong —
round-trip is the property a serialization layer must have, and it catches the
entire class of "encoder and decoder drifted apart" bugs that output-range
contracts miss completely.

### 5. Dispatch totality (fail-closed decoders)

**Intent:** prove that a decoder/dispatcher maps **every** valid input to a
defined output and **never** silently falls through to a default.

**When:** a function switches over a tag/opcode/discriminator. This is the
single highest-value shape for Vow, because silent-fallback normalization
(mapping an unknown tag to a valid-looking default) is the bug class issue #81
was filed over.

The pattern has two halves — a validity precondition and an explicit error
sentinel for the unreachable tail:

```vow
fn is_valid_binop(op: i64) -> bool { op >= 0 && op <= 22 }

fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op)
} {
    if op == BINOP_ADD() { return ...; }
    // ... one arm per valid op ...
    -1                                    // unreachable under the precondition
}
```

**Strength — and a live hardening gap.** The precondition pins the domain, but
this contract does **not yet prove totality**: nothing asserts the function never
returns the `-1` sentinel. The strong form adds a postcondition that rules out
the fallthrough:

```vow
fn binop_opcode(op: i64, operand_ty: i64) -> i64 vow {
    requires: is_valid_binop(op),
    ensures: result != -1                 // proves every valid op is handled
} { ... }
```

With `ensures: result != -1`, ESBMC must show that on every `op` in `0..=22` some
arm returns before the sentinel — i.e. the dispatch is exhaustive. If an agent
later adds opcode 23 to `is_valid_binop` but forgets the matching arm,
verification fails instead of miscompiling. This is the contract that converts a
silent fallback into a caught error.

The static classifier rates this clause `substantive`, and `vow contracts --verify`'s
body-replace probe reports it `trivially_satisfiable: true` — both are correct, because
they measure different things. The probe replaces the body with `return 0` (the `i64`
default); `0 != -1` holds, so by the definition in **Weakness** (below) this is a *true*
positive: `ensures: result != -1` does not constrain the op→opcode *mapping* — a constant
non-sentinel body (`return 5`) satisfies it for every valid `op`. What the clause *does*
prove is dispatch **totality**: every valid `op` reaches an arm before the `-1` fallthrough
(delete an arm and verification fails). Totality is the silent-fallback property #81
targets, and — absent a quantifier to say "result is the correct opcode for `op`" — it is
the strongest property a `!= sentinel` postcondition can express. So read the
`trivially_satisfiable: true` as accurate (the clause pins totality, not the mapping), not
as a probe artifact to dismiss. This is *not* the constant-result false positive noted in
**Weakness**: `binop_opcode`'s correct result varies per `op`, so it is not genuinely the
type default.

> Vow has no surface quantifier (`forall i in 0..n`) today, so "covers all valid
> inputs" is expressed as `requires` (pin the finite domain) + a postcondition
> that excludes the failure value, letting ESBMC enumerate the finite branch
> structure. Bounded quantifiers are tracked as a roadmap item (#467/#470).

### 6. Relational / cross-function (uniqueness, agreement)

**Intent:** state a property that spans more than one function or more than one
argument.

**When:** tags in a family must be distinct; two collections must have equal
length; a result must relate two inputs.

The argument-relational form is directly expressible:

```vow
fn build_pairs(ids: Vec<i64>, names: Vec<i64>) -> Vec<Pair> vow {
    requires: ids.len() == names.len()
} { ... }
```

The *cross-function uniqueness* form — "`tok_kw_fn() != tok_kw_let()` for every
pair in the family" — is expressible only as O(n²) pairwise inequalities, which
does not scale to dozens of tag constants. **The better fix is structural, not
contractual:** encode each family as a base+offset range or a generated table so
uniqueness is a property of the *encoding* rather than something every function
must restate. Treat a wall of zero-argument tag constants as an API smell that
shape-6 contracts cannot economically repair.

### 7. Loop invariant / frame

Covered in `contracts.md` (counter bounds, search-range invariants, the
inductiveness requirement). The methodology point: an invariant is strong when it
is the property the loop *maintains toward its postcondition*, not merely
`i >= 0`. See `contracts.md` §Loop Invariants and the worked CEGIS cycle in
`examples.md`.

## Hollow contracts: three failure modes to detect

A contract can pass verification while proving nothing. There are three distinct
ways this happens; a contract-quality tool should distinguish them.

### Weakness

The clause is satisfiable and true, but so loose that an incorrect
implementation also satisfies it (`ensures: result >= 0` on a computed value).
This is the 354-contract problem.

**Detection (the body-replace probe).** `vow contracts --verify` ships this check
(#81). It mutates the implementation in the strongest possible way — replaces the
whole body with a trivial `return <type-default>` — and re-verifies the
`ensures`. If the contract still proves against that body, a constant-returning
implementation satisfies it, so it does not constrain the real computation: each
such `ensures` is reported `trivially_satisfiable: true`. This is exactly the
`body-replace` mutation of `vowc mutants` with ESBMC as the oracle.

The signal is **one-sided (sound, not complete)**: a `true` verdict is a proof of
weakness (a specific trivial body satisfies the contract), but a `false` verdict
does not prove strength — the probe uses a single default value and skips
non-scalar returns, returned parameters, and φ-merged/branchy results, so it can
miss weak contracts it cannot witness this way. It is informational and never
changes the exit code; pair it with the static `quality` field. The one known
false positive is a function whose correct result genuinely *is* the type default
(e.g. a constant `ensures result == 0` on a `{ 0 }` body) — an equivalent mutant,
the standard caveat of mutation testing.

### Tautology

The clause is true independent of the program — `ensures: true`,
`ensures: result == result`, `ensures: x >= 0 || x < 0`. **Detection:** the clause
is valid (provable) with the function body removed; a cheap check folds constant
clauses and flags any clause with no dependence on parameters or `result`.

### Vacuity (antecedent failure)

The clause is proved only because its **preconditions are unsatisfiable**, so the
path it guards is dead and the postcondition never has to hold. Because Vow
lowers `requires` to `__ESBMC_assume`, a contradictory or over-strong precondition
makes *any* `ensures` provable — an assume-false / dead-path proof.

This is the classic vacuity of Beer, Ben-David, Eisner, and Rodeh, *"Efficient
Detection of Vacuity in Temporal Model Checking"* (Formal Methods in System
Design, 2001): a subformula is vacuous when replacing it changes nothing about
the result. Their industrial data is the reason to take it seriously — across
years of hardware verification at IBM, ~20% of formulas were trivially valid on
first runs, and trivial validity *always* indicated a real defect in the design,
spec, or environment.

**Detection (the reachability probe).** `vow contracts --verify` ships this check
(#81). For any function carrying a `requires`, it re-runs ESBMC over the same
model with a `vow_reach` label planted immediately after the `requires` assumes,
under `--error-label vow_reach`. If ESBMC reports the label **unreachable**
(`VERIFICATION SUCCESSFUL`), the conjoined preconditions are contradictory and
every `ensures` held only vacuously — all of the function's clauses are reported
`status: "vacuous"` and the command fails closed. If the label is **reachable**
(`VERIFICATION FAILED`), the precondition domain is non-empty and the proof is
live. This is operationally the dual of the classic `ensures: false` re-check —
asking "is the post-`requires` point reachable?" instead of "does `assert(false)`
still pass?" — but it needs only one extra run per function and is unaffected by
body divergence, since the label precedes the body. The label sits after the
requires prefix rather than at the function end precisely so an unbounded loop or
an `assume(0)` deeper in the body cannot make it spuriously unreachable.

**Interesting witnesses.** Beer et al. also propose the dual of a counterexample:
for a proof that holds, emit a non-trivial *witness* — concrete inputs that
exercise the property for a substantive reason — so the author can confirm the
proof is not hollow. Vow's structured output is well-suited to carrying a witness
alongside each `Verified` result.

## When to write contracts

### Builtins and `extern` blocks

Runtime functions (`Vec.push`, `String.from`, `HashMap.insert`) are implemented in
Rust/C and cannot be verified by ESBMC. Their behavior enters verification through
the `vow` contract on the `extern "C"` block, which becomes an **assumed**
(`__ESBMC_assume`) surface for callers. Because these contracts are *assumed, not
checked*, they are the most dangerous place for an error: a wrong `ensures` on an
extern block silently weakens every proof that depends on it. Extern contracts
must be reviewed as assumptions, audited against the runtime implementation, and
kept minimal. (Omitting the block is a `MissingContract` error — see `errors.md`.)

### Library functions (written in Vow)

Public Vow functions are fully within verification reach. Give each one its true
domain precondition and the strongest postcondition shape that applies (3–6, not
2). Add contracts when the function's contract is *known*, which is usually at
definition time for pure utilities and after the signature stabilizes for APIs.

### Application code (including agent-generated)

Vow's target author is an AI agent, and the failure mode to design against is
*volume over substance* — an agent emitting many `ensures: result >= 0` clauses
because the prompt said "add contracts." Skill guidance should push the opposite:
for each function, identify which shape applies (equality, round-trip, dispatch
totality, relational) and write that one; prefer one discriminating contract to
five weak ones. The Specification Pattern System of Dwyer, Avrunin, and Corbett
(ICSE 1999) — a survey-validated catalog built specifically to turn imprecise
intent into precise specifications — is the model for guiding an author from "this
should be valid" to a postcondition that says what *valid* means.

## Expressibility and verifiability matrix

Whether a shape is usable depends on five independent axes, not just "can the
syntax say it":

- **expressible** — surface Vow has the syntax
- **typechecked** — the checker validates the clause to `bool` (added in #81 Phase 0)
- **lowerable** — the lowerer emits IR for the clause
- **modelable** — the C emitter / ESBMC model supports the operations used
  (pure, non-effectful helpers only; see `is_modelable` in the C emitter)
- **backend** — ESBMC actually discharges it within bounds

| Shape | expressible | typechecked | lowerable | modelable | backend |
|-------|:-----------:|:-----------:|:---------:|:---------:|:-------:|
| 1. Domain precondition | ✓ | ✓ | ✓ | ✓ | ✓ |
| 2. Output-range postcond. | ✓ | ✓ | ✓ | ✓ | ✓ (but weak) |
| 3. Exact equality | ✓ | ✓ | ✓ | ✓ | ✓ within overflow bounds |
| 4. Round-trip / inverse | ✓ | ✓ | ✓ | ✓ if partner is pure & modelable | ✓ for arithmetic |
| 5. Dispatch totality | ✓ | ✓ | ✓ | ✓ (pure dispatch) | ✓ over finite domain |
| 6a. Argument-relational | ✓ | ✓ | ✓ | ✓ | ✓ |
| 6b. Cross-fn uniqueness | ✓ (O(n²)) | ✓ | ✓ | ✓ | ✓ but unscalable → prefer structural encoding |
| 7. Loop invariant | ✓ | ✓ | ✓ | ✓ | partial (bounded / k-induction) |
| Bounded quantifier (`forall i in 0..n`) | ✗ (no surface syntax) | — | — | — | — (roadmap #467/#470) |

Contract expressions must be **pure** — they cannot call effectful functions
(`grammar.md` §Contract Purity). A property that needs an effectful helper is
blocked at the *modelable* axis, not the expressible one; classify such gaps as
model limitations, not contract-language limitations.

## Tooling

`vow contracts --verify` performs the **per-obligation** quality analysis tracked
in #81 / roadmap WS-3.2. Each clause gets an individual verification verdict (via
ESBMC `--multi-property`), plus the three quality signals above:

- **Tautology** — the static `quality` field flags constant clauses (no ESBMC).
- **Vacuity** — a contradictory `requires` is caught by the `--error-label`
  reachability probe and reported `status: "vacuous"` (fail-closed).
- **Weakness** — the body-replace probe reports `trivially_satisfiable: true` for
  an `ensures` a trivial `return <default>` body satisfies (informational).

The `summary` carries `vacuous` and `trivially_satisfiable` counts alongside the
status and quality tallies, so an author or CI can gate on hollow proofs.

**CI weak-gate.** `scripts/check_contract_quality.py` ratchets on the static
quality of the self-hosted compiler's own contracts: it reads
`vow contracts compiler/main.vow` and fails if the `weak` or `tautological` count
exceeds a committed baseline, so a new `ensures result >= 0` cannot slip in
unnoticed. It runs in `scripts/full_test.sh`. The baseline is an upper bound to
ratchet down as contracts harden. The dispatch-totality example above
(`binop_opcode`, `ensures: result != -1`) and `binop_result_ty`
(`ensures: result == ITY_BOOL() || result == ITY_U64() || result == ITY_I64()`)
are enforced in `compiler/lower.vow` today.

**Tag families are structural, not contracted.** The bulk of the old `weak`
count was nullary tag constants — `fn IOP_VOW_REQ() -> i64 { 73 }`, the `ITY_*`,
`EXPR_*`, `BINOP_*`, `RSUM_KIND_*`, … enum families. A per-constant `ensures
result >= 0` proves nothing: a constant's value is the only fact about it, and
that fact is structural (each is a distinct literal). So these carry **no**
contract. Their correctness is established where it matters — at use sites: the
dispatch-totality contracts above prove every valid tag is handled, the IR
validator and serializer round-trips exercise every kind, and the binary
fixed-point bootstrap miscompiles if any two tags collide. Removing the
contracts cut the compiler's `weak` count from 408 to 11; the remaining bit-packers — the
region/span packers and friends (`region_pack`/`region_kind`/`region_val`,
`span_pack`, `item_kind`, `marker_caller_store`, `suffix_len`) — were then hardened
with exact functional
postconditions: `item_kind` with `result == v / 4294967296`, and `suffix_len` with a
per-suffix conditional mapping (`(suffix != tok_suffix_i64() || result == 3) && …`, one
conjunct per suffix plus an unknown-suffix → `0` clause), bringing `weak` to **0** (#81).
The CI weak-gate now holds that baseline: no weak
contract may enter the self-hosted compiler.

## References

- N. Polikarpova, C. A. Furia, Y. Pei, Y. Wei, B. Meyer. *What Good Are Strong
  Specifications?* ICSE 2013. https://arxiv.org/abs/1208.3337
- I. Beer, S. Ben-David, C. Eisner, Y. Rodeh. *Efficient Detection of Vacuity in
  Temporal Model Checking.* Formal Methods in System Design 18:141–163, 2001.
- M. Dwyer, G. Avrunin, J. Corbett. *Property Specification Patterns for
  Finite-State Verification.* ICSE 1999.
- B. Meyer. *Object-Oriented Software Construction* (Design by Contract).
"#,
        ),
        (
            r#"reference/errors.md"#,
            r#"# Vow Error Catalog

Every Vow error has a machine-readable `error_code` in the JSON output. This document lists all error codes, their phase, meaning, an example trigger, and how to fix them.

## Compile-Time Errors

These appear in the `diagnostics` array of the build output JSON.

### UnterminatedString

**Phase:** Lexer
**Meaning:** A string literal was opened with `"` but never closed.

```vow
fn f() -> () [io] {
    print_str("hello);
}
```

**Fix:** Close the string with a matching `"`.

### InvalidCharacter

**Phase:** Lexer
**Meaning:** The source contains a character the lexer does not recognize.

```vow
fn f() -> i64 {
    x @ y
}
```

**Fix:** Remove the invalid character. Vow has no `@` operator.

### UnexpectedToken

**Phase:** Parser
**Meaning:** The parser encountered a token it did not expect at that position.

```vow
module M 123
```

**Fix:** Check the syntax around the reported span. Common causes: missing `{`, `}`, `(`, `)`, or a keyword in the wrong position.

### MissingDelimiter

**Phase:** Parser
**Meaning:** A matching delimiter (`}`, `)`, `]`) is missing.

```vow
fn f() -> i64 {
    42
```

**Fix:** Add the missing closing delimiter.

### TypeMismatch

**Phase:** Type Checker
**Meaning:** An expression has a different type than expected.

```vow
fn f() -> i32 {
    true
}
```

**Output:** `function body has type 'bool' but declared return type is 'i32'`

**Fix:** Change the expression or the declared type to match.

### LiteralOutOfRange

**Phase:** Type Checker
**Meaning:** An integer literal appears in a typed context (annotated `let`, function argument, struct field, or const declaration) whose target type cannot hold the literal's value. The check runs after context coercion, so the offending literal is the one written in the source, not a widened intermediate.

```vow
let x: u8 = 300;
const NEG: u16 = -1;
```

**Output:** `literal 300 does not fit in u8 (range 0..=255)`

**Fix:** Use a value within the target type's range, change the target type, or write an explicit narrowing intrinsic (`i64_to_u8_try`, `i64_to_u8_wrap`, `i64_to_u8_sat`) if you intend to convert a wider value at runtime.

### NarrowingCastNotAllowed

**Phase:** Type Checker
**Meaning:** The `as` operator was used to convert a wider integer type to a narrower one. `as` is widening-only; narrowing must use a named intrinsic so the agent chooses an explicit semantics (range-checked vs. truncating vs. saturating). See `grammar.md` §Type Cast.

```vow
fn f(big: i64) -> u8 {
    big as u8
}
```

**Output:** `cannot cast 'i64' to 'u8' via 'as'; use 'i64_to_u8_try', 'i64_to_u8_wrap', or 'i64_to_u8_sat' to choose the narrowing semantics`

**Fix:** Replace the cast with the narrowing intrinsic that matches your intent:
- `i64_to_u8_try(big) -> Option<u8>` — reject out-of-range with `None`
- `i64_to_u8_wrap(big) -> u8` — truncate (keep low bits)
- `i64_to_u8_sat(big) -> u8` — clamp to `0..=255`

### ShiftCountOutOfRange

**Phase:** Type Checker
**Meaning:** A constant-expression shift count is greater than or equal to the bit-width of the left operand. Shifting an `N`-bit value by `>= N` bits is undefined in the underlying C model and is rejected at compile time when the count is statically known. Dynamic shift counts (non-const expressions) get a Vow contract on the operation and are checked by ESBMC and at runtime in debug mode.

```vow
fn f(x: u8) -> u8 {
    x << 8
}
```

**Output:** `shift count 8 is out of range for u8 (max 7)`

**Fix:** Use a count less than the LHS bit-width. To shift a narrow value by a larger amount, widen first: `(x as u32) << 8` is legal (it shifts the widened `u32` value by 8), but the result is `u32`; to get back to `u8`, use a narrowing intrinsic such as `u32_to_u8_wrap`.

### StaticLiteralRequired

**Phase:** Type Checker
**Meaning:** A compiler intrinsic requires a string literal operand so it can be lowered without allocation.

```vow
fn f(s: String, key: String) -> i64 {
    string_matches_literal_at(s, 0, key)
}
```

**Output:** `string_matches_literal_at requires a string literal as its third argument`

**Fix:** Pass a literal directly, for example `string_matches_literal_at(s, 0, "name")`.

### EffectViolation

**Phase:** Type Checker
**Meaning:** A function calls another function with effects not declared in its own signature.

```vow
fn f() -> () {
    print_str("hi");
}
```

**Fix:** Add the required effect to the function signature: `fn f() -> () [io]`.

### LinearTypeViolation

**Phase:** Type Checker
**Meaning:** A value of a `linear struct` type is used in a way that is immediately invalid before region inference runs, such as consuming it twice, consuming it inside a loop that may execute more than once, or consuming it after only some control-flow paths already consumed it.

```vow
linear struct Handle { fd: i64 }

fn f(h: Handle) -> Handle {
    let h2: Handle = h;
    let h3: Handle = h;  // h was already consumed
    h2
}
```

**Fix:** Restructure ownership so each path uses a consumed linear value at most once. Obligations that are simply left live at scope exit are reported later as `RegionLinear`.

### RegionLinear

**Phase:** Region Inference
**Meaning:** A `linear struct` value can remain live when its owning region closes. Returning the value transfers the linear obligation to the caller; consuming it before the close satisfies the obligation.

```vow
linear struct Handle { fd: i64 }

fn f() -> i64 {
    let h: Handle = Handle { fd: 1 };
    0
}
```

**Fix:** Consume the value before the region closes, or return it so the caller receives the obligation.

### NonExhaustiveMatch

**Phase:** Type Checker
**Meaning:** A `match` expression does not cover all possible variants.

```vow
fn f(o: Option<i64>) -> i64 {
    match o {
        Option::Some(x) => x,
    }
}
```

**Fix:** Add a `_ => ...` wildcard arm or cover all variants (`Option::None => ...`).

### UnsupportedPattern

**Phase:** Type Checker
**Meaning:** A parsed `match` pattern or scrutinee is not in the subset that
the compiler can lower safely. Match currently accepts enum-valued scrutinees,
qualified unit variants, qualified tuple variants with `_` or immutable
identifier payloads, and final catchall `_` or immutable identifier arms.

```vow
fn f(n: i64) -> i64 {
    match n {
        0 => 5,
        _ => 9,
    }
}
```

**Output:** `literal match patterns are not supported`

**Fix:** Use `if`/`else` comparisons for scalar or literal cases. For enum
payloads, bind each payload to `_` or an immutable identifier and inspect it
separately. Unsupported patterns fail before lowering and never produce an
executable.

### ImmutableAssignment

**Phase:** Type Checker
**Meaning:** A binding not declared `mut` was reassigned. Bindings are immutable
by default; `mut` is required only for whole-binding reassignment `x = e`. Field
writes (`s.f = e`) and index writes (`v[i] = e`) are allowed through any binding.

```vow
fn f() -> i64 {
    let x: i64 = 1;
    x = 2;
    x
}
```

**Fix:** Declare the binding `mut`: `let mut x: i64 = 1;`.

### UnusedMut

**Phase:** Type Checker
**Meaning:** A `let mut` binding is never reassigned, so the `mut` is dead. Only
whole-binding reassignment counts as a use of `mut` — a binding mutated solely
via `s.f = e`, `v[i] = e`, or a method call does not need `mut`.

```vow
fn f() -> i64 {
    let mut x: i64 = 1;
    x
}
```

**Fix:** Remove `mut`: `let x: i64 = 1;`.

### UnknownMethod

**Phase:** Type Checker
**Meaning:** A method call uses a name that does not exist on the receiver type.

```vow
fn f() -> () {
    let v: Vec<i64> = Vec::new();
    v.psh(42);
}
```

**Output:** `unknown method 'psh' on type 'Vec<i64>'`

**Fix:** Check the method name for typos. Use `--help` to see available methods for each type.

### UnsupportedFeature

**Phase:** Type Checker
**Meaning:** A language feature that is not supported in Vow was used.

```vow
trait Foo {
    fn bar() -> i64;
}
```

**Output:** `trait blocks are not supported in Vow`

**Fix:** Remove the unsupported construct. Vow does not support traits or impl blocks.

### BTreeMapKeyTypeMustBeI64

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with `K` not equal to `i64`. Phase 1 of the BTreeMap stdlib only supports `i64` keys; the runtime helpers and ESBMC C model are hard-coded to i64.

```vow
fn f() -> () {
    let m: BTreeMap<bool, i64> = BTreeMap::new();
    m.insert(true, 1);
}
```

**Output:** `BTreeMap key type must be i64; found 'bool'`

**Fix:** Use `BTreeMap<i64, V>`. If you need string or struct keys, hash or intern them to `i64` at the call site and keep a side-table for the originals.

### BTreeMapValueMustBeNonLinear

**Phase:** Type Checker
**Meaning:** A `BTreeMap<K, V>` was instantiated with a `V` that is or transitively contains a `linear struct`. Non-linear containers like `BTreeMap`, `Vec`, and `HashMap` cannot hold linear values because their internal shift/copy operations are bitwise and would silently duplicate the linear ownership obligation.

```vow
linear struct Token { id: i64 }
fn f() -> () {
    let m: BTreeMap<i64, Token> = BTreeMap::new();
}
```

**Output:** `BTreeMap value type must be non-linear; found 'Token'`

**Fix:** Either drop the `linear` qualifier on the struct, or keep handles in a `Vec<i64>` indirection and consume the linear values via direct function calls outside the map.

### MissingContract

**Phase:** Type Checker
**Meaning:** An `extern "C"` block was declared without a `vow { ... }` contract. Every foreign function call requires a mandatory contract specifying expected behavior.

```vow
extern "C" {
    fn write(fd: i32, ptr: i64, len: i64) -> i64 [io];
}
```

**Output:** `extern block requires a vow contract`

**Fix:** Add a `vow { ... }` block to the extern declaration with `requires` and/or `ensures` clauses.

### ContractTypeMismatch

**Phase:** Type Checker
**Meaning:** A `requires`, `ensures`, or `invariant` clause expression does not have type `bool`.

```vow
fn add(a: i64, b: i64) -> i64 vow {
    requires: a + b
} {
    a + b
}
```

**Output:** `` `requires` clause has type `i64` but must be `bool` ``

**Fix:** Ensure every contract clause is a boolean expression (comparison, logical operator, or a call to a predicate function returning `bool`).

### VowRequiresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs that violate a `requires` precondition. This is a **static** verification error — it means the function's callers can reach it with invalid arguments.

**Fix:** Strengthen the `requires` clause, or fix the callers to pass valid arguments.

### VowEnsuresViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found inputs where the function's return value does not satisfy the `ensures` postcondition.

**Fix:** Fix the function body to satisfy the postcondition, or weaken the `ensures` clause.

### VowInvariantViolated

**Phase:** Verification (ESBMC)
**Meaning:** ESBMC found a loop iteration where the `invariant` does not hold.

**Fix:** Strengthen the invariant or fix the loop body.

### EsbmcNotFound

**Phase:** Verification
**Meaning:** ESBMC is not installed or not on `$PATH`. When verification is enabled (the default for `vowc build`, always for `vowc verify`), the compiler checks for ESBMC upfront before compilation. If ESBMC is not found, the build aborts immediately with exit code 1.

**Fix:** Install ESBMC, or use `--no-verify` to skip verification: `vowc build --no-verify <file>`.

### RegionConflict

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Meaning:** A heap-typed value's required lifetime cannot be satisfied by the regions the surrounding code provides. This fires when an interprocedural store-effect constraint is unsatisfiable against the **inferred** region — that is, the value's `region(I) = LUB(must_outlive(I))` resolves to a concrete block strictly narrower than the target container's region.

> **Coverage note (as of issue #314):** the check is now semantic, consulting
> the inferred region populated by §4.1 step 3's LUB pass rather than the
> raw IR opcode. A fresh allocation routed through a callee's store-effect
> chain into a parameter container has its inferred region widened to
> `Caller(HiddenRegionIdx(N))` by §4.1 step 2's must-outlive marker
> propagation, where `N` is the precise slot index implied by the
> destination (issue #317 slot-aware inference). Such single-slot routings
> satisfy the constraint and are accepted. Allocations whose caller-region
> markers require more than one hidden caller-arena slot (for example, the
> value is stored into two distinct parameter targets, or returned and also
> stored into a parameter) have no single caller arena that outlives every
> destination, so their inferred region widens to the root region (`Root`) —
> a strictly wider placement than any one escaped pointer requires, hence
> sound (leak-but-safe) — and they compile without a blocking error (issue
> #871). Such a widen-to-root placement is not silent, though: it surfaces a
> non-blocking `RegionRootEscape` **note** (issue #366; see below), so the
> permanent root-region placement is still visible. `RegionConflict`
> therefore fires only when a value's inferred region is a concrete block
> strictly narrower than the target container's region.

```vow
fn store_into(out: Vec<String>, prefix: String) [io] {
    let s: String = String::from(prefix);
    s.push_str(String::from(" world"));
    out.push(s);  // s is allocated in this function's scope but escapes into out's region
}
```

**Fix:** Move the allocation to a wider scope, or copy the value into the target region (e.g., `String::from(s)` into the outer arena). For routings that compile cleanly but you'd like to know about (root-region placement), see `RegionRootEscape` below. See `docs/design/arena_memory.md` §4.4 for the full rejection vs. visibility distinction.

### RegionRootEscape

**Phase:** Region Inference (arena-per-scope, Phase 3)
**Severity:** Note (informational — does not fail the build)
**Meaning:** A heap allocation may land in the never-freed root region (`__vow_root_arena`). The note fires in either of two cases:

1. The allocation's inferred region is `Caller` and the surrounding function publishes a `FreshInCaller` return summary or store effect — so the value may flow up the caller chain to `main`.
2. The allocation's region **widens to the root region** without an intrinsic root pin (`pin_to_root` / a literal) — either because it is routed into more than one distinct hidden caller slot (a multi-slot widen), or because it reaches a container through a Phi over caller containers (a Phi widen). Both are sound (leak-but-safe) placements, but the allocation lives for the whole process.

This is a memory-cost decision the compiler surfaces visibly per `docs/design/arena_memory.md` §4.4: silent root-region placement caused growth-with-no-signal in earlier compiler versions, and the note restores that signal without conflating it with unsoundness (`RegionConflict`).

The note is conservative — it fires for any qualifying allocation in a function that could route to a caller or that widens to root, even if the actual concrete chain in this program doesn't reach `main`. False positives are tolerated because the diagnostic is non-blocking. A widen-to-root allocation is flagged even when it is also returned: being returned does not undo a root-region placement.

```json
{
  "error_code": "RegionRootEscape",
  "severity": "note",
  "message": "allocation may live in the root region: routed via store-effect chain to a caller whose target_region ultimately resolves to root",
  "hints": [
    "if intentional (e.g. program-lifetime data), no action needed; if you want this allocation freed earlier, restructure so the value is returned rather than stored into a parameter container"
  ]
}
```

**Fix:** Often none — if the program is short-lived (a checker, a CLI tool) or the values are genuinely program-lifetime, the note is informational. To free the allocation earlier, restructure so the value is **returned** from the constructing function rather than stored into a parameter container; the canonical `FreshInCaller` return path (`fn make_X() -> X`) does not trigger the note for the returned value or any allocation installed as a field of the returned struct (e.g. `Item { name: String::from("hi") }`). The exemption applies only to the *currently-installed* field initializers — a field overwritten before the return (`x.f = A; x.f = B; return x`) does not suppress the dead allocation `A`, which fires the note as expected (per-block last-write dedup, issue #326).

### VerificationSkipped

**Phase:** Verification (Warning surfaced alongside `BuildStatus::Skipped`)
**Meaning:** The function carries a `vow {}` block but its body uses opcodes the verifier's C model cannot represent — most commonly `RegionAlloc` and `FieldSet` produced by struct construction, also `Load`/`Store`, `RemF*`, and the `Linear*` family. The function is skipped before any C is emitted or ESBMC is invoked. The contract becomes documentary: runtime checks still apply in `--mode debug`, but no static proof is attempted.

```json
{
  "error_code": "VerificationSkipped",
  "severity": "warning",
  "message": "skipped verification of `ir_inst_set_region`: function `ir_inst_set_region` is not modelable in the verifier (contains unsupported opcode `RegionAlloc`)",
  "hints": [
    "the contract is documentary; runtime checks still apply in --mode debug"
  ]
}
```

**Why the build fails closed.** Per `CLAUDE.md`'s "Contract Authoring" guidance, contracts express semantic correctness and must not be weakened to fit the verifier. When the verifier's bounded model checker cannot represent a function's body, the function is skipped with a structured warning instead of tripping the defense-in-depth `__ESBMC_assert(0, "vow:UNSUPPORTED_OP_VOW_ID")` that historically broke the bootstrap on every vowed struct-builder. But a skipped contract is still an unproved contract, so the build lifts its overall status to `Skipped` (exit 1). Use `--no-verify` if you explicitly want a non-failing path that does not invoke ESBMC at all (`Unverified`, exit 0).

**Fix:** Refactor the function so its body uses only modelable opcodes — typically by splitting allocation/initialisation away from the contract-bearing computation. Alternatively, run with `--no-verify` if the contract is intentionally documentary.

## Runtime Errors

These are emitted to stderr as JSON when a compiled program runs (debug mode for VowViolation).

**Exit status.** Every runtime abort below terminates the process with the reserved exit status **`134`** (128 + `SIGABRT`, the conventional "aborted" status), never a plain `1`. A runtime abort is an environment or soundness failure, not an application result. `134` is **reserved for aborts by convention**: a runtime abort never *spontaneously* collides with an application's own `return N` from `main`, so a program that does not itself return — or `process_exit` — `134` can treat any `134` exit as a runtime abort (a checker that returns `0`/`1`/`2` for accepted/rejected/declined will never mistake an out-of-memory or a contract violation for a genuine "rejected"). The runtime does not *enforce* the reservation — `process_exit(134)` and `return 134i32` still exit `134` — so a program that deliberately uses `134` opts out of the distinction; applications that care should reserve around it. The JSON envelope on stderr still names the specific abort. This is separate from the *compiler* exit codes in [`cli.md`](cli.md), which describe `vowc build`/`vowc verify`.

### VowViolation

**When:** Debug mode only (`--mode debug`). A `requires`, `ensures`, or `invariant` predicate evaluates to false at runtime.

```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"divide.vow","offset":42,"values":{"y":0}}
```

The `blame` field indicates who is at fault:
- `Caller` — a `requires` was violated (the caller passed bad arguments)
- `Callee` — an `ensures` or `invariant` was violated (the function has a bug)

**Fix:** See the `description` and `values` fields to understand which predicate failed and with what runtime values.

### ArithmeticOverflow

**When:** A checked arithmetic operator (`+!`, `-!`, `*!`, `/!`, `%!`) overflows at runtime.

```json
{"error":"ArithmeticOverflow"}
```

**Fix:** Use wrapping arithmetic (`+`, `-`, etc.) if overflow is acceptable, or add bounds contracts to prevent overflow.

### UnwrapOnNone

**When:** `.unwrap()` is called on `Option::None`.

```json
{"error":"UnwrapOnNone"}
```

**Fix:** Use `match` to handle `None`, or add contracts that guarantee the value is `Some`.

### IndexOutOfBounds

**When:** A `Vec` index access (`v[i]` or `v[i] = val`) uses an index outside `0..v.len()`.

```json
{"error":"IndexOutOfBounds"}
```

**Fix:** Add a bounds check before indexing, or add contracts: `requires: i >= 0, requires: i < v.len()`.

### RegionLiteralMutation

**When:** A `Vec`, `String`, or `HashMap` mutation is attempted on a literal-backed container — one whose descriptor carries the `VOW_CAP_RODATA` sentinel (backing lives in `.rodata` or was pinned to the root region). Calls that statically trace a mutating target to a literal are rejected during compilation with this code; a runtime fallback emits the JSON shape below if an unchecked mutation reaches a `VOW_CAP_RODATA` descriptor. See `docs/design/arena_memory.md` §6.1, §7.3.

```json
{"error":"RegionLiteralMutation","operation":"String::push_str","origin":"rodata"}
```

A plain-text hint follows on the next line (not a JSON field). The hint text is dispatched on the operation's type prefix:

```
hint: make an explicit mutable copy with String::from(value) before mutating  # for String::* operations
hint: construct a mutable Vec and copy entries before mutating                # for Vec::*    operations
hint: construct a mutable HashMap and copy entries before mutating  # for HashMap::* operations
```

The `operation` field identifies the source-level method that trapped (e.g., `Vec::push`, `Vec::pop`, `HashMap::insert`, `String::clear`). The `origin` field identifies the storage class of the immutable backing; today only `rodata` is emitted.

**Fix:** Obtain an explicit mutable copy before mutation: `String::from(value)`, or construct a fresh mutable container and copy the entries you need before mutating.

### StackOverflow

**When:** The native call stack is exhausted, typically due to unbounded recursion.

```json
{"error":"StackOverflow"}
```

In debug or sanitize mode, the diagnostic includes call depth and the function that was executing when the overflow occurred:

```json
{"error":"StackOverflow","depth":10693,"function":"recurse"}
```

The signal handler is installed in **all** build modes. The `depth` and `function` fields are only available in debug/sanitize mode where call-depth instrumentation is emitted.

**Fix:** Add a base case to recursive functions, or restructure the algorithm to use iteration instead of recursion.

### OutOfMemory

**When:** A runtime arena operation (`__vow_arena_open` or `__vow_arena_alloc`) failed because the underlying `malloc` returned null. Non-recoverable from within Vow (`docs/design/arena_memory.md` §3.3, §16).

```json
{"error":"OutOfMemory","operation":"arena_alloc"}
```

The `operation` field is `arena_open` for the initial chunk allocation or `arena_alloc` for a later fallback chunk allocation.

Like every runtime abort, an OOM exits with the reserved status **`134`** (see *Exit status* above), so it is distinguishable from an application's own `exit 1`.

**Fix:** Reduce working-set size, raise the process memory limit, or run on a machine with more memory. This is not a Vow program error.

## Warnings

### LoweringWarning

**Phase:** IR Lowering
**Meaning:** The IR lowerer could not resolve a struct type tag or field name, defaulting to index 0. This usually indicates a missing type annotation on a `let` binding, causing the compiler to lose track of which struct type a pointer refers to.

**Fix:** Add an explicit type annotation: `let x: MyStruct = ...;` so the compiler can track struct type tags through the IR.
"#,
        ),
        (
            r#"reference/stdlib.md"#,
            r#"# Vow Standard Library

The standard library is a curated set of reusable, contract-annotated Vow modules
under `stdlib/`. Each module is a self-contained directory: one or more library
`.vow` files plus a `main.vow` that demonstrates and exercises the API.

This is a **reference collection**, not a globally-importable package set. Vow has
no module search path today (see [Consumption model](#consumption-model)). Modules
carry contracts, but only some are statically verifiable under the current ESBMC
model — read [Verification status](#verification-status) before relying on a
contract as a proof rather than a runtime check.

In all examples below, `vow` refers to `build/vowc`. Always run `ulimit -v 2000000`
before invoking the compiler or any binary it produces.

## Modules at a glance

| Module path           | Provides                                                                 | ESBMC status |
|-----------------------|-------------------------------------------------------------------------|---------------------------|
| `stdlib/math`         | `arithmetic`, `number_theory`, `vec_math` — integer & vector math        | VerifyFailed (env)        |
| `stdlib/heap`         | `min_heap`, `max_heap` — binary heaps over `i64`                          | VerifyFailed (env)        |
| `stdlib/stack`        | `stack` — Vec-backed LIFO stack over `i64`                               | Skipped     |
| `stdlib/geometry`     | `point`, `shape` — 2D points, circles, rectangles                        | **Verified**              |
| `stdlib/bignum`       | `bignum` — arbitrary-precision signed integers                          | Skipped     |
| `stdlib/gc`           | `gc` — mark-and-sweep garbage collector over `i64` slots                 | VerifyFailed              |

These are the `vow verify <module>/main.vow` results, measured against ESBMC 8.3.0. `(env)` marks an environmental verifier limitation, not a contract
defect. The statuses reflect the verifier's memory model, **not** the soundness of
the contracts — see [Verification status](#verification-status).

## Consumption model

`use` declarations resolve to a single directory: `use foo` loads `<dir>/foo.vow`,
where `<dir>` is the directory of the **entry file** passed to `vow build`/`vow verify`.
All transitive `use`s in dependency modules resolve against that **same** directory.
There is no search path, and `--module-root` is only available on `vow test` — not
`vow build` or `vow verify`.

Two practical ways to use a stdlib module:

**1. Run the module's own demo in place.** Each module ships a `main.vow`. Build with
`--no-verify` — most stdlib modules do not pass `vow verify` yet (see
[Verification status](#verification-status)), and the point here is to *run* the demo,
not to verify it:
```
$ ulimit -v 2000000; build/vowc build --no-verify stdlib/math/main.vow -o /tmp/math_demo
$ ulimit -v 2000000; /tmp/math_demo
```

**2. Copy the module's `.vow` file(s) into your project directory.** Because `use`
resolves against your entry file's directory, the library file must sit next to
your program. For a single-file module:
```
$ cp stdlib/math/arithmetic.vow myproject/arithmetic.vow
```
```vow
module Main
use arithmetic

fn main() -> i32 [io] {
    print_i64(clamp(15, 0, 10));   // 10
    0
}
```
For a multi-file module, copy **all** sibling files together — e.g. `stdlib/geometry`
ships `shape.vow` which internally does `use point`, so `point.vow` must be copied
alongside it.

> A real import mechanism (a module search path so `use std.math.arithmetic`
> resolves from any location) is future work. Until then, treat stdlib modules as
> vendored source you copy in, exactly like the self-hosted compiler's own modules.

## Verification status

The verifier statuses below were measured with `vow verify` against ESBMC 8.3.0.
They are **pre-existing properties of the code and the verifier**, unchanged by the
move into `stdlib/`. A `Skipped`/`VerifyFailed` status does not mean a contract is
wrong — in `--mode debug` every contract is still enforced at runtime via
`__vow_violation`.

| Module          | `vow verify` result | Why                                                                                                   |
|-----------------|---------------------|-------------------------------------------------------------------------------------------------------|
| `geometry`      | `Verified`          | The vowed shape functions use exact `i64` overflow bounds and are fully modelable. (`point_distance_sq` carries no contract, so it is not a proof obligation — see the geometry section.) |
| `math`          | `VerifyFailed`      | The old `abs`/`<stdlib.h>` collision is resolved — user functions are namespaced `vow_user_fn_<id>` in the emitted ESBMC model. The remaining blocker is genuine: `pow`'s `ensures result >= 0` is refuted by an `i64` overflow counterexample (a large `base`/`exp` wraps negative). A contract-hardening gap (overflow guard needed), not environmental. |
| `heap`          | `VerifyFailed`*     | A `Vec`-typed argument to a helper hits a C-model type mismatch; most heap functions are `Skipped` because `Vec`/region allocation (`RegionAlloc`) is not modelable. |
| `stack`         | `Skipped`           | `stack_push` allocates a `Vec` (`RegionAlloc`), which the verifier cannot model; contracts are documentary. |
| `bignum`        | `Skipped`           | `Vec`-based limb arithmetic allocates per call (`RegionAlloc`); not modelable. 24 `RegionRootEscape` notes (the demo intentionally holds results for program lifetime). |
| `gc`            | `VerifyFailed`      | ESBMC produces a `gc_add_root` precondition counterexample related to in-module caller-`requires` checking (cf. issue #764). |

\* Environmental verifier limitation, not a contract defect.

**Takeaway for agents:** only `geometry`'s `vow verify` passes today — and that proves
the *vowed* checks reachable from its demo, not every function (e.g. `point_distance_sq`
carries no contract and is not a proof obligation). For
the others, the contracts are precise specifications that are enforced at runtime in
`--mode debug`; static proof is gated on verifier-model improvements (Vec/region
modeling and the #764 caller-`requires` fix). When you build
on these modules and need a *static* guarantee, prefer `geometry`'s pattern: keep
hot paths in plain `i64` with explicit overflow `requires`.

---

## math

Three modules under `stdlib/math/`. Each is independent (no cross-`use`); copy only
the one you need. All functions are `pub`.

### math.arithmetic

Integer primitives with overflow-guarded contracts. The `safe_*` family operates on
**non-negative** inputs only — they are overflow-checked unsigned-style helpers, not
general signed wrappers.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `abs` | `(x: i64) -> i64` | `requires x > -9223372036854775807`; `ensures result >= 0`; `ensures result == x \|\| result == 0 - x` | Guards `i64::MIN` negation overflow. |
| `min` | `(a, b: i64) -> i64` | `ensures result <= a`; `result <= b`; `result == a \|\| result == b` | Tight: result is one of the inputs. |
| `max` | `(a, b: i64) -> i64` | `ensures result >= a`; `result >= b`; `result == a \|\| result == b` | |
| `clamp` | `(x, lo, hi: i64) -> i64` | `requires lo <= hi`; `ensures lo <= result <= hi` | |
| `sign` | `(x: i64) -> i64` | `ensures -1 <= result <= 1` | -1 / 0 / 1. |
| `safe_add` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a <= I64_MAX - b`; `ensures result == a + b` | Non-negative inputs only. |
| `safe_sub` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a >= b`; `ensures 0 <= result <= a` | |
| `safe_mul` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, b == 0 \|\| a <= I64_MAX / b`; `ensures result == a * b` | |
| `safe_div` | `(a, b: i64) -> i64` | `requires a >= 0, b > 0`; `ensures 0 <= result <= a` | `b > 0`, not just `b != 0`. |
| `safe_mod` | `(a, b: i64) -> i64` | `requires a >= 0, b > 0`; `ensures 0 <= result < b` | |
| `pow` | `(base, exp: i64) -> i64` | `requires base >= 0, exp >= 0`; `ensures result >= 0` | O(exp) — no fast exponentiation; no overflow guard on the running product. |
| `midpoint` | `(a, b: i64) -> i64` | `requires a >= 0, a <= b`; `ensures a <= result <= b` | Overflow-safe `a + (b-a)/2`. |
| `diff` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0`; `ensures result >= 0` | `|a - b|`. |
| `divides` | `(d, n: i64) -> bool` | `requires d != 0` | |
| `is_even` / `is_odd` | `(x: i64) -> bool` | — | |

Representative contract — overflow guard expressed in the precondition rather than
via checked arithmetic:
```vow
pub fn safe_mul(a: i64, b: i64) -> i64 vow {
    requires: a >= 0,
    requires: b >= 0,
    requires: b == 0 || a <= 9223372036854775807 / b,
    ensures: result == a * b,
    ensures: result >= 0
}
```

### math.number_theory

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `gcd` | `(a, b: i64) -> i64` | `requires a >= 0, b >= 0, a > 0 \|\| b > 0`; `ensures result > 0` | Euclid; loop invariants `x >= 0, y >= 0`. |
| `lcm` | `(a, b: i64) -> i64` | `requires a > 0, b > 0`; `ensures result > 0` | No overflow guard on `(a/g)*b`. |
| `is_prime` | `(n: i64) -> bool` | `requires n >= 0` | Trial division to `i*i <= n`. |
| `power_mod` | `(base, exp, modulus: i64) -> i64` | `requires base >= 0, exp >= 0, modulus > 1, modulus <= 3037000499`; `ensures 0 <= result < modulus` | Modulus bound = `isqrt(I64_MAX)`, prevents `(r*b)` overflow. |
| `factorial` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 1` | No upper bound on `n` — product overflows past 20!. |
| `fibonacci` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0` | Iterative; overflows past F(92). |
| `isqrt` | `(n: i64) -> i64` | `requires n >= 0`; `ensures result >= 0, result*result <= n` | Floor integer sqrt; postcondition is the real spec. |
| `largest_divisor` | `(n: i64) -> i64` | `requires n > 1`; `ensures 1 <= result < n` | Largest proper divisor. |
| `count_divisors` | `(n: i64) -> i64` | `requires n > 0`; `ensures result >= 1` | |

### math.vec_math

Operates on `Vec<i64>`. None of the summation helpers guard against accumulator
overflow — use on bounded data, or add `requires` bounds at the call site.

| Function | Signature | Key contracts | Notes |
|----------|-----------|---------------|-------|
| `vec_sum` | `(v: Vec<i64>) -> i64` | — | No overflow guard. |
| `vec_min` / `vec_max` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | |
| `vec_mean` | `(v: Vec<i64>) -> i64` | `requires v.len() > 0` | Integer mean. |
| `vec_dot` | `(a, b: Vec<i64>) -> i64` | `requires a.len() == b.len()` | |
| `vec_count` | `(v: Vec<i64>, target: i64) -> i64` | `ensures 0 <= result <= v.len()` | Invariant `count <= i`. |
| `vec_all_in_range` | `(v: Vec<i64>, lo, hi: i64) -> bool` | `requires lo <= hi` | |
| `vec_is_sorted` | `(v: Vec<i64>) -> bool` | — | Ascending. |
| `vec_prefix_sum` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |
| `vec_reverse` | `(v: Vec<i64>) -> Vec<i64>` | `ensures result.len() == v.len()` | |

---

## heap

`stdlib/heap/min_heap.vow` and `max_heap.vow` are structural mirrors (a min-heap and
a max-heap over `i64`), with the comparator flipped. Both are value types: every
mutator takes a heap by value and returns a new one.

The defining contract pattern is the **size-shadow invariant** `size == data.len()`,
threaded through every mutator. This is what lets ESBMC reason about in-bounds
`data[i]` access without a universal quantifier:
```vow
pub fn min_heap_push(h: MinHeap, val: i64) -> MinHeap vow {
    requires: h.size == h.data.len(),
    requires: h.size < 9223372036854775807,
    ensures: result.size == h.size + 1,
    ensures: result.size == result.data.len()
}
```

| Function (min; `max_*` mirrors) | Signature | Key contracts |
|---------------------------------|-----------|---------------|
| `min_heap_new` | `() -> MinHeap` | `ensures result.size == 0, result.data.len() == 0` |
| `min_heap_len` | `(h) -> i64` | `ensures result == h.size` |
| `min_heap_is_empty` | `(h) -> bool` | `ensures result == (h.size == 0)` |
| `min_heap_push` | `(h, val: i64) -> MinHeap` | size-shadow in/out; `ensures result.size == h.size + 1` |
| `min_heap_peek` | `(h) -> i64` | `requires h.size > 0, size-shadow`; `ensures result == h.data[0]` |
| `min_heap_pop` | `(h) -> MinHeap` | `requires h.size > 0, size-shadow`; `ensures result.size == h.size - 1` |
| `min_heap_clear` | `(h) -> MinHeap` | size-shadow in; `ensures result.size == 0` |
| `is_min_heap` | `(h) -> bool` | `requires size-shadow` — runtime check of the heap-order property |

**Heap-order is a runtime predicate, by design.** Vow has no universal quantifier, so
the property `∀i. data[parent(i)] <= data[i]` cannot be written as an `ensures`.
`is_min_heap` / `is_max_heap` check it at runtime instead; the static contracts cover
index safety and the size-shadow invariant only.

---

## stack

`stdlib/stack/stack.vow` — a `Vec<i64>`-backed LIFO stack (value type). `node.vow` in
the same directory is a vestigial `Node` struct kept for the demo; the stack does not
use it.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `stack_new` | `() -> Stack` | — |
| `stack_push` | `(s, val: i64) -> Stack` | `ensures result.size == s.size + 1` |
| `stack_peek` | `(s) -> i64` | `requires s.size > 0` |
| `stack_size` | `(s) -> i64` | — |
| `stack_is_empty` | `(s) -> bool` | — |

**Known gaps (move-verbatim; tracked follow-up):** no `stack_pop`; no size-shadow
invariant (`size == data.len()`) like `heap` has; `stack_peek` has no `ensures`
relating the result to `data[size-1]`; functions are not marked `pub`; `node.vow` is
unused.

---

## geometry

`stdlib/geometry/point.vow` (a `Point` struct) and `shape.vow` (a `Shape` enum with
circle/rectangle area and perimeter). **The only module whose `vow verify` passes
today** — its shape functions use exact derived overflow bounds. Note this means the
*vowed* checks verify (`vow verify stdlib/geometry/main.vow` → `Verified`); it is not a
proof of the whole API, since `point_distance_sq` carries no contract (see Known gaps).

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `point_new` / `point_x` / `point_y` | `Point` accessors | — |
| `point_distance_sq` | `(a, b: Point) -> i64` | — (no overflow guard — gap for large coordinates) |
| `circle_area` | `(r: i64) -> i64` | `requires 0 <= r <= 1753413056`; `ensures result >= 0` |
| `rect_area` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, h == 0 \|\| w <= I64_MAX / h`; `ensures result >= 0` |
| `circle_perimeter` | `(r: i64) -> i64` | `requires 0 <= r <= 1537228672809129301`; `ensures result >= 0` |
| `rect_perimeter` | `(w, h: i64) -> i64` | `requires w >= 0, h >= 0, w <= 4611686018427387903 - h`; `ensures result >= 0` |

Each magic bound is the exact threshold below which the arithmetic cannot overflow —
e.g. `circle_area` caps `r` at `floor(sqrt(I64_MAX/3))` because it computes `r*r*3`:
```vow
fn circle_area(r: i64) -> i64 vow {
    requires: r >= 0,
    requires: r <= 1753413056,
    ensures: result >= 0
}
```

**Known gaps:** the `Shape` enum is declared but the area/perimeter functions are
free functions that don't dispatch on it; `point_distance_sq` lacks an overflow
guard; `shape_at` is a demo artifact, not a real API.

---

## bignum

`stdlib/bignum/bignum.vow` — arbitrary-precision **signed** integers with a
small-int fast path (`enum BigNum { Small(i64), Big(BigMag) }`). `Small(v)` holds
any value fitting in `i64` with **no heap allocation**; `Big(m)` holds a `BigMag`
magnitude (base 2³² limbs, `Vec<u64>`, sign-magnitude) for `|value| > i64::MAX`.
Pure core language; no builtins beyond `Vec`/`String`/`u64`/`i64`. A non-negative
`BigNum` is the natural number (`Nat`) an arbitrary-precision `Nat` consumer needs;
the binary limb base makes the bitwise operations trivial limb-wise ops, which is
why this module can back a proof kernel's `Nat` / `BitVec` reductions past the 2⁶⁴
ceiling (issue #838). The fast path measured ~3–5× faster and ~40× less peak
memory on small-op-heavy loops vs. the always-allocating representation.

**Public API (selected):**
- Construct: `bignum_zero`, `bignum_from_i64`, `bignum_from_u64`, `bignum_from_string`
- Convert: `bignum_to_string`, `bignum_to_u64` (`Option<u64>`; `None` if negative or > u64)
- Predicates: `bignum_is_zero`, `bignum_is_negative`, `bignum_is_positive`
- Compare: `bignum_cmp`, `bignum_cmp_abs`, `bignum_eq`, `bignum_lt`, `bignum_gt`, `bignum_le`, `bignum_ge`
- Arithmetic: `bignum_negate`, `bignum_abs`, `bignum_add`, `bignum_sub`, `bignum_monus`, `bignum_mul`, `bignum_div`, `bignum_mod`, `bignum_divmod`
- Bitwise (on magnitude): `bignum_and`, `bignum_or`, `bignum_xor`, `bignum_shl`, `bignum_shr`
- Higher-level: `bignum_pow(base, exp: i64)`, `bignum_gcd`, `bignum_factorial(n: i64)`

**Contracts present:** `bignum_div`/`bignum_mod`/`bignum_divmod` require
`!bignum_is_zero(b)`; `bignum_pow` requires `exp >= 0`; `bignum_shl`/`bignum_shr`
require `n >= 0`; `bignum_factorial` requires `n >= 0` (internal `bigmag_sub_abs`
requires `bigmag_cmp_abs(a, b) >= 0`).

**Semantics to know:**
- **Canonicalization invariant:** a value fits `i64` ⟺ it is `Small`. Every
  result-producing op returns through `bignum_normalize`, which demotes a `Big`
  magnitude back to `Small` when it fits — so `cmp`/`eq`/`to_string` never see two
  encodings of one value. The `BigMag` magnitude keeps the usual limb invariant
  (non-empty, no leading-zero limbs except canonical zero `[0]`, `sign ∈ {-1, 1}`,
  each limb `< 2³²`); none of this is stated as a struct invariant or `ensures`.
- The `Small`/`Small` fast path uses conservative magnitude bounds (`2⁶²−1` for
  `add`/`sub`/`monus`, `2³¹` for `mul`) so it never overflows `i64`; values outside
  the bound fall to the limb path and re-normalize. Result correctness is identical
  to the all-`Big` representation.
- Division truncates toward zero; the remainder's sign matches the dividend.
- `bignum_monus` is truncated (Nat) subtraction — `max(a − b, 0)`, saturating at 0.
- `bignum_to_u64` returns `Option::None` when the value is negative or exceeds `u64`.
- Bitwise `and`/`or`/`xor` act on the **magnitude** (Nat semantics) and return a
  non-negative result; `shl`/`shr` shift the magnitude and preserve the sign
  (= multiply / floor-divide by 2ⁿ; a logical bit shift for non-negative operands).
- `bignum_pow`/`bignum_factorial` take a native `i64` exponent/argument, not a BigNum.
- `bignum_gcd` operates on absolute values; the result is non-negative.
- Multiplication is O(n·m) schoolbook (no Karatsuba).
- The limb algorithms live in internal `bigmag_*` functions over the `BigMag`
  magnitude (`bigmag_add`, `bigmag_mul`, `bigmag_divmod`, `bigmag_strip_zeros`,
  `bigmag_to_string`, …); the public `bignum_*` API wraps them with the fast path
  and `bignum_normalize`. `bignum_to_bigmag`/`bignum_normalize`/`u64_to_decimal*`
  and the `bigmag_*` set are internal, not part of the public API.

**Verification:** `Skipped` — limb arithmetic allocates `Vec`s per call (`RegionAlloc`),
which the verifier cannot model. Contracts are runtime-enforced in `--mode debug`.

---

## gc

`stdlib/gc/gc.vow` — a mark-and-sweep garbage collector over a heap of `i64` values
with explicit roots and reference edges (`struct GcHeap`). Slots are opaque integer
handles returned by `gc_alloc`; never fabricate them.

| Function | Signature | Key contracts |
|----------|-----------|---------------|
| `gc_new` | `() -> GcHeap` | — |
| `gc_alloc` | `(h, val: i64) -> i64` | — (returns a slot; reuses freed slots) |
| `gc_add_root` | `(h, slot: i64)` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_remove_root` | `(h, slot: i64)` | `requires 0 <= slot < values.len()` (does **not** require alive — you may unroot a freed slot) |
| `gc_add_ref` | `(h, from, to: i64)` | `requires` both in range and alive |
| `gc_read` | `(h, slot: i64) -> i64` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_write` | `(h, slot, val: i64)` | `requires 0 <= slot < values.len(), alive[slot] == 1` |
| `gc_is_alive` | `(h, slot: i64) -> bool` | `requires 0 <= slot < values.len()` |
| `gc_count` | `(h) -> i64` | — |
| `gc_collect` | `(h) -> i64` | — (returns count of newly-freed objects) |

**Semantics to know:**
- `gc_collect` invalidates every slot not reachable from a root; calling
  `gc_read`/`gc_write` on a freed slot violates its precondition.
- Roots and references are not deduplicated — adding a root twice needs two
  `gc_remove_root` calls.
- The heap stores only `i64`; represent richer object graphs as indices/tagged ints.
- Mark/sweep handles cycles naturally via the mark bit; no separate cycle detection.

**Verification:** `VerifyFailed` — ESBMC produces a `gc_add_root` precondition
counterexample tied to how in-module caller-`requires` are checked (cf. issue #764).
Contracts are runtime-enforced in `--mode debug`.

---

## Known gaps and roadmap

These are tracked follow-ups, intentionally **not** addressed by the reorg that
created `stdlib/` (which moved code verbatim):

- **Static verifiability.** Make `Vec`/region-allocating functions modelable so
  `stack`, `bignum`, and most of `heap` can be statically verified; resolve the
  `gc_add_root` caller-`requires` counterexample (#764).
- **Contract hardening.** Add struct/representation invariants and `ensures` clauses
  to `bignum` and `gc`; add a size-shadow invariant and `stack_pop` to `stack`; add
  an overflow guard to `point_distance_sq` and to `pow` (so its `ensures result >= 0`
  holds under `i64`); wire the `Shape` enum into `geometry`'s area/perimeter functions.
- **Consistency.** Mark all intended-public functions `pub` (currently only `math`
  and `heap` do); remove or rebuild the vestigial `stack/node.vow`.
- **Distribution.** A module search path so stdlib modules can be imported without
  copying source into the consuming project.
"#,
        ),
        (
            r#"examples/examples.md"#,
            r#"# Worked Examples

Verification workflow examples. The first three demonstrate Counterexample-Guided Inductive Synthesis (CEGIS) cycles: write spec, build, read JSON, diagnose, fix, verify. The fourth shows break-with-value in loop expressions. The fifth shows an EOF-safe interactive command loop using `stdin_read_line()`. The sixth shows bounded-memory streaming file input.

## 1. Safe Division — Requires Pattern

### Goal

Write a division function that is safe (cannot divide by zero).

### Step 1: Write the spec

```vow
module Divide

fn divide(x: i64, y: i64) -> i64 vow {
    requires: y != 0
} {
    x / y
}

fn main() -> i32 [io] {
    divide(10, 0);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/divide.vow
```

```json
{"status":"Verified","executable":"examples/divide","diagnostics":[],"counterexamples":[]}
```

ESBMC proves the contract: whenever `y != 0` holds, the division is safe.

### Step 3: Runtime behavior (debug mode)

The `main()` calls `divide(10, 0)` which violates `requires: y != 0`. In debug mode:

```
$ vow build --mode debug --no-verify examples/divide.vow -o /tmp/divide_debug
$ /tmp/divide_debug
```

Stderr:
```json
{"error":"VowViolation","vow_id":0,"blame":"Caller","description":"y != 0","file":"examples/divide.vow","offset":56,"values":{"y":0}}
```

The `blame: "Caller"` tells you: `main()` passed `y=0`, which violates the precondition.

---

## 2. CEGIS Broken → Fixed — The Core Workflow

### Goal

Write `safe_sub(a, b)` that always returns a non-negative result.

### Step 1: Initial attempt (broken)

```vow
module CegisBroken

fn safe_sub(a: i64, b: i64 where b >= 0) -> i64 vow {
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 2: Build

```
$ vow build examples/cegis_broken.vow
```

```json
{
  "status": "VerifyFailed",
  "executable": "examples/cegis_broken",
  "diagnostics": [],
  "function": "safe_sub",
  "counterexample": "[Counterexample]",
  "counterexamples": [
    {
      "function": "safe_sub",
      "inputs": { "a": "-9223372036854775808", "b": "0" },
      "violation": "ensures result >= 0",
      "vow_id": 1,
      "source": { "file": "examples/cegis_broken.vow", "offset": 76, "length": 20 }
    }
  ]
}
```

### Step 3: Diagnose

The counterexample shows `a = -9223372036854775808` (i64 min), `b = 0`. Then `a - b = a`, which is negative. The `ensures: result >= 0` is violated.

**Root cause:** We need `a >= b` to guarantee a non-negative result, and `a >= 0` to prevent negative inputs.

### Step 4: Fix

```vow
module CegisFixed

fn safe_sub(a: i64 where a >= 0, b: i64 where b >= 0) -> i64 vow {
    requires: a >= b,
    ensures: result >= 0
} {
    a - b
}

fn main() -> i32 [io] {
    print_i64(safe_sub(10, 3));
    0
}
```

### Step 5: Verify

```
$ vow build examples/cegis_fixed.vow
```

```json
{"status":"Verified","executable":"examples/cegis_fixed","diagnostics":[],"counterexamples":[]}
```

Verified. With `a >= 0`, `b >= 0`, and `a >= b`, ESBMC proves `result >= 0`.

---

## 3. Vec Fill — Loop Invariant

### Goal

Fill a vector with `n` elements and prove its length equals `n`.

### Step 1: Write the spec

```vow
module VecFill

fn fill_vec(n: i64) -> Vec<i64> vow {
    requires: n >= 0,
    requires: n <= 8,
    ensures: result.len() == n
} {
    let v: Vec<i64> = Vec::new();
    let mut i: i64 = 0;
    while i < n vow {
        invariant: i >= 0,
        invariant: i <= n
    } {
        v.push(i);
        i = i + 1;
    }
    v
}

fn main() -> i32 [io] {
    let v: Vec<i64> = fill_vec(5);
    print_i64(v.len());
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/vec_fill.vow
```

```json
{"status":"Verified","executable":"examples/vec_fill","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `requires: n <= 8` keeps iterations tractable for verification
- `invariant: i >= 0, invariant: i <= n` is inductive: true on entry, preserved by the loop body
- The Vec model tracks `len`, so ESBMC can reason about `result.len() == n`

---

## 4. Linear Search — Break-with-Value

### Goal

Search a vector for a target value and return its index, or `-1` if not found. Uses `loop` with `break <value>` to produce a result directly from the loop expression.

### Step 1: Write the spec

```vow
module Search

fn linear_search(data: Vec<i64>, target: i64) -> i64
    vow { requires: data.len() > 0 }
{
    let mut i: i64 = 0;
    let n: i64 = data.len();
    let result: i64 = loop {
        if i >= n {
            break -1;
        }
        if data[i] == target {
            break i;
        }
        i = i + 1;
    };
    result
}

fn main() -> i32 [io] {
    let data: Vec<i64> = Vec::new();
    data.push(10);
    data.push(20);
    data.push(30);
    data.push(40);
    data.push(50);

    let idx: i64 = linear_search(data, 30);
    print_i64(idx);

    let idx2: i64 = linear_search(data, 99);
    print_i64(idx2);
    0
}
```

### Step 2: Build and verify

```
$ vow build examples/search.vow
```

```json
{"status":"Verified","executable":"examples/search","diagnostics":[],"counterexamples":[]}
```

**Key points:**
- `loop { ... break <value>; ... }` is an expression that evaluates to the break value
- All `break` expressions in a `loop` must produce the same type (`i64` here)
- `break <value>` is only allowed in `loop`, not in `while` (which always evaluates to `()`)
- The result is bound with `let result: i64 = loop { ... };`

---

## 5. Command Loop — EOF-Safe `stdin_read_line`

### Goal

Write a line-oriented command interpreter that reads from stdin, dispatches commands, skips empty lines, and exits cleanly on EOF. This is the canonical pattern for CI-safe interactive programs.

### Step 1: Write the program

```vow
module CmdLoop

fn trim_newline(s: String) -> String {
    let n: i64 = s.len();
    if n == 0 { return s; }
    let last: i64 = s.byte_at(n - 1);
    if last == 10 {
        if n >= 2 {
            let prev: i64 = s.byte_at(n - 2);
            if prev == 13 {
                return s.substring(0, n - 2);
            }
        }
        return s.substring(0, n - 1);
    }
    s
}

fn skip_spaces(s: String, start: i64) -> i64 {
    let mut i: i64 = start;
    let n: i64 = s.len();
    while i < n {
        if s.byte_at(i) != 32 { return i; }
        i = i + 1;
    }
    i
}

fn main() -> i32 [read, io] {
    let mut line: String = stdin_read_line();
    while line.len() > 0 {
        let cmd: String = trim_newline(line);

        if cmd.len() > 0 {
            if cmd.eq(String::from("quit")) {
                return 0;
            }

            if cmd.eq(String::from("hello")) {
                print_str(String::from("Hello, world!\n"));
            } else {
                if cmd.len() >= 5 {
                    let prefix: String = cmd.substring(0, 5);
                    if prefix.eq(String::from("echo ")) {
                        let start: i64 = skip_spaces(cmd, 5);
                        let text: String = cmd.substring(start, cmd.len());
                        print_str(text);
                        print_str(String::from("\n"));
                    } else {
                        print_str(String::from("unknown: "));
                        print_str(cmd);
                        print_str(String::from("\n"));
                    }
                } else {
                    print_str(String::from("unknown: "));
                    print_str(cmd);
                    print_str(String::from("\n"));
                }
            }
        }

        line = stdin_read_line();
    }
    0
}
```

### Step 2: Build

```
$ vow build --no-verify examples/cmdloop.vow -o /tmp/cmdloop
```

```json
{"status":"Unverified","executable":"/tmp/cmdloop","diagnostics":[],"counterexamples":[]}
```

No contracts here — this example focuses on the I/O pattern, not verification.

### Step 3: Run with piped input

```
$ printf 'hello\necho Vow is great\n\nbogus\nquit\n' | /tmp/cmdloop
Hello, world!
Vow is great
unknown: bogus
```

The `quit` command causes an early `return 0`. Empty lines are silently skipped.

### Step 4: Run with EOF (no quit)

```
$ printf 'hello\necho test\n' | /tmp/cmdloop
Hello, world!
test
```

When stdin is exhausted, `stdin_read_line()` returns `""` (length 0), the `while` condition fails, and the program exits cleanly with code 0.

### Key points

- **EOF detection:** `stdin_read_line()` returns `""` at EOF. Check `.len() > 0` to exit the loop.
- **Newline stripping:** `stdin_read_line()` includes the trailing `\n` (or `\r\n`). Strip it with `byte_at` + `substring` before comparing commands.
- **Empty line handling:** After trimming, `cmd.len() == 0` means the line was blank — skip it.
- **Effects:** `stdin_read_line()` requires `[read]`; `print_str()` requires `[io]`. The `main` function declares both.
- **CI-safe:** No blocking reads, no prompts — the program processes whatever stdin provides and exits at EOF. Safe to run in pipelines and test harnesses.

## 6. Streaming File Input

`fs_read(path)` materializes the entire file as one `String`. Use `fs_open` plus `fs_read_line` for newline-delimited files that may be large.

```vow
module StreamingFile

fn main() -> i32 [read, io] {
    let argv: Vec<String> = args();
    if argv.len() < 2 {
        eprintln_str(String::from("usage: streaming_file <path>"));
        return 1;
    }

    let h: i64 = fs_open(argv[1]);
    if h < 0 {
        eprintln_str(String::from("could not open input"));
        return 1;
    }

    let mut lines: i64 = 0;
    let mut bytes: i64 = 0;
    let mut line: String = fs_read_line(h);
    while line.len() > 0 {
        lines = lines + 1;
        bytes = bytes + line.len();
        line = fs_read_line(h);
    }

    if fs_status(h) != 1 {
        fs_close(h);
        eprintln_str(String::from("read error"));
        return 1;
    }
    if fs_close(h) != 0 {
        eprintln_str(String::from("close error"));
        return 1;
    }

    print_i64(lines);
    print_str(String::from("\n"));
    print_i64(bytes);
    print_str(String::from("\n"));
    0
}
```

Key points:

- `fs_read_line(handle)` includes the trailing newline when present.
- Blank lines are returned as `"\n"`; EOF returns `""`.
- Check `fs_status(handle)` after `fs_read_line(handle)` returns `""`: `1` means EOF, `-1` means invalid handle or read error.
- Close each successful handle with `fs_close(handle)` and check for a non-zero close result.

## 7. BTreeMap basic usage

`BTreeMap<i64, V>` is the deterministic alternative to `HashMap` — sorted ascending by key, binary-search lookup. Use it when iteration order affects program output (codegen, serialization, or any reproducible build).

```vow
module BTreeMapExample

fn fetch(m: BTreeMap<i64, i64>) -> Option<i64> [io] {
    let r: Option<i64> = m.get(7);
    let v: i64 = r?;
    print_i64(v);
    print_str(String::from("\n"));
    Option::Some(v)
}

fn main() -> i32 [io] {
    let m: BTreeMap<i64, i64> = BTreeMap::new();
    m.insert(7, 42);
    let prev: Option<i64> = m.insert(7, 99);
    // prev is Some(42); the second insert overwrote the first.
    fetch(m);
    print_i64(m.len());
    0
}
```

Note that `.insert` returns `Option<V>` (the previous value, if any), and `.get` returns `Option<V>`. Use `?` to short-circuit on `None`. Phase 1 only supports `i64` keys; using any other key type raises `BTreeMapKeyTypeMustBeI64`.

### Why BTreeMap and not HashMap

`HashMap.insert` returns `()` and its iteration order is unspecified. For maps whose iteration is observable in the output binary, the byte-identical bootstrap requirement (`stage1 == stage2` sha256) demands deterministic order. `BTreeMap` provides it; `HashMap` does not.
"#,
        ),
        (
            r#"schemas/build-result.schema.json"#,
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/build-result.schema.json",
  "title": "BuildResult",
  "description": "JSON output from `vow build` on stdout",
  "type": "object",
  "required": ["status", "executable", "diagnostics", "counterexamples"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["Verified", "Unverified", "Skipped", "CompileFailed", "VerifyFailed"],
      "description": "Build outcome. `Skipped` means ESBMC was invoked but at least one vowed function could not be modelled; the build fails closed with exit 1 (distinct from `Unverified`, which means ESBMC was not invoked, e.g. `--no-verify`/`--dump-ir`, exit 0)."
    },
    "executable": {
      "type": ["string", "null"],
      "description": "Path to compiled binary, or null on failure or when source has no main function (library module)"
    },
    "diagnostics": {
      "type": "array",
      "items": { "$ref": "diagnostic.schema.json" },
      "description": "Compiler diagnostics (parse errors, type errors, vow violations)"
    },
    "message": {
      "type": "string",
      "description": "Error detail (present only when status is CompileFailed)"
    },
    "function": {
      "type": "string",
      "description": "Function name (present only when status is VerifyFailed)"
    },
    "counterexample": {
      "type": "string",
      "description": "Legacy counterexample description (present only when status is VerifyFailed)"
    },
    "counterexamples": {
      "type": "array",
      "items": { "$ref": "counterexample.schema.json" },
      "description": "Structured counterexamples from ESBMC verification"
    },
    "verify_status": {
      "type": "string",
      "enum": ["timeout", "unknown", "error", "tool_not_found", "panicked"],
      "description": "Verification sub-status (present only when the verification backend did not produce a proof or counterexample; \"panicked\" signals the verifier worker thread crashed and the build fails closed)"
    },
    "verify_message": {
      "type": "string",
      "description": "Verification backend error detail (present only when verify_status is set)"
    }
  },
  "allOf": [
    {
      "if": { "properties": { "status": { "const": "CompileFailed" } } },
      "then": { "required": ["message"] }
    },
    {
      "if": { "properties": { "status": { "const": "VerifyFailed" } } },
      "then": { "required": ["function", "counterexample"] }
    }
  ],
  "additionalProperties": false
}
"#,
        ),
        (
            r#"schemas/complexity-result.schema.json"#,
            r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.org/schemas/complexity-result.schema.json",
  "title": "ComplexityReport",
  "description": "Output of `vow complexity <file>` on stdout. Byte-identical across the Rust and self-hosted compilers. Non-integer metrics are fixed-3-decimal numbers computed in integer fixed-point (scale 1000), never native floats.",
  "type": "object",
  "required": ["schema_version", "kind", "tool", "files", "summary"],
  "properties": {
    "schema_version": { "const": "1" },
    "kind": { "const": "complexity_report" },
    "tool": { "const": "vow" },
    "files": {
      "type": "array",
      "items": { "$ref": "#/$defs/File" }
    },
    "summary": { "$ref": "#/$defs/Summary" }
  },
  "additionalProperties": false,
  "$defs": {
    "File": {
      "type": "object",
      "required": ["file", "complexity_score", "functions_over_threshold", "nloc", "functions", "module"],
      "properties": {
        "file": { "type": "string", "description": "Source path as passed on the command line." },
        "complexity_score": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Max complexity_score across the file's functions." },
        "functions_over_threshold": { "type": "integer", "minimum": 0, "description": "Count of functions whose score exceeds the threshold (--max-score if passed, else the recommended 80)." },
        "nloc": { "type": "integer", "minimum": 0, "description": "Source line count of the file." },
        "functions": { "type": "array", "items": { "$ref": "#/$defs/Function" } },
        "module": { "$ref": "#/$defs/Module" }
      },
      "additionalProperties": false
    },
    "Module": {
      "description": "Module-level coupling aggregates (experimental tier). fan_in counts in-file callers only.",
      "type": "object",
      "required": ["tier", "functions", "fan_in_max", "fan_out_max", "henry_kafura_max"],
      "properties": {
        "tier": { "const": "experimental" },
        "functions": { "type": "integer", "minimum": 0 },
        "fan_in_max": { "type": "integer", "minimum": 0, "description": "Max in-file callers of any function." },
        "fan_out_max": { "type": "integer", "minimum": 0, "description": "Max distinct callees of any function." },
        "henry_kafura_max": { "type": "integer", "minimum": 0, "description": "Max nloc*(fan_in*fan_out)^2, saturated. Unvalidated." }
      },
      "additionalProperties": false
    },
    "Vow": {
      "description": "Vow-surface metrics (experimental tier).",
      "type": "object",
      "required": ["tier", "effects", "effect_breadth", "effect_fanout", "linear_values", "linear_consumes", "linear_borrows", "contract"],
      "properties": {
        "tier": { "const": "experimental" },
        "effects": { "type": "array", "items": { "enum": ["io", "panic", "read", "unsafe", "write"] }, "description": "Declared effects, canonical order." },
        "effect_breadth": { "type": "integer", "minimum": 0, "maximum": 5, "description": "popcount of the effect bitset." },
        "effect_fanout": { "type": "integer", "minimum": 0, "description": "Distinct in-module callees that are themselves effectful." },
        "linear_values": { "type": "integer", "minimum": 0, "description": "Count of linear-struct literals constructed in the function." },
        "linear_consumes": { "type": "integer", "minimum": 0, "description": "IOP_LINEAR_CONSUME count (linear resource moves)." },
        "linear_borrows": { "type": "integer", "minimum": 0, "description": "IOP_LINEAR_BORROW count." },
        "contract": { "$ref": "#/$defs/Contract" }
      },
      "additionalProperties": false
    },
    "Function": {
      "type": "object",
      "required": ["name", "line", "complexity_score", "score_factors", "size", "structural", "vow", "verification"],
      "properties": {
        "name": { "type": "string" },
        "line": { "type": "integer", "minimum": 1, "description": "1-based source line of the function." },
        "complexity_score": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Readability / refactor-priority gate. NOT a defect predictor." },
        "score_factors": { "$ref": "#/$defs/ScoreFactors" },
        "size": { "$ref": "#/$defs/Size" },
        "structural": { "$ref": "#/$defs/Structural" },
        "vow": { "$ref": "#/$defs/Vow" },
        "verification": { "$ref": "#/$defs/Verification" }
      },
      "additionalProperties": false
    },
    "Verification": {
      "description": "Verification-difficulty metrics (experimental tier, Vow-unique).",
      "type": "object",
      "required": ["tier", "loops_total", "loops_without_invariant", "max_loop_nesting", "contract_predicate_cost"],
      "properties": {
        "tier": { "const": "experimental" },
        "loops_total": { "type": "integer", "minimum": 0 },
        "loops_without_invariant": { "type": "integer", "minimum": 0, "description": "Loops the BMC must unwind without an invariant." },
        "max_loop_nesting": { "type": "integer", "minimum": 0 },
        "contract_predicate_cost": { "type": "integer", "minimum": 0, "description": "predicate_nodes + free_vars (value identifiers, excluding callees/method names/result) + quantifier flag, summed across clauses." }
      },
      "additionalProperties": false
    },
    "Contract": {
      "description": "Contract surface (experimental tier).",
      "type": "object",
      "required": ["requires", "ensures", "invariants", "predicate_nodes", "predicate_depth", "free_vars", "has_vec_quantification"],
      "properties": {
        "requires": { "type": "integer", "minimum": 0 },
        "ensures": { "type": "integer", "minimum": 0 },
        "invariants": { "type": "integer", "minimum": 0, "description": "Function-level invariant clauses (loop invariants are counted under verification)." },
        "predicate_nodes": { "type": "integer", "minimum": 0, "description": "Total AST nodes across all clause predicates." },
        "predicate_depth": { "type": "integer", "minimum": 0 },
        "free_vars": { "type": "integer", "minimum": 0, "description": "Distinct value identifiers across clauses; excludes the result binding, function callee identifiers, and method names." },
        "has_vec_quantification": { "type": "boolean", "description": "A predicate indexes a Vec (no quantifier syntax exists; this is the proxy)." }
      },
      "additionalProperties": false
    },
    "ScoreFactors": {
      "description": "Sub-scores the gate is built from. cognitive_sub/size_sub/base are in [0,1] (fixed 3 decimals).",
      "type": "object",
      "required": ["cognitive_sub", "size_sub", "vow_bump", "base", "over_threshold"],
      "properties": {
        "cognitive_sub": { "type": "number", "description": "anchor_map(cognitive, --cog-anchor)." },
        "size_sub": { "type": "number", "description": "anchor_map(nloc, --nloc-anchor)." },
        "vow_bump": { "type": "number", "description": "Experimental Vow-surface bump (scale 1000). Sum of effect-breadth, linear-consume, and contract-predicate penalties; capped at 150." },
        "base": { "type": "number", "description": "soft-OR of cognitive_sub and size_sub." },
        "over_threshold": { "type": "boolean" }
      },
      "additionalProperties": false
    },
    "Size": {
      "description": "Stable baseline metrics. Co-reported with every structural metric.",
      "type": "object",
      "required": ["nloc", "tokens", "stmts", "params"],
      "properties": {
        "nloc": { "type": "integer", "minimum": 0, "description": "Source lines spanned by the function." },
        "tokens": { "type": "integer", "minimum": 0, "description": "Halstead length N (total operators + operands)." },
        "stmts": { "type": "integer", "minimum": 0, "description": "Statement count (recursive; trailing expression counts as one)." },
        "params": { "type": "integer", "minimum": 0 }
      },
      "additionalProperties": false
    },
    "Structural": {
      "type": "object",
      "required": ["cyclomatic", "cyclomatic_ir", "cognitive", "max_nesting", "halstead"],
      "properties": {
        "cyclomatic": { "type": "integer", "minimum": 1, "description": "AST decision-count form: base 1 + if/while/for/loop + (match arms - 1) + each &&/|| + each ?." },
        "cyclomatic_ir": { "type": "integer", "minimum": -1, "description": "IR branch-count cross-check: (number of IOP_BRANCH) + 1, or -1 if the function has no IR (e.g. a body-less declaration). Agrees with `cyclomatic` modulo &&/||/? lowering. Experimental tier." },
        "cognitive": { "type": "integer", "minimum": 0, "description": "Vow-adapted Cognitive Complexity (nesting-aware). Headline structural metric." },
        "max_nesting": { "type": "integer", "minimum": 0, "description": "Deepest structural nesting depth." },
        "halstead": { "$ref": "#/$defs/Halstead" }
      },
      "additionalProperties": false
    },
    "Halstead": {
      "type": "object",
      "required": ["n1", "n2", "N1", "N2", "vocabulary", "length", "volume", "difficulty", "effort"],
      "properties": {
        "n1": { "type": "integer", "minimum": 0, "description": "Distinct operators." },
        "n2": { "type": "integer", "minimum": 0, "description": "Distinct operands." },
        "N1": { "type": "integer", "minimum": 0, "description": "Total operators." },
        "N2": { "type": "integer", "minimum": 0, "description": "Total operands." },
        "vocabulary": { "type": "integer", "minimum": 0, "description": "n1 + n2." },
        "length": { "type": "integer", "minimum": 0, "description": "N1 + N2." },
        "volume": { "type": "number", "description": "length * log2(vocabulary). Fixed 3 decimals, saturated." },
        "difficulty": { "type": "number", "description": "(n1/2) * (N2/n2). Fixed 3 decimals." },
        "effort": { "type": "number", "description": "difficulty * volume. Fixed 3 decimals, saturated." }
      },
      "additionalProperties": false
    },
    "Summary": {
      "type": "object",
      "required": ["functions", "nloc_total", "threshold", "functions_over_threshold", "thresholds_exceeded"],
      "properties": {
        "functions": { "type": "integer", "minimum": 0 },
        "nloc_total": { "type": "integer", "minimum": 0 },
        "threshold": { "type": "integer", "description": "--max-score if passed, else the recommended 80." },
        "functions_over_threshold": { "type": "integer", "minimum": 0 },
        "thresholds_exceeded": { "type": "array", "items": { "type": "string" }, "description": "Names of functions whose score exceeds the threshold." }
      },
      "additionalProperties": false
    }
  }
}
"##,
        ),
        (
            r#"schemas/contracts-result.schema.json"#,
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/contracts-result.schema.json",
  "title": "ContractsResult",
  "description": "JSON output from `vow contracts` on stdout",
  "type": "object",
  "required": ["contracts", "summary"],
  "properties": {
    "contracts": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["vow_id", "function", "kind", "description", "blame", "source", "status", "quality"],
        "properties": {
          "vow_id": {
            "type": "integer",
            "description": "Unique contract identifier within the program"
          },
          "function": {
            "type": "string",
            "description": "Function containing this contract"
          },
          "kind": {
            "type": "string",
            "enum": ["requires", "ensures", "invariant"],
            "description": "Contract kind"
          },
          "description": {
            "type": "string",
            "description": "Full contract text"
          },
          "blame": {
            "type": "string",
            "enum": ["Caller", "Callee"],
            "description": "Blame assignment: Caller for requires, Callee for ensures/invariant"
          },
          "source": {
            "type": "object",
            "required": ["file", "offset"],
            "properties": {
              "file": {
                "type": "string",
                "description": "Source file path"
              },
              "offset": {
                "type": "integer",
                "description": "Byte offset in source file"
              }
            },
            "additionalProperties": false
          },
          "status": {
            "type": "string",
            "enum": ["proven", "proven-ir", "failed", "unknown", "timeout", "error", "not_verified", "skipped", "vacuous"],
            "description": "Verification status"
          },
          "quality": {
            "type": "string",
            "enum": ["weak", "tautological", "substantive"],
            "description": "Static, no-ESBMC classification of the clause shape: weak (an ensures that only bounds result by a constant), tautological (constant clause that says nothing), or substantive (equality/relational/inverse/call). See contracts-methodology.md"
          },
          "trivially_satisfiable": {
            "type": "boolean",
            "description": "`--verify` only: true when a trivial `return <default>` body still satisfies this `ensures` (verification-confirmed weakness). Always false for `requires`/`invariant` and without `--verify`. Informational; never affects the exit code. See contracts-methodology.md"
          }
        },
        "additionalProperties": false
      }
    },
    "summary": {
      "type": "object",
      "required": ["total", "proven", "failed", "unknown", "timeout", "error", "not_verified", "skipped", "quality"],
      "properties": {
        "total": { "type": "integer" },
        "proven": { "type": "integer" },
        "failed": { "type": "integer" },
        "unknown": { "type": "integer" },
        "timeout": { "type": "integer" },
        "error": { "type": "integer" },
        "not_verified": { "type": "integer" },
        "skipped": { "type": "integer" },
        "vacuous": { "type": "integer" },
        "trivially_satisfiable": { "type": "integer" },
        "quality": {
          "type": "object",
          "description": "Static contract-quality tallies independent of verification status",
          "required": ["weak", "tautological", "substantive"],
          "properties": {
            "weak": { "type": "integer" },
            "tautological": { "type": "integer" },
            "substantive": { "type": "integer" }
          },
          "additionalProperties": false
        }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
"#,
        ),
        (
            r#"schemas/counterexample.schema.json"#,
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/counterexample.schema.json",
  "title": "Counterexample",
  "description": "A structured counterexample from ESBMC verification failure",
  "type": "object",
  "required": ["function", "values", "violation", "vow_id", "source", "blame"],
  "properties": {
    "function": {
      "type": "string",
      "description": "Name of the function whose verification query failed"
    },
    "values": {
      "type": "object",
      "additionalProperties": { "type": "string" },
      "description": "Map of source names or ESBMC variable names to counterexample values"
    },
    "violation": {
      "type": "string",
      "description": "Description of the violated contract clause"
    },
    "vow_id": {
      "type": "integer",
      "minimum": 0,
      "description": "Function-local ID of the violated vow clause"
    },
    "source": {
      "oneOf": [
        {
          "type": "object",
          "required": ["file", "offset", "length"],
          "properties": {
            "file": { "type": "string", "description": "Source file path" },
            "offset": { "type": "integer", "minimum": 0, "description": "Byte offset of the vow clause" },
            "length": { "type": "integer", "minimum": 0, "description": "Byte length of the vow clause" }
          },
          "additionalProperties": false
        },
        { "type": "string" },
        { "type": "null" }
      ],
      "description": "Source location of the violated vow clause; Rust emits a span object, self-hosted emits the source path string"
    },
    "blame": {
      "type": "string",
      "enum": ["caller", "callee", "none"],
      "description": "Who is responsible for the violation"
    },
    "call_sites": {
      "type": "array",
      "description": "Caller locations relevant to caller-blame failures",
      "items": {
        "type": "object",
        "required": ["caller_function", "file", "offset", "length"],
        "properties": {
          "caller_function": { "type": "string" },
          "file": { "type": "string" },
          "offset": { "type": "integer", "minimum": 0 },
          "length": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
      }
    },
    "violating_args": {
      "type": "array",
      "description": "Callee parameters and caller argument spans for caller-blame precondition failures",
      "items": {
        "type": "object",
        "required": ["param", "value", "arg_offset", "arg_length"],
        "properties": {
          "param": { "type": "string" },
          "value": {
            "type": "string",
            "description": "Counterexample value for the caller argument. The empty string means the value could not be statically recovered; arg_offset and arg_length still identify the caller argument."
          },
          "arg_offset": { "type": "integer", "minimum": 0 },
          "arg_length": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
      }
    },
    "execution_path": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["block_id", "offset", "length"],
        "properties": {
          "block_id": { "type": "integer", "minimum": 0 },
          "offset": { "type": "integer", "minimum": 0 },
          "length": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
      }
    },
    "branch_decisions": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["condition_offset", "condition_length", "taken"],
        "properties": {
          "condition_offset": { "type": "integer", "minimum": 0 },
          "condition_length": { "type": "integer", "minimum": 0 },
          "taken": { "type": "string", "enum": ["then", "else"] }
        },
        "additionalProperties": false
      }
    },
    "replay": {
      "type": "string",
      "enum": ["confirmed", "diverged", "skipped"],
      "description": "Differential-test outcome, present only with --replay-cex"
    },
    "replay_reason": {
      "type": "string",
      "description": "Human-readable explanation for a diverged or skipped replay"
    }
  },
  "additionalProperties": false
}
"#,
        ),
        (
            r#"schemas/diagnostic.schema.json"#,
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/diagnostic.schema.json",
  "title": "Diagnostic",
  "description": "A single compiler diagnostic (error, warning, or note)",
  "type": "object",
  "required": ["error_code", "message", "severity", "span"],
  "properties": {
    "error_code": {
      "type": "string",
      "enum": [
        "UnterminatedString",
        "InvalidCharacter",
        "UnexpectedToken",
        "MissingDelimiter",
        "TypeMismatch",
        "StaticLiteralRequired",
        "EffectViolation",
        "LinearTypeViolation",
        "NonExhaustiveMatch",
        "UnsupportedPattern",
        "ImmutableAssignment",
        "UnusedMut",
        "VowRequiresViolated",
        "VowEnsuresViolated",
        "VowInvariantViolated",
        "UnknownMethod",
        "UnsupportedFeature",
        "LoweringWarning",
        "MissingContract",
        "ContractTypeMismatch",
        "EsbmcNotFound",
        "IoError",
        "RegionConflict",
        "RegionLinear",
        "RegionRootEscape"
      ],
      "description": "Machine-readable error code"
    },
    "message": {
      "type": "string",
      "description": "Human-readable error message"
    },
    "severity": {
      "type": "string",
      "enum": ["error", "warning", "note"],
      "description": "Diagnostic severity"
    },
    "span": {
      "type": "object",
      "required": ["file", "offset", "length"],
      "properties": {
        "file": { "type": "string", "description": "Source file path" },
        "offset": { "type": "integer", "minimum": 0, "description": "Byte offset from start of file" },
        "length": { "type": "integer", "minimum": 0, "description": "Byte length of the span" }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}
"#,
        ),
        (
            r#"schemas/mutants-result.schema.json"#,
            r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.org/schemas/mutants-result.schema.json",
  "title": "MutantsOutput",
  "description": "Output of `vow-mutants run` populates a directory (default `mutants.out/`). Each file has its own schema; the union is documented here.",
  "$defs": {
    "Mutant": {
      "description": "Per-mutant catalog record (in mutants.json).",
      "type": "object",
      "required": ["name", "file", "line", "col", "off", "len", "kind", "from", "to", "label", "clause_index"],
      "properties": {
        "name":         { "type": "string", "description": "Stable cargo-mutants-style name: `file:line:col: <label>`." },
        "file":         { "type": "string", "description": "Repo-relative source path. Independent of --workdir." },
        "line":         { "type": "integer", "minimum": 1, "description": "1-based source line of `off`." },
        "col":          { "type": "integer", "minimum": 1, "description": "1-based source column of `off`." },
        "off":          { "type": "integer", "minimum": 0, "description": "Byte offset of the mutation site." },
        "len":          { "type": "integer", "minimum": 0, "description": "Byte length of the mutation span." },
        "kind":         { "enum": ["op-flip", "const-flip", "body-replace", "contract-weaken"] },
        "from":         { "type": "string", "description": "Original source text at the site (empty for body-replace)." },
        "to":           { "type": "string", "description": "Replacement text spliced in for this mutation." },
        "label":        { "type": "string", "description": "Human-readable summary of the mutation." },
        "clause_index": { "type": "integer", "minimum": 0, "description": "Disambiguates sibling contract clauses on the same function. 0 for non-contract sites." }
      },
      "additionalProperties": false
    },
    "Outcome": {
      "description": "Per-mutant verdict (in outcomes.json).",
      "type": "object",
      "required": ["id", "name", "status", "tier", "oracle_ms"],
      "properties": {
        "id":         { "type": "integer", "minimum": 0, "description": "Position of this mutant within the shard's `mutants.json` array." },
        "name":       { "type": "string", "description": "Same name as the corresponding Mutant record." },
        "status":     { "enum": ["caught", "missed", "timeout", "unviable", "unrun"] },
        "tier":       { "enum": [1, 2], "description": "Oracle tier that produced the verdict." },
        "oracle_ms":  { "type": "integer", "minimum": 0, "description": "Per-mutant oracle wall-clock in milliseconds. Non-deterministic — excluded from the determinism guarantee." }
      },
      "additionalProperties": false
    },
    "Summary": {
      "description": "Aggregate counts across all of this shard's mutants. Same shape as the stdout summary line.",
      "type": "object",
      "required": ["total", "caught", "missed", "timeout", "unviable", "unrun", "shard"],
      "properties": {
        "total":     { "type": "integer", "minimum": 0 },
        "caught":    { "type": "integer", "minimum": 0 },
        "missed":    { "type": "integer", "minimum": 0 },
        "timeout":   { "type": "integer", "minimum": 0 },
        "unviable":  { "type": "integer", "minimum": 0 },
        "unrun":     { "type": "integer", "minimum": 0, "description": "Tier-1 survivors not run because the Tier-2 budget was exhausted." },
        "shard":     { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" }
      },
      "additionalProperties": false
    },
    "MutantsJson": {
      "description": "Format of `mutants.out/mutants.json`. Written before testing begins.",
      "type": "object",
      "required": ["version", "tool", "shard", "mutants"],
      "properties": {
        "version":  { "const": 1 },
        "tool":     { "const": "vow-mutants" },
        "shard":    { "type": "string", "pattern": "^[0-9]+/[1-9][0-9]*$" },
        "mutants":  { "type": "array", "items": { "$ref": "#/$defs/Mutant" } }
      },
      "additionalProperties": false
    },
    "OutcomesJson": {
      "description": "Format of `mutants.out/outcomes.json`. Written after all mutants in this shard have been classified.",
      "type": "object",
      "required": ["version", "summary", "outcomes"],
      "properties": {
        "version":  { "const": 1 },
        "summary":  { "$ref": "#/$defs/Summary" },
        "outcomes": { "type": "array", "items": { "$ref": "#/$defs/Outcome" } }
      },
      "additionalProperties": false
    }
  }
}
"##,
        ),
        (
            r#"schemas/test-result.schema.json"#,
            r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/test-result.schema.json",
  "title": "TestResult",
  "description": "JSON output from `vow test` on stdout",
  "type": "object",
  "required": ["status", "total", "passed", "failed", "skipped", "tests", "contract_density"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["TestsPassed", "TestsFailed"],
      "description": "Overall test outcome"
    },
    "total": {
      "type": "integer",
      "description": "Total number of tests discovered"
    },
    "passed": {
      "type": "integer",
      "description": "Number of tests that passed"
    },
    "failed": {
      "type": "integer",
      "description": "Number of tests that failed"
    },
    "skipped": {
      "type": "integer",
      "description": "Number of tests that were skipped"
    },
    "tests": {
      "type": "array",
      "items": { "$ref": "#/$defs/TestEntry" },
      "description": "Per-test results"
    },
    "contract_density": {
      "$ref": "#/$defs/ContractDensity",
      "description": "Contract density across tested modules"
    }
  },
  "$defs": {
    "TestEntry": {
      "type": "object",
      "required": ["file", "name", "status", "stdout", "stderr", "duration_ms", "diagnostics", "counterexamples"],
      "properties": {
        "file": { "type": "string", "description": "Path to the test .vow file" },
        "name": { "type": "string", "description": "Test name (file stem)" },
        "status": {
          "type": "string",
          "enum": ["passed", "failed", "timeout", "skipped", "compile_error", "verify_failed", "contract_skipped"],
          "description": "Per-test outcome (`contract_skipped`: ESBMC never invoked because a vowed function is non-modelable; fail-closed, counts toward `failed`)"
        },
        "exit_code": {
          "type": ["integer", "null"],
          "description": "Process exit code, null on compile/verify failure or timeout"
        },
        "stdout": { "type": "string", "description": "Captured stdout from test binary" },
        "stderr": { "type": "string", "description": "Captured stderr from test binary" },
        "duration_ms": { "type": "integer", "description": "Wall-clock duration in milliseconds" },
        "diagnostics": {
          "type": "array",
          "items": { "$ref": "diagnostic.schema.json" },
          "description": "Compiler diagnostics (on compile_error)"
        },
        "counterexamples": {
          "type": "array",
          "items": { "$ref": "counterexample.schema.json" },
          "description": "ESBMC counterexamples (on verify_failed)"
        }
      }
    },
    "ContractDensity": {
      "type": "object",
      "required": ["functions_total", "functions_with_vows", "density_pct"],
      "properties": {
        "functions_total": { "type": "integer", "description": "Total non-main functions across tested modules" },
        "functions_with_vows": { "type": "integer", "description": "Functions with at least one vow block" },
        "density_pct": { "type": "number", "description": "Percentage of functions with vow contracts" }
      }
    }
  }
}
"##,
        ),
        (
            r#"schemas/vow-violation.schema.json"#,
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://vow-lang.dev/schemas/vow-violation.schema.json",
  "title": "VowViolation",
  "description": "Runtime vow violation emitted to stderr (debug mode only). This is emitted by the vow-runtime C code, not by serde.",
  "type": "object",
  "required": ["error", "vow_id", "blame", "description", "file", "offset"],
  "properties": {
    "error": {
      "type": "string",
      "const": "VowViolation",
      "description": "Always the string VowViolation"
    },
    "vow_id": {
      "type": "integer",
      "minimum": 0,
      "description": "Numeric ID of the violated vow"
    },
    "blame": {
      "type": "string",
      "enum": ["Caller", "Callee"],
      "description": "Who is blamed: Caller for requires violations, Callee for ensures/invariant"
    },
    "description": {
      "type": "string",
      "description": "The contract predicate text"
    },
    "file": {
      "type": "string",
      "description": "Source file path"
    },
    "offset": {
      "type": "integer",
      "minimum": 0,
      "description": "Byte offset of the vow in the source file"
    },
    "values": {
      "type": "object",
      "additionalProperties": {
        "type": ["integer", "number", "boolean"]
      },
      "description": "Runtime values of free variables in the predicate (optional, present when bindings exist)"
    }
  },
  "additionalProperties": false
}
"#,
        ),
    ]
}
// GENERATE:SKILL_FULL:END

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Args, Command, SkillAction};
    use clap::Parser;
    use tempfile::TempDir;

    #[test]
    fn interface_exposes_help_and_installable_docs() {
        // The module's seam: machine help, human help, the concise entrypoint,
        // and the full bundle. main.rs and tests cross exactly this interface.
        let parsed: serde_json::Value = serde_json::from_str(&json()).unwrap();
        assert_eq!(parsed["kind"], "tool_help");
        assert_eq!(parsed["tool"], "vow");
        assert!(human().contains("USAGE"));
        assert!(entrypoint_markdown().starts_with("---\nname: vow\n"));
        assert!(bundle_markdown().contains("# Vow Language Reference"));
    }

    #[test]
    fn skill_install_writes_concise_entrypoint_and_support_files() {
        let dir = TempDir::new().unwrap();
        install_skill_tree_to(dir.path()).unwrap();

        let skill_dir = dir.path().join(".claude/skills/vow");
        let contents = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(contents.starts_with("---\nname: vow\n"));
        assert!(contents.contains("when_to_use: >-"));
        assert!(!contents.contains("\npaths:"));
        assert!(!contents.contains("\nallowed-tools:"));
        assert!(contents.lines().count() < 500);
        assert!(
            !contents.contains("```json"),
            "SKILL.md entrypoint should link to schemas, not inline them"
        );

        let grammar = std::fs::read_to_string(skill_dir.join("reference/grammar.md")).unwrap();
        assert!(grammar.contains("# Vow Grammar Reference"));
        let schema =
            std::fs::read_to_string(skill_dir.join("schemas/build-result.schema.json")).unwrap();
        assert!(schema.contains("\"title\": \"BuildResult\""));
    }

    #[test]
    fn skill_bundle_markdown_contains_full_skill_document() {
        let out = skill_bundle_markdown();
        assert!(out.starts_with("---\nname: vow\n"));
        assert!(!out.contains("\npaths:"));
        assert!(!out.contains("\nallowed-tools:"));
        assert!(out.contains("# Vow Language Reference"));
        assert!(out.contains("### `vow skill`"));
        assert!(out.contains("schemas/build-result.schema.json"));
    }

    /// `npx skills add vow-lang/vow` clones the repo and scans `skills/*/SKILL.md`.
    /// The checked-in `skills/vow/` mirror must therefore match what the compiler
    /// installs, or the two install paths drift.
    #[test]
    fn checked_in_skills_vow_matches_install_output() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CARGO_MANIFEST_DIR has a parent");
        let skill_md = repo_root.join("skills/vow/SKILL.md");
        let checked_in =
            std::fs::read_to_string(&skill_md).expect("skills/vow/SKILL.md must exist");
        assert!(
            checked_in.starts_with("---\nname: vow\n"),
            "checked-in skills/vow/SKILL.md must declare `name: vow` for npx skills"
        );
        assert_eq!(
            checked_in,
            skill_entrypoint_markdown(),
            "skills/vow/SKILL.md drifted from compiler-embedded skill; \
             run `uv run python scripts/generate_help.py`"
        );
        for (rel, expected) in skill_support_files() {
            let path = repo_root.join("skills/vow").join(rel);
            let actual = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("missing skills/vow/{rel}: {e}"));
            assert_eq!(
                actual, *expected,
                "skills/vow/{rel} drifted from compiler-embedded copy"
            );
        }
    }

    fn generated_skill_support_count(source: &str) -> usize {
        let body = generated_vow_function_lines(source, "fn skill_support_count() -> i64 {");
        let mut literal_lines = body
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty());
        let literal = literal_lines
            .next()
            .expect("skill_support_count() must return an integer literal");
        assert!(
            literal_lines.next().is_none(),
            "skill_support_count() should contain only the generated count literal"
        );
        literal
            .parse::<usize>()
            .expect("skill_support_count() must return a non-negative integer literal")
    }

    fn generated_skill_support_branch_count(source: &str, signature: &str) -> usize {
        generated_vow_function_lines(source, signature)
            .into_iter()
            .filter(|line| line.trim_start().starts_with("if index =="))
            .count()
    }

    fn generated_vow_function_lines<'a>(source: &'a str, signature: &str) -> Vec<&'a str> {
        let mut lines = source.lines().skip_while(|line| line.trim() != signature);
        lines
            .next()
            .unwrap_or_else(|| panic!("generated Vow function `{signature}` must exist"));

        let mut body = Vec::new();
        for line in lines {
            if line.trim() == "}" {
                return body;
            }
            body.push(line);
        }
        panic!("generated Vow function `{signature}` must close with a standalone `}}`");
    }

    #[test]
    fn generated_vow_skill_support_uses_indexed_lookup() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CARGO_MANIFEST_DIR has a parent");
        let compiler_main = repo_root.join("compiler/main.vow");
        let source = std::fs::read_to_string(&compiler_main).expect("compiler/main.vow must exist");

        assert!(source.contains("fn skill_support_count() -> i64"));
        assert!(source.contains("fn skill_support_path(index: i64) -> String vow {"));
        assert!(source.contains("fn skill_support_content_index_guard(index: i64) vow {"));
        assert!(source.contains("fn skill_support_content(index: i64) -> String {"));
        assert!(source.contains("    skill_support_content_index_guard(index);"));
        assert_eq!(
            source
                .matches("requires: index >= 0 && index < skill_support_count()")
                .count(),
            2,
            "indexed support lookup contracts should guard path lookup and content access"
        );
        let support_count = generated_skill_support_count(&source);
        let path_branch_count = generated_skill_support_branch_count(
            &source,
            "fn skill_support_path(index: i64) -> String vow {",
        );
        let content_branch_count = generated_skill_support_branch_count(
            &source,
            "fn skill_support_content(index: i64) -> String {",
        );
        assert_eq!(
            path_branch_count, support_count,
            "skill_support_count() returned {support_count}, \
             but skill_support_path has {path_branch_count} generated index branches"
        );
        assert_eq!(
            content_branch_count, support_count,
            "skill_support_count() returned {support_count}, \
             but skill_support_content has {content_branch_count} generated index branches"
        );
        assert!(
            !source.contains("fn skill_support_paths() -> Vec<String>"),
            "generated Vow should expose indexed path lookup, not a path vector"
        );
        assert!(
            !source.contains("fn skill_support_content_0()"),
            "generated Vow support helpers should not be numbered by bundle position"
        );
        assert!(
            source.contains("fn skill_support_content_reference_grammar_md_92418de9() -> String {"),
            "generated Vow content helpers should use stable path-derived names with an 8-char hash suffix"
        );
        assert!(
            !source.contains("fn skill_support_contents() -> Vec<String>"),
            "generated Vow should stream support contents by index during install"
        );
    }

    #[test]
    fn auto_install_skill_skips_when_no_claude_dir() {
        let dir = TempDir::new().unwrap();
        maybe_auto_install(dir.path());
        let claude_dir = dir.path().join(".claude");
        assert!(
            !claude_dir.exists(),
            "auto-install must not create .claude/ when absent"
        );
    }

    #[test]
    fn auto_install_skill_installs_when_claude_dir_present() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        maybe_auto_install(dir.path());
        let installed = dir.path().join(".claude/skills/vow/SKILL.md");
        assert!(
            installed.exists(),
            "auto-install must populate the skill when .claude/ is present"
        );
        let contents = std::fs::read_to_string(installed).unwrap();
        assert!(!contents.contains("\nallowed-tools:"));
    }

    #[test]
    fn auto_install_skill_leaves_existing_file_untouched() {
        let dir = TempDir::new().unwrap();
        let target_dir = dir.path().join(".claude/skills/vow");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("SKILL.md");
        std::fs::write(&target, "user-managed content").unwrap();
        maybe_auto_install(dir.path());
        let contents = std::fs::read_to_string(&target).unwrap();
        assert_eq!(contents, "user-managed content");
    }

    #[test]
    fn skill_install_local_writes_into_current_git_project() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();
        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();

        let installed =
            run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
                .unwrap();

        assert_eq!(installed, dir.path().join(".claude/skills/vow/SKILL.md"));
        assert!(installed.exists());
        assert!(
            dir.path()
                .join(".claude/skills/vow/reference/cli.md")
                .exists()
        );
    }

    #[test]
    fn skill_install_accepts_explicit_scope_flags() {
        let parsed = Args::try_parse_from(["vow", "skill", "install", "--local"]).unwrap();
        let Some(Command::Skill(skill)) = parsed.command else {
            panic!("expected skill command");
        };
        let Some(SkillAction::Install { local, global }) = skill.action else {
            panic!("expected install action");
        };
        assert!(local);
        assert!(!global);

        let parsed = Args::try_parse_from(["vow", "skill", "install", "--global"]).unwrap();
        let Some(Command::Skill(skill)) = parsed.command else {
            panic!("expected skill command");
        };
        let Some(SkillAction::Install { local, global }) = skill.action else {
            panic!("expected install action");
        };
        assert!(!local);
        assert!(global);
    }

    #[test]
    fn skill_install_global_writes_under_home() {
        let cwd = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();

        let installed = run_skill_install_scoped(
            cwd.path(),
            Some(home.path()),
            false,
            true,
            &mut stdin,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(installed, home.path().join(".claude/skills/vow/SKILL.md"));
        assert!(installed.exists());
        assert!(
            home.path()
                .join(".claude/skills/vow/schemas/diagnostic.schema.json")
                .exists()
        );
    }

    #[test]
    fn skill_install_local_requires_git_and_claude_project() {
        let dir = TempDir::new().unwrap();
        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();

        let err = run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(err.contains(".claude"));

        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        let err = run_skill_install_scoped(dir.path(), None, true, false, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(err.contains("git"));
    }

    #[test]
    fn skill_install_prompts_when_scope_is_omitted() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".git"), "gitdir: ../real-git-dir\n").unwrap();
        let mut stdin = std::io::Cursor::new(b"local\n".to_vec());
        let mut stderr = Vec::new();

        let installed =
            run_skill_install_scoped(dir.path(), None, false, false, &mut stdin, &mut stderr)
                .unwrap();
        assert_eq!(installed, dir.path().join(".claude/skills/vow/SKILL.md"));
        assert!(String::from_utf8(stderr).unwrap().contains("[l/g]"));

        let mut stdin = std::io::Cursor::new(Vec::<u8>::new());
        let mut stderr = Vec::new();
        let err = run_skill_install_scoped(dir.path(), None, false, false, &mut stdin, &mut stderr)
            .unwrap_err();
        assert!(err.contains("--local or --global"));
    }
}
