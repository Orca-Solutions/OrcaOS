#!/usr/bin/env bash
# SPDX-FileCopyrightText: The OrcaOS Authors
# SPDX-License-Identifier: GPL-3.0-or-later

# Starts and stops the conventional composition's processes on one host:
# PostgreSQL, the Temporal dev server, the coordinator worker, the mail
# adapter and the approval service. The mock mail service is not part of the
# platform and is started separately.
#
#   stack.sh init    create the PostgreSQL cluster, database and schema (once)
#   stack.sh start   start whatever is not running and wait until it answers
#   stack.sh kill    SIGKILL every platform process and participant container
#                    (the crash tests)
#
# Run from experiment/. Scorecard: excluded (operations; its steps are
# reported under operational burden).

set -euo pipefail

BASE=${ORCA_BASE:-/opt/baseline}
PGBIN=${PGBIN:-/usr/lib/postgresql/16/bin}
RUN=$BASE/run
BIN=$(pwd)/target/debug/conventional
export ORCA_PG=${ORCA_PG:-"host=/var/run/postgresql port=5433 user=postgres dbname=orca"}
mkdir -p "$RUN" "$BASE/data"

alive() {
    [ -f "$RUN/$1.pid" ] && kill -0 "$(cat "$RUN/$1.pid")" 2>/dev/null
}

launch() {
    local name=$1
    shift
    alive "$name" && return 0
    nohup "$@" >>"$RUN/$name.log" 2>&1 &
    echo $! >"$RUN/$name.pid"
}

wait_for() {
    local what=$1
    shift
    for _ in $(seq 1 240); do
        if "$@" >/dev/null 2>&1; then return 0; fi
        sleep 0.25
    done
    echo "$what did not come up" >&2
    return 1
}

pg_up() {
    "$PGBIN/pg_isready" -q -h /var/run/postgresql -p 5433
}

temporal_up() {
    "$BASE/bin/temporal" operator namespace describe --namespace default --address 127.0.0.1:7233
}

case "${1:-}" in
init)
    mkdir -p "$BASE/pg" /var/run/postgresql
    chown postgres "$BASE/pg" /var/run/postgresql
    [ -d "$BASE/pg/data" ] || runuser -u postgres -- "$PGBIN/initdb" -D "$BASE/pg/data" -A trust -U postgres
    "$0" start-postgres
    runuser -u postgres -- "$PGBIN/createdb" -h /var/run/postgresql -p 5433 orca 2>/dev/null || true
    "$BIN" ctl - init-db
    ;;
start-postgres)
    alive postgres || launch postgres runuser -u postgres -- "$PGBIN/postgres" -D "$BASE/pg/data" -p 5433 \
        -k /var/run/postgresql -c listen_addresses=''
    wait_for postgres pg_up
    ;;
start)
    "$0" start-postgres
    launch temporal "$BASE/bin/temporal" server start-dev --db-filename "$BASE/data/temporal.db" \
        --headless --port 7233 --log-level warn
    wait_for temporal temporal_up
    launch approval "$BIN" approval
    launch worker "$BIN" worker
    MOCK_MAIL_CREDENTIAL_FILE="$RUN/mail-credential" launch adapter "$BIN" adapter
    wait_for approval curl -sf "http://127.0.0.1:18090/display?run=-&proposal=-"
    ;;
kill)
    for name in adapter worker approval temporal postgres; do
        if [ -f "$RUN/$name.pid" ]; then
            kill -9 "$(cat "$RUN/$name.pid")" 2>/dev/null || true
            rm -f "$RUN/$name.pid"
        fi
    done
    # runuser forks the postmaster; kill it and its backends too.
    pkill -9 -u postgres -f "$PGBIN/postgres" 2>/dev/null || true
    pkill -9 -f "temporal server start-dev" 2>/dev/null || true
    podman ps --quiet --filter label=orca.run | xargs -r podman kill >/dev/null 2>&1 || true
    # PostgreSQL refuses to start while the pid in either of its lock files
    # still exists, and a killed process stays a zombie until this host's
    # init reaps it. Wait for the postmaster to die, then remove both.
    lock=$BASE/pg/data/postmaster.pid
    if [ -f "$lock" ]; then
        pid=$(head -1 "$lock")
        for _ in $(seq 1 50); do
            state=$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || true)
            [ -z "$state" ] || [ "$state" = Z ] && break
            sleep 0.1
        done
        if [ -n "$state" ] && [ "$state" != Z ]; then
            echo "the postmaster ($pid) is still running; leaving its lock files" >&2
            exit 1
        fi
    fi
    rm -f "$lock" /var/run/postgresql/.s.PGSQL.5433.lock
    ;;
*)
    echo "usage: stack.sh init|start|kill" >&2
    exit 2
    ;;
esac
