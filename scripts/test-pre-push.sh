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
git config user.name synthalorian
git config user.email "$OK"
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

# A stale default-branch tracking ref (remote was rewound) must not hide it either.
git update-ref refs/remotes/origin/main "$stale"
git checkout -q -b new-on-stale-main "$stale"
expect fail "new branch containing a commit from a rewound origin/main fails" origin new-on-stale-main
git update-ref refs/remotes/origin/main "$base"

# A remote HEAD pointing at the pushed branch cannot exclude its history.
git checkout -q -b head-trick "$base"
bad "bad commit behind a remote HEAD trick"
git update-ref refs/remotes/origin/head-trick HEAD
git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/head-trick
expect fail "remote HEAD pointing at the pushed branch cannot hide commits" origin head-trick
git update-ref -d refs/remotes/origin/head-trick
git remote set-head origin main >/dev/null

# Dependabot-style commits on a new branch are exempt.
git checkout -q -b dep "$base"
dependabot "bump dep"
expect pass "new branch with a dependabot commit passes" origin dep

# Tags are new refs: checked against the default branch too.
git checkout -q -b tagged "$base"
bad "tagged bad"
git tag -a bad-tag -m "bad tag"
expect fail "new tag on a bad commit fails" origin bad-tag
git checkout -q "$base"
git tag -a good-tag -m "good tag"
expect pass "new tag on a good commit passes" origin good-tag

# Remote given as a URL (no tracking refs at all).
git checkout -q -b by-url "$base"
bad "by url bad"
expect fail "push to a URL with a bad commit fails" "$tmp/remote.git" by-url
git checkout -q -b by-url-good "$base"
good "by url good"
expect pass "push to a URL with a good commit passes" "$tmp/remote.git" by-url-good

# Force updates check the range from the real remote tip.
git checkout -q -b forced "$base"
good "forced 1"
git push -q origin forced >/dev/null 2>&1
git reset -q --hard "$base"
bad "forced bad"
expect fail "force push with a bad commit fails" --force origin forced
git reset -q --hard "$base"
good "forced good"
expect pass "force push with a good commit passes" --force origin forced

# A bad ref anywhere in a multi-ref push rejects the whole push.
git checkout -q -b multi-good "$base"
good "multi good"
git checkout -q -b multi-bad "$base"
bad "multi bad"
expect fail "multi-ref push with one bad ref fails" origin multi-good multi-bad
if on_remote refs/heads/multi-good; then echo "FAIL - good ref reached the remote in a rejected push"; fails=$((fails + 1)); fi

# Remote with no default branch: fail closed on the whole history.
git init -q --bare -b main "$tmp/empty.git"
git checkout -q -b empty-clean "$base"
expect pass "empty remote, clean history passes" "$tmp/empty.git" empty-clean
git init -q --bare -b main "$tmp/empty2.git"
git checkout -q -b empty-bad "$stale"
expect fail "empty remote, bad commit in history fails" "$tmp/empty2.git" empty-bad

# A separate push URL is the destination that counts, not the fetch URL.
git init -q --bare -b main "$tmp/pushdest.git"
git push -q "$tmp/pushdest.git" "$base:refs/heads/main" >/dev/null 2>&1
git init -q --bare -b main "$tmp/fetchside.git"
git checkout -q -b split-bad "$base"
bad "split url bad"
git push -q --no-verify "$tmp/fetchside.git" "HEAD:refs/heads/main" >/dev/null 2>&1
git remote add split "$tmp/fetchside.git"
git remote set-url --push split "$tmp/pushdest.git"
expect fail "separate push URL is checked, not the fetch URL" split split-bad
git remote remove split

# Unreachable remote: new refs are rejected.
rc=0
printf 'refs/heads/x %s refs/heads/x %s\n' "$base" "$zero" | bash .githooks/pre-push "$tmp/does-not-exist.git" >/dev/null 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then echo "ok   - unreachable remote rejects a new ref"; else echo "FAIL - unreachable remote (exit $rc)"; fails=$((fails + 1)); fi

# Default branch tip missing locally is rejected with a clear message.
git --git-dir="$tmp/remote.git" update-ref refs/heads/main "$(git --git-dir="$tmp/remote.git" rev-parse main)"
git clone -q "$tmp/remote.git" "$tmp/clone2"
(
  cd "$tmp/clone2"
  git config commit.gpgsign false
  git config user.name synthalorian
  git config user.email "$OK"
  git commit -q --allow-empty -m "elsewhere"
  git push -q origin main >/dev/null 2>&1
)
git checkout -q -b behind "$base"
good "behind good"
rc=0
out="$(printf 'refs/heads/behind %s refs/heads/behind %s\n' "$(git rev-parse HEAD)" "$zero" | bash .githooks/pre-push origin 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && grep -q "not present locally" <<<"$out"; then
  echo "ok   - default tip missing locally is rejected"
else
  echo "FAIL - default tip missing locally (exit $rc)"; fails=$((fails + 1))
fi
git fetch -q origin

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
