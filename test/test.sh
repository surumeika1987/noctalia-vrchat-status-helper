#!/bin/sh

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
IPC_LOG_FILE="$SCRIPT_DIR/ipc.log"
SOCKET_FILE="$SCRIPT_DIR/vrchat-status-helper.sock"
HELPER="$SCRIPT_DIR/../target/release/vrchat-status-helper"

TIMEOUT=5
NOT_PASS_TEST=0
PID=""

cleanup() {
    if [ -n "$PID" ]; then
        kill "$PID" 2>/dev/null
        wait "$PID" 2>/dev/null
    fi

    rm -f "$IPC_LOG_FILE"
    rm -f "$SOCKET_FILE"
}

wait_for_ipc_log() {
    elapsed=0

    while [ ! -f "$IPC_LOG_FILE" ]; do
        if [ "$elapsed" -ge "$TIMEOUT" ]; then
            echo "Timeout: ipc log file was not created: $IPC_LOG_FILE" >&2
            return 1
        fi

        sleep 1
        elapsed=$((elapsed + 1))
    done
}

check_ipc_log() {
    test_name="$1"
    expected="$2"

    if ! wait_for_ipc_log; then
        echo "$test_name: NG"
        NOT_PASS_TEST=1
        return
    fi

    actual="$(cat "$IPC_LOG_FILE")"

    if [ "$actual" = "$expected" ]; then
        echo "$test_name: OK"
    else
        echo "$test_name: NG"
        echo "expected=$expected"
        echo "actual=$actual"
        NOT_PASS_TEST=1
    fi

    rm -f "$IPC_LOG_FILE"
}

run_helper() {
    PATH="$SCRIPT_DIR:$PATH" \
    XDG_RUNTIME_DIR="$SCRIPT_DIR" \
    "$HELPER" "$@"
}

trap cleanup EXIT INT TERM

# 前処理
rm -f "$IPC_LOG_FILE"
rm -f "$SOCKET_FILE"

cargo build \
    --release \
    --manifest-path "$SCRIPT_DIR/../Cargo.toml" \
    2>/dev/null || exit 1

# helperをバックグラウンド起動
run_helper test &
PID=$!

# First test
check_ipc_log \
    "First test" \
    "msg plugin surumeika1987/vrchat-status:status all push-status 4:Test Mode"

# Second test
run_helper msg push-status '2:Test Message'

check_ipc_log \
    "Second test" \
    "msg plugin surumeika1987/vrchat-status:status all push-status 2:Test Message"

# Third test
run_helper msg request-push

check_ipc_log \
    "Third test" \
    "msg plugin surumeika1987/vrchat-status:status all push-status 2:Test Message"

exit "$NOT_PASS_TEST"
