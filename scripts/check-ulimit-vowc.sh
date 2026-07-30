#!/usr/bin/env bash
# PreToolUse hook: block Bash commands that run build/vowc or self-compiled
# binaries without ulimit -v 2000000 to prevent memory exhaustion.

if ! command -v jq >/dev/null 2>&1; then
  echo "BLOCK: jq is required for the ulimit safety hook but is not installed." >&2
  exit 2
fi

input=$(cat)
CMD=$(echo "$input" | jq -r '.tool_input.command // empty')
[ -z "$CMD" ] && exit 0

# Strip heredoc bodies and quoted string contents before matching, so prose
# mentions of the binary (commit messages passed via heredoc, gh comment
# bodies, echo strings) don't false-trigger the guard on commands that never
# invoke it. Known trade-off: this also misses a genuine invocation nested
# inside a heredoc used as script input (e.g. `bash <<'EOF'` containing a
# real build/vowc call) rather than as literal data; that pattern is rare in
# practice compared to heredoc-as-commit-message, which is the common case.
UNQUOTED=""
in_heredoc=0
delim=""
while IFS= read -r line; do
  if [ "$in_heredoc" = "1" ]; then
    if [ "$line" = "$delim" ]; then
      in_heredoc=0
    fi
    continue
  fi
  if [[ "$line" =~ \<\<-?[[:space:]]*[\'\"]?([A-Za-z_][A-Za-z0-9_]*)[\'\"]? ]]; then
    delim="${BASH_REMATCH[1]}"
    in_heredoc=1
  fi
  UNQUOTED+="$line"$'\n'
done <<< "$CMD"
UNQUOTED=$(echo "$UNQUOTED" | sed -E "s/'[^']*'//g; s/\"[^\"]*\"//g")

if echo "$UNQUOTED" | grep -qP '(build/vowc|/tmp/vow_|/tmp/compiler_|/tmp/lexer\b)'; then
  if ! echo "$CMD" | grep -qP 'ulimit\s+-v\s+2000000\b'; then
    echo "BLOCK: Running build/vowc or self-compiled binaries without 'ulimit -v 2000000' risks exhausting all system memory." >&2
    echo "Prefix your command with: ulimit -v 2000000;" >&2
    exit 2
  fi
fi

exit 0
