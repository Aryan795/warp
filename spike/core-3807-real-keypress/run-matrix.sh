#!/usr/bin/env bash
# CORE-3807 spike: verify that Warp's wrapper-widget handoff works for every
# shell/tool combination, by driving a real interactive shell on a PTY.
#
# Each case sends the wrapper key sequence, filters, then either selects an entry
# or cancels, and asserts two things:
#   1. the selection Warp would have received over the DCS hook, and
#   2. that the shell's own line buffer was left empty (Warp owns the command).
#
# Requires: fzf and atuin on PATH, python3.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DRIVE="$HERE/harness/vtdrive.py"
WORK=${WORK:-/tmp/core3807}
SEQ_BASH='\C-x\C-r'
SEQ_ZSH='^X^R'
SEQ_FISH=$'\x18\x12'   # fish binds literal bytes, not escape notation
# The bytes Warp would write for C-x C-r, as a vtdrive escape string.
SEND='\x18\x12'

export PATH=/tmp/bin:/tmp/atuin-x86_64-unknown-linux-gnu:$PATH
FZF_SHELL=${FZF_SHELL:-/tmp/fzf-shell}
BASH_PREEXEC=${BASH_PREEXEC:-/tmp/bash-preexec.sh}
TARGET='echo zzz-target-marker-XYZ'
SEEDS=("echo alpha-marker-one" "echo beta-marker-two" "git status --short" "$TARGET")

pass=0; fail=0
result() { # name expected actual
  if [[ $2 == "$3" ]]; then printf '  PASS  %-34s %s\n' "$1" "$3"; pass=$((pass+1))
  else printf '  FAIL  %-34s expected=%q actual=%q\n' "$1" "$2" "$3"; fail=$((fail+1)); fi
}

# Test-mode reporter: the real snippets emit a DCS hook, which is invisible by
# design. Redefining it to write a file is the only way to assert on the value.
report_override_posix() {
  cat <<EOF
__warp_report_ctrl_r_selection() { printf '%s' "\$1" > $1; }
EOF
}

seed_atuin() { # home
  local home=$1
  mkdir -p "$home/.config/atuin"
  cat > "$home/.config/atuin/config.toml" <<'EOF'
auto_sync = false
update_check = false
enter_accept = false
style = "compact"
inline_height = 12
EOF
  local sess; sess=$(HOME=$home atuin uuid)
  for c in "${SEEDS[@]}"; do
    local id
    id=$(HOME=$home ATUIN_SESSION=$sess atuin history start -- "$c") || return 1
    HOME=$home ATUIN_SESSION=$sess atuin history end --exit 0 "$id" >/dev/null 2>&1
  done
}

run_case() { # name home shell query keys_after outfile expected_selection
  local name=$1 home=$2 shell=$3 query=$4 finish=$5 out=$6 expected=$7
  rm -f "$out"
  local screen
  screen=$(HOME=$home python3 "$DRIVE" --cmd "$shell -i" \
    --step 'wait:1.5' \
    --step "send:$SEND" \
    --step 'wait:2.5' \
    --step "send:$query" --step 'wait:1.5' \
    --step "send:$finish" --step 'wait:2.0' \
    --step 'line:final' 2>&1)
  local sel; sel=$(cat "$out" 2>/dev/null)
  result "$name selection" "$expected" "$sel"
  local finalline; finalline=$(sed -n 's/^\[final\] CURSOR LINE: //p' <<<"$screen")
  # The prompt must be back with nothing after it.
  if [[ $finalline == *"PROMPT>'" ]]; then
    printf '  PASS  %-34s buffer left empty (%s)\n' "$name buffer" "$finalline"; pass=$((pass+1))
  else
    printf '  FAIL  %-34s buffer not empty: %s\n' "$name buffer" "$finalline"; fail=$((fail+1))
  fi
}

