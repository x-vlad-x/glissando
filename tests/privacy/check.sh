#!/usr/bin/env bash
# Privacy checks:
#  1. no networking crates in the dependency graph,
#  2. the keyboard binary imports no name-resolution functions,
#  3. the full end-to-end suite runs with a socket guard (any non-local
#     socket or DNS lookup is refused and recorded -> failure),
#  4. the end-to-end suite passes inside a network namespace without any
#     network interfaces (works fully offline).
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
fail=0

deny='^(reqwest|hyper|ureq|curl|isahc|surf|attohttpc|openssl|native-tls|rustls|tokio-rustls|h2|quinn|trust-dns|hickory|sentry|opentelemetry)$'
bad=$(cargo metadata --format-version 1 --locked 2>/dev/null | python3 -I -c 'import json,sys; print("\n".join(sorted({p["name"] for p in json.load(sys.stdin)["packages"]})))' | grep -E "$deny" || true)
if [ -n "$bad" ]; then echo "FAIL networking crates: $bad"; fail=1; else echo "PASS no networking crates"; fi

cargo build -q --release -p glissando -p glissando-testkit
syms=$(nm -D --undefined-only target/release/glissando | grep -wE 'getaddrinfo|gethostbyname|gethostbyname2|inet_pton|inet_aton|res_query' || true)
if [ -n "$syms" ]; then echo "FAIL name resolution imports: $syms"; fail=1; else echo "PASS no name-resolution imports"; fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gcc -shared -fPIC -O2 -o "$tmp/netguard.so" tests/privacy/netguard.c -ldl
export NETGUARD_LOG="$tmp/net.log"
: > "$NETGUARD_LOG"
if E2E_KB_PRELOAD="$tmp/netguard.so" dbus-run-session -- python3 -I tests/e2e/e2e.py --release > "$tmp/e2e.log" 2>&1; then
    echo "PASS e2e with socket guard"
else
    echo "FAIL e2e with socket guard"; grep -E "^(PASS|FAIL)" "$tmp/e2e.log"; fail=1
fi
grep -E "^SKIP" "$tmp/e2e.log" || true
if [ -s "$NETGUARD_LOG" ]; then echo "FAIL network attempts:"; cat "$NETGUARD_LOG"; fail=1; else echo "PASS no network attempts"; fi

if unshare --user --map-root-user --net sh -c 'ip -o link 2>/dev/null | grep -qv " lo:" && exit 1; exit 0'; then
    if unshare --user --map-root-user --net dbus-run-session -- python3 -I tests/e2e/e2e.py --release > "$tmp/offline.log" 2>&1; then
        echo "PASS e2e offline (empty network namespace)"
    else
        echo "FAIL e2e offline"; grep -E "^(PASS|FAIL)" "$tmp/offline.log"; fail=1
    fi
else
    echo "FAIL could not create an empty network namespace; the offline check did not run"; fail=1
fi
exit $fail
