#!/bin/zsh
cd "${0:a:h}"

EXEC=target/Juku.app/Contents/MacOS/Juku

/usr/bin/log stream --process Juku --level debug --predicate 'senderImagePath ENDSWITH "Juku"' &
LOG_PID=$!

cleanup() {
    kill $LOG_PID 2>/dev/null
    wait $LOG_PID 2>/dev/null
    exit 0
}

trap cleanup INT TERM

sleep 0.3

"$EXEC"

cleanup

