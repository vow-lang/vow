#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
DIR="$(cd "$(dirname "$0")/.." && pwd)/compiler"

if [[ "$MODE" != "ir" && "$MODE" != "cgen" ]]; then
    echo "Usage: $0 {ir|cgen}" >&2
    exit 1
fi

strip_header() {
    sed '/^module /d; /^use /d; /^$/d' "$1"
    echo
}

echo "module Compiler"
echo

if [[ "$MODE" == "ir" ]]; then
    FILES=(span token lexer ast parser types env checker ir ir_printer lower main)
    for f in "${FILES[@]}"; do
        strip_header "$DIR/$f.vow"
    done
elif [[ "$MODE" == "cgen" ]]; then
    FILES=(span token lexer ast parser types env checker cgen)
    for f in "${FILES[@]}"; do
        strip_header "$DIR/$f.vow"
    done
    # main_cgen: always generates C code
    cat <<'MAIN_CGEN'
fn main() -> i32 [io] {
    let argv: Vec<String> = args();
    let argc: i64 = argv.len();
    if argc == 0 {
        eprintln_str("usage: main <file.vow>");
        return 1;
    }
    let path: String = argv[argc - 1];
    let src: String = fs_read(path);
    let tokens: Vec<Token> = lex(src);
    let n_tokens: i64 = tokens.len();
    let m: Module = parse_module(tokens);
    let n_items: i64 = m.items.len();
    let e: CheckEnv = env_new();
    check_module(e, m);
    let n_errors: i64 = e.error_count;
    print_i64(n_tokens);
    print_str(" tokens, ");
    print_i64(n_items);
    print_str(" items, ");
    print_i64(n_errors);
    print_str(" errors\n");
    if n_errors == 0 {
        let c_code: String = gen_module_str(m);
        let out_path: String = str2(path, String::from(".c"));
        fs_write(out_path, c_code);
    }
    0
}
MAIN_CGEN
fi
