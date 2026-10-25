#!/usr/bin/env bash
# Tests for .githooks/pre-push using real pushes to a local bare remote.
# Run: bash scripts/test-pre-push.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$(cd "$here/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

OK="212206047+synthalorian@users.noreply.github.com"
BOT_EMAIL="49699333+dependabot[bot]@users.noreply.github.com"
zero=0000000000000000000000000000000000000000
fails=0

git init -q --bare -b main "$tmp/remote.git"
git init -q -b main "$tmp/work"
cd "$tmp/work"
git config commit.gpgsign false
git config core.hooksPath .githooks
mkdir -p .githooks scripts
cp "$src/.githooks/pre-push" .githooks/pre-push
cp "$src/scripts/check-noreply.sh" scripts/check-noreply.sh
git remote add origin "$tmp/remote.git"

commit() {
  GIT_AUTHOR_NAME="$1" GIT_AUTHOR_EMAIL="$2" GIT_COMMITTER_NAME="$3" GIT_COMMITTER_EMAIL="$4" \
    git commit -q --allow-empty -m "$5"
}
good()       { commit synthalorian "$OK" synthalorian "$OK" "$1"; }
bad()        { commit synthalorian "gmail@example.com" synthalorian "gmail@example.com" "$1"; }
dependabot() { commit "dependabot[bot]" "$BOT_EMAIL" GitHub noreply@github.com "$1"; }

# expect <pass|fail> <label> <git push args...>
expect() {
  local want="$1" label="$2" rc=0
  shift 2
  git push -q "$@" >/dev/null 2>&1 || rc=$?
  if { [ "$want" = pass ] && [ "$rc" -eq 0 ]; } || { [ "$want" = fail ] && [ "$rc" -ne 0 ]; }; then
    echo "ok   - $label"
  else
    echo "FAIL - $label (wanted $want, exit $rc)"; fails=$((fails + 1))
  fi
}
# reached <ref>: true if the bare remote has the ref.
on_remote() { git --git-dir="$tmp/remote.git" rev-parse -q --verify "$1" >/dev/null 2>&1; }

good "base"
expect pass "first push of a good history to an empty remote" origin main
git remote set-head origin main >/dev/null
base="$(git rev-parse HEAD)"

# Existing branch updates use the remote ref update from the hook's stdin.
good "good update"
expect pass "good update to an existing branch passes" origin main
bad "bad update"
expect fail "bad update to an existing branch fails" origin main
git reset -q --hard HEAD~1

# New branches.
git checkout -q -b feature "$base"
good "feature good"
expect pass "new branch with good commits passes" origin feature
git checkout -q -b feature-bad "$base"
bad "feature bad"
expect fail "new branch with a bad commit fails" origin feature-bad
if on_remote refs/heads/feature-bad; then echo "FAIL - bad branch reached the remote"; fails=$((fails + 1)); fi

# A stale tracking ref that contains a bad commit must not hide it.
git checkout -q -b stale-src "$base"
bad "bad commit behind a stale tracking ref"
stale="$(git rev-parse HEAD)"
git update-ref refs/remotes/origin/stale-leftover "$stale"
git checkout -q -b new-on-stale "$stale"
good "good on top of stale"
expect fail "new branch containing a commit from a stale tracking ref fails" origin new-on-stale
git update-ref -d refs/remotes/origin/stale-leftover

# Dependabot-style commits on a new branch are exempt.
git checkout -q -b dep "$base"
dependabot "bump dep"
expect pass "new branch with a dependabot commit passes" origin dep

# Missing default-branch ref: fail closed on the whole history.
git checkout -q -b nodefault "$base"
good "nodefault good"
git symbolic-ref -d refs/remotes/origin/HEAD
git update-ref -d refs/remotes/origin/main
git checkout -q -b nodefault-clean-hist "$base"
expect pass "no default ref, clean history passes" origin nodefault-clean-hist
git checkout -q -b nodefault-bad "$stale"
expect fail "no default ref, bad commit in history fails" origin nodefault-bad
git fetch -q origin
git remote set-head origin main >/dev/null

# Branch deletion is not checked.
expect pass "branch deletion passes" origin --delete feature

# Remote tip missing locally fails with a clear message (hook run directly).
rc=0
out="$(printf 'refs/heads/main %s refs/heads/main %s\n' "$base" "1111111111111111111111111111111111111111" \
  | bash .githooks/pre-push origin 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "not present locally" <<<"$out"; then
  echo "ok   - unknown remote tip is rejected"
else
  echo "FAIL - unknown remote tip (exit $rc)"; fails=$((fails + 1))
fi

# Deletion line on stdin passes without touching history (hook run directly).
rc=0
printf '(delete) %s refs/heads/x %s\n' "$zero" "$base" | bash .githooks/pre-push origin >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 0 ]; then echo "ok   - delete line passes"; else echo "FAIL - delete line (exit $rc)"; fails=$((fails + 1)); fi

if [ "$fails" -ne 0 ]; then echo "$fails test(s) failed" >&2; exit 1; fi
echo "all tests passed"
