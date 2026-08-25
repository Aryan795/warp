# Spike artifact for CORE-3807 (real-keypress handoff). fish has no Warp
# bootstrap body today, so this would need one; sourced after the user's
# config.fish so `bind \cr` reports what the user configured.
#
# Both fzf's and atuin's fish widgets write their result back with the
# `commandline` builtin, which fails with status 1 outside an interactive-editing
# context. A `bind` function is such a context, so invoking the user's binding
# from this wrapper is what makes fish work at all.

# `bind \cr` prints the full re-binding command, e.g. `bind \cr fzf-history-widget`.
# Strip the `bind <seq> ` prefix to recover just the command.
set -g __warp_orig_ctrl_r (bind \cr | string replace -r '^bind\s+\S+\s+' '')

function __warp_report_ctrl_r_selection
  printf '\eP$f{"hook": "ExternalCtrlRSelection", "value": {"command": %s}}\x9c' \
    (__warp_json_string $argv[1]) > /dev/tty
end

function __warp_external_ctrl_r
  eval $__warp_orig_ctrl_r

  __warp_report_ctrl_r_selection (commandline)

  commandline ''
  commandline -f repaint
end

# Unlike bash's `bind` (\C-x notation) and zsh's `bindkey` (^X notation), fish's
# `bind` wants the literal key bytes, and fish does not expand backslash escapes
# stored in a variable. WARP_EXTERNAL_CTRL_R_KEYSEQ must therefore already hold
# the raw bytes here. Getting this wrong fails silently in the worst way: the
# unmatched leading byte is swallowed and the trailing ^R fires the user's own
# binding directly, so the widget still appears but the wrapper never runs and
# the selection is left in the shell's buffer.
bind $WARP_EXTERNAL_CTRL_R_KEYSEQ __warp_external_ctrl_r
