#!/usr/bin/env sh
# Exercises a running openmod server end to end: health, auth, both APIs,
# the SightEngine-compatible endpoints, the error envelope and the admin
# interface's access control.
# Usage: OPENMOD_KEY=... sh scripts/smoke-test.sh <safe.jpg> <unsafe.jpg> [base_url] [admin_url]
set -eu

SAFE="$1"
UNSAFE="$2"
BASE="${3:-http://127.0.0.1:8088}"
ADMIN="${4:-http://127.0.0.1:8089}"
KEY="${OPENMOD_KEY:?set OPENMOD_KEY}"
fail=0

check() { # name, expected HTTP status, then curl arguments
  name="$1"
  want="$2"
  shift 2
  body="$(mktemp)"
  got="$(curl -s -o "$body" -w '%{http_code}' "$@")"
  if [ "$got" = "$want" ]; then
    echo "ok   $got $name: $(head -c 160 "$body" | tr '\n' ' ')"
  else
    echo "FAIL $got (want $want) $name: $(head -c 300 "$body")"
    fail=1
  fi
  rm -f "$body"
}

AUTH="Authorization: Bearer $KEY"
check "health" 200 "$BASE/health"
check "no key" 401 --data-binary @"$SAFE" "$BASE/v1/image"
check "wrong key" 401 -H "Authorization: Bearer wrong-key-wrong-key-wrong" --data-binary @"$SAFE" "$BASE/v1/image"
check "safe image" 200 -H "$AUTH" --data-binary @"$SAFE" "$BASE/v1/image?workflow=strict"
check "unsafe image" 200 -H "$AUTH" --data-binary @"$UNSAFE" "$BASE/v1/image?workflow=strict"
check "not an image" 415 -H "$AUTH" --data-binary "<html><script>alert(1)</script></html>" "$BASE/v1/image"
check "empty body" 400 -H "$AUTH" -X POST "$BASE/v1/image"
check "unknown workflow" 400 -H "$AUTH" --data-binary @"$SAFE" "$BASE/v1/image?workflow=nope"
check "text" 200 -H "$AUTH" -H "Content-Type: application/json" \
  -d '{"text": "Call me on +32 470 12 34 56, I sell pills", "workflow": "default"}' "$BASE/v1/text"
check "compat image" 200 -F "media=@$UNSAFE" -F "workflow=strict" -F "api_user=x" -F "api_secret=$KEY" \
  "$BASE/1.0/check-workflow.json"
check "compat text" 200 -F "text=see you at the party tonight" -F "workflow=default" -F "api_secret=$KEY" \
  "$BASE/1.0/text/check-workflow.json"
check "compat wrong secret" 401 -F "media=@$SAFE" -F "api_secret=nope" "$BASE/1.0/check-workflow.json"
check "admin page" 200 "$ADMIN/"
check "admin state needs a session" 401 "$ADMIN/api/state"
check "admin write without the interface header" 403 -H "Content-Type: application/json" -d '{"token":"x"}' "$ADMIN/api/login"
check "admin write from another site" 403 -H "x-openmod-admin: 1" -H "Origin: https://evil.example"   -H "Content-Type: application/json" -d '{"token":"x"}' "$ADMIN/api/login"
check "admin wrong token" 401 -H "x-openmod-admin: 1" -H "Content-Type: application/json" -d '{"token":"x"}' "$ADMIN/api/login"
exit "$fail"
