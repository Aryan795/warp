#!/usr/bin/env bash
# Regression test for the cobra description-padding fix in `_warp_native_bash_completions`
# (see the COMP_TYPE comment in bash_body.sh). Exercises two synthetic completion functions
# rather than real `gh`/`git` binaries, so it runs deterministically without external tools:
#
#   - `__cobra_style_complete` reproduces cobra's documented COMP_TYPE branch (see
#     https://github.com/spf13/cobra/issues/1508): under COMP_TYPE 9 (plain Tab) with more than
#     one match it bakes a padded "name  (description)" string into COMPREPLY; under 37 or 42 it
#     strips descriptions and emits bare names regardless of match count.
#   - `__ordinary_style_complete` reproduces a bash-completion-style function that ignores
#     COMP_TYPE entirely and always emits bare names -- the shape the fix must not break.
#
# Usage: bash bash_native_completions_test.sh

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
failures=0

define_bashpreexec_functions() { :; }
install_bashpreexec() { :; }
WARP_IS_SUBSHELL=1
WARP_SESSION_ID=1
WARP_IS_LOCAL_SHELL_SESSION=0
WARP_USING_WINDOWS_CON_PTY=false
WARP_IN_MSYS2=false
source "$REPO_ROOT/app/assets/bundled/bootstrap/bash_body.sh" >/dev/null 2>&1

__cobra_style_complete() {
  if [[ "$COMP_TYPE" == 9 && ${#COMPREPLY[@]} -ge 0 ]]; then
    # Cobra's actual condition is "more than one match"; reproduce that by hard-coding two
    # matches for this fixture, as the real cobra scripts do for a real multi-match prefix.
    COMPREPLY=("checkout  (Check out a pull request)" "checks    (Show CI status)")
  else
    COMPREPLY=("checkout" "checks")
  fi
}
complete -F __cobra_style_complete cobra-cli

__ordinary_style_complete() {
  # Deliberately ignores COMP_TYPE, matching bash-completion's own scripts.
  COMPREPLY=("checkout" "cherry-pick" "cherry")
}
complete -F __ordinary_style_complete ordinary-cli

assert_reply() {
  local desc="$1"
  shift
  local -a expected=("$@")
  if [[ "${replies[*]}" != "${expected[*]}" ]]; then
    echo "FAIL: $desc"
    echo "  expected: ${expected[*]}"
    echo "  actual:   ${replies[*]}"
    failures=$((failures + 1))
  else
    echo "PASS: $desc"
  fi
}

collect_replies() {
  # _warp_native_bash_completions emits one OSC per match with no separator between them
  # ("\e]9280;C;<match>\a\e]9280;C;<match>\a..."), so extract every match with `grep -oP`
  # rather than treating the output as newline-delimited.
  mapfile -t replies < <(
    _warp_native_bash_completions "$1" 2>/dev/null | command -p grep -oP '(?<=9280;C;)[^\x07]*'
  )
}

collect_replies "cobra-cli che"
assert_reply "cobra-style entry stays a bare name (no baked-in description)" "checkout" "checks"

collect_replies "ordinary-cli ch"
assert_reply "ordinary bash-completion-style entry is unaffected" "checkout" "cherry-pick" "cherry"

if [[ $failures -eq 0 ]]; then
  echo "All bash native completions tests passed."
  exit 0
else
  echo "$failures test(s) failed."
  exit 1
fi
