#!/bin/sh
# contrib-preflight.sh — pre-flight a contribution against a project's CI requirements.
#
# Why: fork PRs on many repos (e.g. estkme-group/lpac) sit at CI
# `action_required` until a maintainer approves workflow runs — so the ONLY
# gate that judges your code before a human looks at it is YOUR local one.
# This script (1) discovers what the project's CI demands, (2) runs the
# formatter/lint gates locally with a version-matched clang-format.
#
# Usage:
#   contrib-preflight.sh OWNER/REPO [BASE_REF] [-- <file>...]
#     BASE_REF  defaults to 'main' — used to compute changed files via git.
#     files     default: C/C++ sources changed vs BASE_REF in the cwd git tree.
#
# Environment:
#   CFMT_BIN     clang-format binary (default: ~/tools/cfmt-20/bin/clang-format)
#   CFMT_STRICT  '1' = fail when local formatter major != CI's major
#
# Exit: 0 = gates green / 1 = violations or missing prerequisites.

set -u

REPO="${1:?usage: contrib-preflight.sh OWNER/REPO [BASE_REF] [-- <file>...]}"
BASE="${2:-main}"
shift 2 2>/dev/null || shift $# 2>/dev/null || true
FILES=""
if [ "${1:-}" = "--" ]; then shift; FILES="$*"; fi

fail=0
say() { printf '%s\n' "$*"; }
hr() { say "---------------------------------------------------------------"; }

# --- 1. CI requirements discovery (read-only) -------------------------------
hr; say "CI REQUIREMENTS: $REPO"
WF=$(gh api "repos/$REPO/contents/.github/workflows" --jq '.[].name' 2>/dev/null)
if [ -z "$WF" ]; then
    say "  (no .github/workflows found or unreadable)"
else
    for w in $WF; do
        say "  workflow: $w"
        gh api -H "Accept: application/vnd.github.raw" "repos/$REPO/contents/.github/workflows/$w" 2>/dev/null |
            grep -nE 'clang-format|version:|pint|gofmt|black|ruff|reuse|dco|signoff|stefanzweifel|ncfnav|checkout-mbedtls|pre-commit' |
            sed 's/^/    /'
    done
fi
CI_CFMV=$(gh api -H "Accept: application/vnd.github.raw" "repos/$REPO/contents/.github/workflows/lint.yaml" 2>/dev/null |
    grep -A2 'clang-format-action' | grep 'version:' | grep -oE '[0-9]+' | head -1)
[ -z "$CI_CFMV" ] && CI_CFMV=$(for w in $WF; do
    gh api -H "Accept: application/vnd.github.raw" "repos/$REPO/contents/.github/workflows/$w" 2>/dev/null |
        grep -A2 'clang-format' | grep 'version:' | grep -oE '[0-9]+' | head -1; done | head -1)

for meta in .clang-format .editorconfig CONTRIBUTING.md REUSE.toml .pre-commit-config.yaml; do
    gh api "repos/$REPO/contents/$meta" >/dev/null 2>&1 && say "  meta: $meta present"
done
say "  NOTE: fork PRs may sit at CI 'action_required' until a maintainer"
say "  approves workflow runs — local gates are the real pre-review gate."

# --- 2. local formatter gate -------------------------------------------------
hr; say "LOCAL CLANG-FORMAT GATE"
CFMT="${CFMT_BIN:-$HOME/tools/cfmt-20/bin/clang-format}"
if [ ! -x "$CFMT" ]; then
    if command -v clang-format >/dev/null 2>&1; then CFMT=$(command -v clang-format);
    else say "  NO clang-format found (install: python3 -m venv ~/tools/cfmt-20 && ~/tools/cfmt-20/bin/pip install clang-format==<CI major>)"; exit 1; fi
fi
LOCAL_V=$("$CFMT" --version | grep -oE '[0-9]+\.' | head -1 | tr -d '.')
say "  formatter: $("$CFMT" --version)"
if [ -n "${CI_CFMV:-}" ]; then
    say "  CI clang-format major: $CI_CFMV (local: ${LOCAL_V:-unknown})"
    if [ "$LOCAL_V" != "$CI_CFMV" ]; then
        say "  WARNING: version mismatch — style output can differ between majors."
        [ "${CFMT_STRICT:-0}" = "1" ] && { say "  CFMT_STRICT=1 → failing."; exit 1; }
    fi
fi

if [ -z "$FILES" ]; then
    BASE_SHA=$(git rev-parse --verify -q "$BASE" >/dev/null 2>&1 && echo ok || echo no)
    if [ "$BASE_SHA" = "ok" ]; then
        FILES=$(git diff --name-only "$BASE"...HEAD -- '*.c' '*.h' '*.cc' '*.cpp' '*.hpp' 2>/dev/null)
    fi
fi
if [ -z "$FILES" ]; then
    say "  (no C/C++ files selected — pass '-- <files>' to check explicitly)"; exit 0
fi

bad=0
for f in $FILES; do
    if [ ! -f "$f" ]; then say "  MISSING: $f"; bad=1; continue; fi
    if ! "$CFMT" --dry-run --Werror "$f" >/dev/null 2>&1; then
        say "  VIOLATION: $f"
        "$CFMT" --dry-run --Werror "$f" 2>&1 | head -5 | sed 's/^/    /'
        bad=1
    else
        say "  ok: $f"
    fi
done
hr
if [ "$bad" = 0 ]; then say "RESULT: format gate GREEN"; else say "RESULT: format gate RED — fix before pushing (run: $CFMT -i <files>)"; fail=1; fi
exit $fail