##### bash + fzf #####
setup_bash() { # home tool
  local home=$1 tool=$2
  rm -rf "$home"; mkdir -p "$home"
  {
    echo 'export PATH=/tmp/bin:/tmp/atuin-x86_64-unknown-linux-gnu:$PATH'
    echo "PS1='BASHPROMPT> '"
    echo 'HISTFILE=$HOME/.bash_history'
    if [[ $tool == fzf ]]; then echo "source $FZF_SHELL/key-bindings.bash"
    else echo "source $BASH_PREEXEC"; echo 'eval "$(atuin init bash --disable-up-arrow)"'; fi
    echo "WARP_EXTERNAL_CTRL_R_KEYSEQ='$SEQ_BASH'"
    echo '__warp_json_string() { printf "%s" "$1"; }'
    cat "$HERE/bootstrap-snippets/bash_wrapper.bash"
    report_override_posix "$WORK/sel-bash-$tool.txt"
  } > "$home/.bashrc"
  printf '%s\n' "${SEEDS[@]}" > "$home/.bash_history"
  [[ $tool == atuin ]] && seed_atuin "$home"
  return 0
}

setup_zsh() { # home tool
  local home=$1 tool=$2
  rm -rf "$home"; mkdir -p "$home"
  {
    echo 'export PATH=/tmp/bin:/tmp/atuin-x86_64-unknown-linux-gnu:$PATH'
    echo "PROMPT='ZSHPROMPT> '"
    echo 'HISTFILE=$HOME/.zsh_history'; echo 'HISTSIZE=1000'; echo 'SAVEHIST=1000'
    if [[ $tool == fzf ]]; then echo "source $FZF_SHELL/key-bindings.zsh"
    else echo 'eval "$(atuin init zsh --disable-up-arrow)"'; fi
    echo "WARP_EXTERNAL_CTRL_R_KEYSEQ='$SEQ_ZSH'"
    echo '__warp_json_string() { printf "%s" "$1"; }'
    cat "$HERE/bootstrap-snippets/zsh_wrapper.zsh"
    report_override_posix "$WORK/sel-zsh-$tool.txt"
  } > "$home/.zshrc"
  : > "$home/.zsh_history"
  local t=1700000001
  for c in "${SEEDS[@]}"; do printf ': %s:0;%s\n' "$t" "$c" >> "$home/.zsh_history"; t=$((t+1)); done
  [[ $tool == atuin ]] && seed_atuin "$home"
  return 0
}

setup_fish() { # home tool
  local home=$1 tool=$2
  rm -rf "$home"; mkdir -p "$home/.config/fish" "$home/.local/share/fish"
  {
    echo 'set -gx PATH /tmp/bin /tmp/atuin-x86_64-unknown-linux-gnu $PATH'
    echo "function fish_prompt; echo -n 'FISHPROMPT> '; end"
    echo 'function fish_greeting; end'
    if [[ $tool == fzf ]]; then echo "source $FZF_SHELL/key-bindings.fish"; echo 'fzf_key_bindings'
    else echo 'atuin init fish --disable-up-arrow | source'; fi
    echo "set -g WARP_EXTERNAL_CTRL_R_KEYSEQ '$SEQ_FISH'"
    echo 'function __warp_json_string; printf "%s" $argv[1]; end'
    cat "$HERE/bootstrap-snippets/fish_wrapper.fish"
    echo "function __warp_report_ctrl_r_selection; printf '%s' \$argv[1] > $WORK/sel-fish-$tool.txt; end"
  } > "$home/.config/fish/config.fish"
  : > "$home/.local/share/fish/fish_history"
  local t=1700000001
  for c in "${SEEDS[@]}"; do
    printf -- '- cmd: %s\n  when: %s\n' "$c" "$t" >> "$home/.local/share/fish/fish_history"; t=$((t+1))
  done
  [[ $tool == atuin ]] && seed_atuin "$home"
  return 0
}

mkdir -p "$WORK"
for tool in fzf atuin; do
  for sh in bash zsh fish; do
    home="$WORK/h-$sh-$tool"
    "setup_$sh" "$home" "$tool"
    binpath=/usr/bin/$sh
    out="$WORK/sel-$sh-$tool.txt"
    echo "== $sh + $tool =="
    run_case "$sh/$tool select" "$home" "$binpath" 'zzz' '\r' "$out" "$TARGET"
    run_case "$sh/$tool cancel" "$home" "$binpath" 'zzz' '\x1b' "$out" ''
  done
done

echo
echo "passed=$pass failed=$fail"
[[ $fail -eq 0 ]]
