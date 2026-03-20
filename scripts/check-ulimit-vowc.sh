#!/usr/bin/env bash
# PreToolUse hook: block Bash commands that run ./vowc or self-compiled
# binaries without ulimit -v to prevent memory exhaustion.

CMD=$(echo "$TOOL_INPUT" | jq -r '.command // empty')
[ -z "$CMD" ] && exit 0

# Check if command references vowc or self-compiled binaries
if echo "$CMD" | grep -qP '(\./vowc|/tmp/vow_|/tmp/compiler_)'; then
  # Allow if ulimit is present
  if ! echo "$CMD" | grep -qP 'ulimit'; then
    echo "BLOCK: Running ./vowc or self-compiled binaries without 'ulimit -v 2000000' risks exhausting all system memory." >&2
    echo "Prefix your command with: ulimit -v 2000000;" >&2
    exit 2
  fi
fi

exit 0
